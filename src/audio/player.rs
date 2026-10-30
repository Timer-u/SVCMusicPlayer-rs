use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;
use rusty_opus::{Application, OpusEncoder};
use tokio::sync::{mpsc, RwLock};
use tokio::time::interval;
use tracing::{error, info, warn};

use crate::audio::decoder::AudioDecoder;
use crate::voice::client::VoiceClient;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestOrigin {
    GameChat { sender: Option<String> },
    WebConsole,
    Internal,
}

#[derive(Debug, Clone)]
pub struct TrackInfo {
    pub title: String,
    pub source: String,
    pub is_local_file: bool,
    pub http_headers: Option<std::collections::HashMap<String, String>>,
    pub source_bitrate_kbps: Option<f64>,
    pub origin: RequestOrigin,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlaybackStatus {
    Idle,
    Playing,
    Paused,
}

pub struct AudioPlayer {
    pub current_track: Arc<RwLock<Option<TrackInfo>>>,
    pub status: Arc<RwLock<PlaybackStatus>>,
    pub volume: Arc<AtomicU32>, // 0 ~ 100
    pub current_bitrate_kbps: Arc<AtomicU32>, // 实时推流码率 (x10 存储，例如 960 代表 96.0 kbps)
    pub target_bitrate_kbps: u32,
    pub last_error: Arc<RwLock<Option<String>>>,
    cmd_tx: mpsc::Sender<PlayerCommand>,
}

enum PlayerCommand {
    Play(TrackInfo),
    Pause,
    Resume,
    Stop,
}

impl AudioPlayer {
    pub fn new(
        voice_client: Arc<VoiceClient>,
        ffmpeg_path: Option<PathBuf>,
        bitrate_kbps: u32,
        chat_sender: Arc<RwLock<Option<mpsc::Sender<String>>>>,
    ) -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel(16);
        let current_track = Arc::new(RwLock::new(None));
        let status = Arc::new(RwLock::new(PlaybackStatus::Idle));
        let volume = Arc::new(AtomicU32::new(80)); // 默认 80%
        let current_bitrate_kbps = Arc::new(AtomicU32::new(0));
        let last_error = Arc::new(RwLock::new(None));
        let target_bitrate = if bitrate_kbps > 0 { bitrate_kbps } else { 96 };

        let player = Self {
            current_track: Arc::clone(&current_track),
            status: Arc::clone(&status),
            volume: Arc::clone(&volume),
            current_bitrate_kbps: Arc::clone(&current_bitrate_kbps),
            target_bitrate_kbps: target_bitrate,
            last_error: Arc::clone(&last_error),
            cmd_tx,
        };

        // 启动后台音频推流工作线程
        let current_track_clone = Arc::clone(&current_track);
        let status_clone = Arc::clone(&status);
        let volume_clone = Arc::clone(&volume);
        let current_bitrate_kbps_clone = Arc::clone(&current_bitrate_kbps);
        let last_error_clone = Arc::clone(&last_error);

        tokio::spawn(async move {
            Self::worker_loop(
                voice_client,
                ffmpeg_path,
                target_bitrate,
                current_track_clone,
                status_clone,
                volume_clone,
                current_bitrate_kbps_clone,
                last_error_clone,
                chat_sender,
                cmd_rx,
            )
            .await;
        });

        player
    }

    pub async fn play(&self, track: TrackInfo) -> anyhow::Result<()> {
        self.cmd_tx.send(PlayerCommand::Play(track)).await?;
        Ok(())
    }

    pub async fn pause(&self) -> anyhow::Result<()> {
        self.cmd_tx.send(PlayerCommand::Pause).await?;
        Ok(())
    }

    pub async fn resume(&self) -> anyhow::Result<()> {
        self.cmd_tx.send(PlayerCommand::Resume).await?;
        Ok(())
    }

    pub async fn stop(&self) -> anyhow::Result<()> {
        self.cmd_tx.send(PlayerCommand::Stop).await?;
        Ok(())
    }

    pub fn set_volume(&self, vol: u32) {
        let clamped = vol.min(100);
        self.volume.store(clamped, Ordering::SeqCst);
        info!("[Player] 音量调整为: {}%", clamped);
    }

    pub fn get_volume(&self) -> u32 {
        self.volume.load(Ordering::SeqCst)
    }

    async fn worker_loop(
        voice_client: Arc<VoiceClient>,
        ffmpeg_path: Option<PathBuf>,
        target_bitrate_kbps: u32,
        current_track: Arc<RwLock<Option<TrackInfo>>>,
        status: Arc<RwLock<PlaybackStatus>>,
        volume: Arc<AtomicU32>,
        current_bitrate_kbps: Arc<AtomicU32>,
        last_error: Arc<RwLock<Option<String>>>,
        chat_sender: Arc<RwLock<Option<mpsc::Sender<String>>>>,
        mut cmd_rx: mpsc::Receiver<PlayerCommand>,
    ) {
        let mut active_decoder: Option<AudioDecoder> = None;
        let mut encoder = match OpusEncoder::new(48000, 1, Application::Audio) {
            Ok(mut enc) => {
                enc.bitrate_bps = (target_bitrate_kbps * 1000) as i32;
                info!(
                    "[Player] Opus 音频编码器就绪: 模式=Audio(音乐全频带), 目标码率={} kbps, 采样率=48kHz, 声道=1(单声道)",
                    target_bitrate_kbps
                );
                enc
            }
            Err(e) => {
                error!("[Player] 初始化 Opus 编码器失败: {}", e);
                return;
            }
        };

        let frame_size = 960; // 20ms @ 48000Hz
        let mut ticker = interval(Duration::from_millis(20));
        let mut f32_buf = vec![0.0f32; frame_size];
        let mut opus_buf = vec![0u8; 1024];

        // 实时码率监控统计状态
        let mut total_frames_played: u64 = 0;
        let mut window_frames: u64 = 0;
        let mut window_bytes: usize = 0;
        let mut initial_logged = false;

        loop {
            // 优先检查是否有新指令进入
            tokio::select! {
                cmd = cmd_rx.recv() => {
                    match cmd {
                        Some(PlayerCommand::Play(track)) => {
                            let src_br_str = match track.source_bitrate_kbps {
                                Some(br) => format!("~{:.0} kbps", br),
                                None => if track.is_local_file { "本地文件(按源采样率解码)".to_string() } else { "流媒体自适应".to_string() },
                            };
                            info!(
                                "[Player] 开始播放曲目: \"{}\" | 源音质: {} | 推流编码器: Opus Audio (48kHz, 目标推流: {} kbps)",
                                track.title,
                                src_br_str,
                                target_bitrate_kbps
                            );

                            *current_track.write().await = Some(track.clone());
                            *status.write().await = PlaybackStatus::Playing;
                            *last_error.write().await = None;

                            total_frames_played = 0;
                            window_frames = 0;
                            window_bytes = 0;
                            initial_logged = false;
                            current_bitrate_kbps.store(0, Ordering::Relaxed);

                            match AudioDecoder::open(
                                &track.source,
                                track.is_local_file,
                                track.http_headers.as_ref(),
                                ffmpeg_path.as_deref(),
                            ).await {
                                Ok(dec) => {
                                    active_decoder = Some(dec);
                                }
                                Err(e) => {
                                    let err_msg = e.to_string();
                                    error!("[Player] 打开音频流失败: {}", err_msg);
                                    *last_error.write().await = Some(err_msg.clone());

                                    // 哪边发送的播放请求，就在哪边报错
                                    match &track.origin {
                                        RequestOrigin::GameChat { sender } => {
                                            if let Some(tx) = chat_sender.read().await.as_ref() {
                                                let mention = sender.as_ref().map(|s| format!("@{} ", s)).unwrap_or_default();
                                                let _ = tx.send(format!("[Bot] {}播放失败: 打开音频流失败 ({})", mention, err_msg)).await;
                                            }
                                        }
                                        RequestOrigin::WebConsole => {
                                            warn!("[Player] 来自 Web 控制台的点歌在后台启动失败: {}", err_msg);
                                        }
                                        RequestOrigin::Internal => {}
                                    }

                                    active_decoder = None;
                                    *current_track.write().await = None;
                                    *status.write().await = PlaybackStatus::Idle;
                                }
                            }
                        }
                        Some(PlayerCommand::Pause) => {
                            info!("[Player] 播放暂停");
                            *status.write().await = PlaybackStatus::Paused;
                            current_bitrate_kbps.store(0, Ordering::Relaxed);
                        }
                        Some(PlayerCommand::Resume) => {
                            info!("[Player] 播放恢复");
                            *status.write().await = PlaybackStatus::Playing;
                        }
                        Some(PlayerCommand::Stop) => {
                            info!("[Player] 停止播放");
                            active_decoder = None;
                            *current_track.write().await = None;
                            *status.write().await = PlaybackStatus::Idle;
                            *last_error.write().await = None;
                            total_frames_played = 0;
                            window_frames = 0;
                            window_bytes = 0;
                            initial_logged = false;
                            current_bitrate_kbps.store(0, Ordering::Relaxed);
                        }
                        None => break,
                    }
                }
                _ = ticker.tick() => {
                    let current_status = *status.read().await;
                    if current_status != PlaybackStatus::Playing || active_decoder.is_none() {
                        continue;
                    }

                    let decoder = active_decoder.as_mut().unwrap();
                    match decoder.read_samples(frame_size).await {
                        Ok(Some(samples)) => {
                            let vol = volume.load(Ordering::SeqCst) as f32 / 100.0;
                            for i in 0..frame_size {
                                f32_buf[i] = ((samples[i] as f32) / 32768.0) * vol;
                            }

                            // 实时进行 Opus 编码并推流
                            match encoder.encode(&f32_buf, frame_size, &mut opus_buf) {
                                Ok(encoded_len) => {
                                    let opus_frame = &opus_buf[..encoded_len];
                                    if let Err(e) = voice_client.send_mic_frame(opus_frame, false).await {
                                        warn!("[Player] 发送音频数据包失败: {:?}", e);
                                    }

                                    window_bytes += encoded_len;
                                    window_frames += 1;
                                    total_frames_played += 1;

                                    // 首秒呈现：第 50 帧 (1秒推流完成) 输出初始推流码率
                                    if !initial_logged && window_frames >= 50 {
                                        let kbps = (window_bytes as f64 * 8.0) / (window_frames as f64 * 0.02 * 1000.0);
                                        let avg_bytes = window_bytes / window_frames as usize;
                                        info!(
                                            "[Player] 初始推流码率: {:.1} kbps (实时 Opus 编码, 50 pkt/s, 平均帧长: {} 字节)",
                                            kbps,
                                            avg_bytes
                                        );
                                        current_bitrate_kbps.store((kbps * 10.0) as u32, Ordering::Relaxed);
                                        initial_logged = true;
                                        window_bytes = 0;
                                        window_frames = 0;
                                    }

                                    // 周期性呈现：每 5 秒 (250 帧) 打印一次当前实时动态码率与播放进度
                                    if initial_logged && window_frames >= 250 {
                                        let kbps = (window_bytes as f64 * 8.0) / (window_frames as f64 * 0.02 * 1000.0);
                                        let avg_bytes = window_bytes / window_frames as usize;
                                        let total_secs = total_frames_played / 50;
                                        let mins = total_secs / 60;
                                        let secs = total_secs % 60;
                                        info!(
                                            "[Player] 实时推流码率: {:.1} kbps [进度 {:02}:{:02}] (平均包长: {} 字节, 50 pkt/s, Opus VBR)",
                                            kbps,
                                            mins,
                                            secs,
                                            avg_bytes
                                        );
                                        current_bitrate_kbps.store((kbps * 10.0) as u32, Ordering::Relaxed);
                                        window_bytes = 0;
                                        window_frames = 0;
                                    }
                                }
                                Err(e) => {
                                    warn!("[Player] Opus 编码异常: {:?}", e);
                                }
                            }
                        }
                        Ok(None) => {
                            // 音频流读取完毕 (EOF)
                            let total_secs = total_frames_played / 50;
                            info!(
                                "[Player] 当前曲目播放完毕 (播放时长: {:02}:{:02}, 累计推流: {} 帧)",
                                total_secs / 60,
                                total_secs % 60,
                                total_frames_played
                            );
                            active_decoder = None;
                            *current_track.write().await = None;
                            *status.write().await = PlaybackStatus::Idle;
                            total_frames_played = 0;
                            window_frames = 0;
                            window_bytes = 0;
                            initial_logged = false;
                            current_bitrate_kbps.store(0, Ordering::Relaxed);
                        }
                        Err(e) => {
                            error!("[Player] 音频流读取异常中断: {:?}", e);
                            active_decoder = None;
                            *current_track.write().await = None;
                            *status.write().await = PlaybackStatus::Idle;
                            total_frames_played = 0;
                            window_frames = 0;
                            window_bytes = 0;
                            initial_logged = false;
                            current_bitrate_kbps.store(0, Ordering::Relaxed);
                        }
                    }
                }
            }
        }
    }
}
