use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex, RwLock};
use tracing::warn;
use uuid::Uuid;

use crate::audio::{
    AudioPlayer, MediaResolver, PlayMode, PlaybackStatus, PlayerEvent, PlaylistManager,
    RequestOrigin, TrackInfo,
};
use crate::config::Config;
use crate::tools::ToolsManager;
use crate::voice::protocol::SvcGroup;
use crate::voice::VoiceClient;

#[derive(Debug, Clone)]
pub enum GroupAction {
    JoinOrCreate {
        name: String,
        password: Option<String>,
        group_type: i16,
    },
    JoinById {
        id: Uuid,
        password: Option<String>,
    },
    Leave,
}

pub struct AppManager {
    pub config: Arc<Config>,
    pub tools: Arc<ToolsManager>,
    pub voice_client: Arc<RwLock<Option<Arc<VoiceClient>>>>,
    pub player: Arc<RwLock<Option<Arc<AudioPlayer>>>>,
    pub playlist: Arc<Mutex<PlaylistManager>>,
    pub event_tx: mpsc::Sender<PlayerEvent>,
    pub mc_connected: Arc<AtomicBool>,
    pub voice_connected: Arc<AtomicBool>,
    pub secret_received: Arc<AtomicBool>,
    pub chat_sender: Arc<RwLock<Option<mpsc::Sender<String>>>>,
    pub last_tpa_accepted: Arc<Mutex<Option<tokio::time::Instant>>>,
    pub known_groups: Arc<RwLock<HashMap<Uuid, SvcGroup>>>,
    pub current_group: Arc<RwLock<Option<SvcGroup>>>,
    pub group_action_sender: Arc<RwLock<Option<mpsc::Sender<GroupAction>>>>,
}

impl AppManager {
    pub fn new(config: Arc<Config>, tools: Arc<ToolsManager>) -> Arc<Self> {
        let (event_tx, mut event_rx) = mpsc::channel::<PlayerEvent>(32);
        let playlist = Arc::new(Mutex::new(PlaylistManager::new(PathBuf::from("playlist.json"))));

        let app = Arc::new(Self {
            config,
            tools,
            voice_client: Arc::new(RwLock::new(None)),
            player: Arc::new(RwLock::new(None)),
            playlist,
            event_tx,
            mc_connected: Arc::new(AtomicBool::new(false)),
            voice_connected: Arc::new(AtomicBool::new(false)),
            secret_received: Arc::new(AtomicBool::new(false)),
            chat_sender: Arc::new(RwLock::new(None)),
            last_tpa_accepted: Arc::new(Mutex::new(None)),
            known_groups: Arc::new(RwLock::new(HashMap::new())),
            current_group: Arc::new(RwLock::new(None)),
            group_action_sender: Arc::new(RwLock::new(None)),
        });

        // 启动后台播放器事件自动连播监听协程
        let app_weak = Arc::downgrade(&app);
        tokio::spawn(async move {
            let mut consecutive_failures = 0;
            while let Some(event) = event_rx.recv().await {
                let Some(app) = app_weak.upgrade() else { break };
                match event {
                    PlayerEvent::TrackFinished => {
                        consecutive_failures = 0;
                        let (next_track, mode) = {
                            let mut pl = app.playlist.lock().await;
                            (pl.advance_next(false), pl.mode)
                        };

                        if let Some(track) = next_track {
                            if let Err(e) = app.play_track_internal(track.clone()).await {
                                warn!("[App] 自动连播下一首失败: {:?}", e);
                            } else if mode != PlayMode::SingleLoop {
                                app.send_game_chat(&format!("[Bot] 正在播放: {}", track.title)).await;
                            }
                        }
                    }
                    PlayerEvent::TrackFailed { track, error } => {
                        consecutive_failures += 1;
                        let len = app.playlist.lock().await.tracks.len();
                        warn!(
                            "[App] 曲目 \"{}\" 播放失败 (连续失败: {}/{}): {}",
                            track.title, consecutive_failures, len, error
                        );

                        if len > 0 && consecutive_failures < len.min(5) {
                            let (next_track, mode) = {
                                let mut pl = app.playlist.lock().await;
                                (pl.advance_next(true), pl.mode)
                            };
                            if let Some(next) = next_track {
                                if let Err(e) = app.play_track_internal(next.clone()).await {
                                    warn!("[App] 自动切歌重试失败: {:?}", e);
                                } else if mode != PlayMode::SingleLoop {
                                    app.send_game_chat(&format!("[Bot] 自动跳过损坏曲目，正在播放: {}", next.title)).await;
                                }
                            }
                        } else if consecutive_failures >= len.min(5) {
                            app.send_game_chat("[Bot] 连续多首曲目播放失败，已停止自动连播").await;
                        }
                    }
                }
            }
        });

        app
    }

    /// 内部推流触发辅助函数
    async fn play_track_internal(&self, track: TrackInfo) -> anyhow::Result<()> {
        let player = match self.player.read().await.clone() {
            Some(p) => p,
            None => anyhow::bail!("语音推流引擎尚未就绪"),
        };
        player.play(track).await
    }

    /// 绑定初始化的语音与播放器服务
    pub async fn bind_voice_player(&self, voice_client: Arc<VoiceClient>, player: Arc<AudioPlayer>) {
        *self.voice_client.write().await = Some(voice_client);
        *self.player.write().await = Some(player);
        self.voice_connected.store(true, Ordering::SeqCst);
    }

    /// 绑定向游戏内发送公屏聊天的通道
    pub async fn set_chat_sender(&self, tx: mpsc::Sender<String>) {
        *self.chat_sender.write().await = Some(tx);
    }

    /// 向 Minecraft 游戏内公屏发送消息
    pub async fn send_game_chat(&self, msg: &str) {
        if let Some(tx) = self.chat_sender.read().await.as_ref() {
            let _ = tx.send(msg.to_string()).await;
        }
    }

    /// 绑定向 Minecraft 发送 SVC 群组指令的通道
    pub async fn set_group_action_sender(&self, tx: mpsc::Sender<GroupAction>) {
        *self.group_action_sender.write().await = Some(tx);
    }

    /// 发起加入或创建 SVC 群组操作
    pub async fn send_group_action(&self, action: GroupAction) {
        if let Some(tx) = self.group_action_sender.read().await.as_ref() {
            let _ = tx.send(action).await;
        }
    }

    /// 根据群组名模糊或精确查找已知群组
    pub async fn find_group_by_name(&self, name: &str) -> Option<SvcGroup> {
        let groups = self.known_groups.read().await;
        groups.values().find(|g| g.name.eq_ignore_ascii_case(name)).cloned()
    }

    /// 更新当前激活的群组
    pub async fn set_current_group(&self, group: Option<SvcGroup>) {
        *self.current_group.write().await = group;
    }

    /// 获取当前激活的群组
    pub async fn get_current_group(&self) -> Option<SvcGroup> {
        self.current_group.read().await.clone()
    }

    /// 添加或更新已发现的群组
    pub async fn update_known_group(&self, group: SvcGroup) {
        self.known_groups.write().await.insert(group.id, group);
    }

    /// 移除已解散或离开的群组
    pub async fn remove_known_group(&self, id: Uuid) {
        self.known_groups.write().await.remove(&id);
        let mut cur = self.current_group.write().await;
        if let Some(ref g) = *cur {
            if g.id == id {
                *cur = None;
            }
        }
    }

    /// 默认点播曲目 (play_now = false)
    pub async fn request_play(&self, url: &str, origin: RequestOrigin) -> anyhow::Result<String> {
        self.request_play_with_options(url, origin, false).await
    }

    /// 点播曲目（带立即播放/插播选项）：
    /// - play_now = false：若当前 Idle，加入列表并立即播放；若正在播放/暂停，仅追加至列表末尾。
    /// - play_now = true：插入到 current_index + 1 并立即切到该曲播放。
    pub async fn request_play_with_options(
        &self,
        url: &str,
        origin: RequestOrigin,
        play_now: bool,
    ) -> anyhow::Result<String> {
        let ytdlp = self.tools.ytdlp.as_deref();
        let ffmpeg = self.tools.ffmpeg.as_deref();
        let resolved = MediaResolver::resolve(url, ytdlp, ffmpeg).await?;

        let track = TrackInfo {
            id: Uuid::new_v4(),
            title: resolved.title.clone(),
            raw_query: resolved.raw_query,
            source: resolved.source_url_or_path,
            is_local_file: resolved.is_local_file,
            http_headers: resolved.http_headers,
            source_bitrate_kbps: resolved.source_bitrate_kbps,
            duration_secs: resolved.duration_secs,
            replay_gain_db: resolved.replay_gain_db,
            origin,
        };

        let player = match self.player.read().await.clone() {
            Some(p) => p,
            None => anyhow::bail!("语音推流引擎尚未就绪，请等待假人连接并完成握手！"),
        };

        let status = *player.status.read().await;

        if play_now {
            let mut pl = self.playlist.lock().await;
            let insert_idx = pl.insert_next(track.clone());
            pl.jump_to(insert_idx);
            drop(pl);
            player.play(track).await?;
            return Ok(format!("立即播放: {}", resolved.title));
        }

        if status == PlaybackStatus::Idle {
            let mut pl = self.playlist.lock().await;
            pl.add_track(track.clone());
            let cur_idx = pl.tracks.len() - 1;
            pl.jump_to(cur_idx);
            drop(pl);
            player.play(track).await?;
            Ok(format!("正在播放: {}", resolved.title))
        } else {
            let mut pl = self.playlist.lock().await;
            pl.add_track(track.clone());
            Ok(format!("已加入播放列表: {}", resolved.title))
        }
    }

    pub async fn pause(&self) -> anyhow::Result<()> {
        if let Some(player) = self.player.read().await.as_ref() {
            player.pause().await?;
        }
        Ok(())
    }

    /// 若处于 Paused 则恢复；若处于 Idle 且歌单非空，则启动播放当前曲目
    pub async fn resume(&self) -> anyhow::Result<()> {
        if let Some(player) = self.player.read().await.as_ref() {
            let status = *player.status.read().await;
            if status == PlaybackStatus::Paused {
                player.resume().await?;
                return Ok(());
            } else if status == PlaybackStatus::Idle {
                let track_opt = self.playlist.lock().await.get_current_track();
                if let Some(track) = track_opt {
                    player.play(track).await?;
                    return Ok(());
                }
            }
        }
        Ok(())
    }

    /// 仅停止当前播放，保留歌单内容！
    pub async fn stop(&self) -> anyhow::Result<()> {
        if let Some(player) = self.player.read().await.as_ref() {
            player.stop().await?;
        }
        Ok(())
    }

    /// 手动切到下一首
    pub async fn skip(&self) -> anyhow::Result<Option<String>> {
        let next_track = self.playlist.lock().await.advance_next(true);
        if let Some(track) = next_track {
            let title = track.title.clone();
            if let Some(player) = self.player.read().await.as_ref() {
                player.play(track).await?;
            }
            Ok(Some(title))
        } else {
            if let Some(player) = self.player.read().await.as_ref() {
                player.stop().await?;
            }
            Ok(None)
        }
    }

    /// 手动切到上一首
    pub async fn prev(&self) -> anyhow::Result<Option<String>> {
        let prev_track = self.playlist.lock().await.advance_prev();
        if let Some(track) = prev_track {
            let title = track.title.clone();
            if let Some(player) = self.player.read().await.as_ref() {
                player.play(track).await?;
            }
            Ok(Some(title))
        } else {
            Ok(None)
        }
    }

    /// 播放模式控制
    pub async fn get_play_mode(&self) -> PlayMode {
        self.playlist.lock().await.mode
    }

    pub async fn set_play_mode(&self, mode: PlayMode) {
        self.playlist.lock().await.set_mode(mode);
    }

    /// 跳转到指定序号 (1-based)
    pub async fn jump_to_index(&self, index_1based: usize) -> anyhow::Result<String> {
        if index_1based == 0 {
            anyhow::bail!("曲目序号必须从 1 开始");
        }
        let idx = index_1based - 1;
        let track = self
            .playlist
            .lock()
            .await
            .jump_to(idx)
            .ok_or_else(|| anyhow::anyhow!("序号越界，未找到第 {} 首曲目", index_1based))?;

        if let Some(player) = self.player.read().await.as_ref() {
            player.play(track.clone()).await?;
        }
        Ok(track.title)
    }

    /// 置顶为下一首 (1-based)
    pub async fn move_to_next(&self, index_1based: usize) -> anyhow::Result<String> {
        if index_1based == 0 {
            anyhow::bail!("曲目序号必须从 1 开始");
        }
        let idx = index_1based - 1;
        let mut pl = self.playlist.lock().await;
        let title = pl
            .tracks
            .get(idx)
            .map(|t| t.title.clone())
            .ok_or_else(|| anyhow::anyhow!("序号越界，未找到第 {} 首曲目", index_1based))?;
        pl.move_to_next(idx)?;
        Ok(title)
    }

    /// 移除指定曲目 (1-based)
    pub async fn remove_at_index(&self, index_1based: usize) -> anyhow::Result<String> {
        if index_1based == 0 {
            anyhow::bail!("曲目序号必须从 1 开始");
        }
        let idx = index_1based - 1;
        let mut pl = self.playlist.lock().await;
        let (removed, was_cur) = pl
            .remove_track(idx)
            .ok_or_else(|| anyhow::anyhow!("序号越界，未找到第 {} 首曲目", index_1based))?;

        if was_cur {
            if let Some(next) = pl.get_current_track() {
                if let Some(player) = self.player.read().await.as_ref() {
                    let _ = player.play(next).await;
                }
            } else if let Some(player) = self.player.read().await.as_ref() {
                let _ = player.stop().await;
            }
        }
        Ok(removed.title)
    }

    /// 清空歌单
    pub async fn clear_playlist(&self) -> anyhow::Result<()> {
        self.playlist.lock().await.clear();
        if let Some(player) = self.player.read().await.as_ref() {
            player.stop().await?;
        }
        Ok(())
    }

    /// ReplayGain 控制
    pub async fn is_replay_gain_enabled(&self) -> bool {
        if let Some(player) = self.player.read().await.as_ref() {
            player.is_replay_gain_enabled()
        } else {
            self.config.voice.replay_gain
        }
    }

    pub async fn set_replay_gain_enabled(&self, enabled: bool) {
        if let Some(player) = self.player.read().await.as_ref() {
            player.set_replay_gain_enabled(enabled);
        }
    }

    pub async fn get_current_gain_db(&self) -> Option<f32> {
        if let Some(player) = self.player.read().await.as_ref() {
            player.get_current_gain_db()
        } else {
            None
        }
    }

    pub async fn set_volume(&self, vol: u32) {
        if let Some(player) = self.player.read().await.as_ref() {
            player.set_volume(vol);
        }
    }

    pub async fn get_volume(&self) -> u32 {
        if let Some(player) = self.player.read().await.as_ref() {
            player.get_volume()
        } else {
            80
        }
    }

    pub async fn get_bitrate_kbps(&self) -> f64 {
        if let Some(player) = self.player.read().await.as_ref() {
            player.current_bitrate_kbps.load(Ordering::Relaxed) as f64 / 10.0
        } else {
            0.0
        }
    }
}
