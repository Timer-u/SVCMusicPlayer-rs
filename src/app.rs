use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex, RwLock};
use uuid::Uuid;

use crate::audio::{AudioPlayer, MediaResolver, PlaybackStatus, RequestOrigin, TrackInfo};
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
    pub playlist: Arc<Mutex<VecDeque<TrackInfo>>>,
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
        Arc::new(Self {
            config,
            tools,
            voice_client: Arc::new(RwLock::new(None)),
            player: Arc::new(RwLock::new(None)),
            playlist: Arc::new(Mutex::new(VecDeque::new())),
            mc_connected: Arc::new(AtomicBool::new(false)),
            voice_connected: Arc::new(AtomicBool::new(false)),
            secret_received: Arc::new(AtomicBool::new(false)),
            chat_sender: Arc::new(RwLock::new(None)),
            last_tpa_accepted: Arc::new(Mutex::new(None)),
            known_groups: Arc::new(RwLock::new(HashMap::new())),
            current_group: Arc::new(RwLock::new(None)),
            group_action_sender: Arc::new(RwLock::new(None)),
        })
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

    /// 点播曲目：解析链接、加入队列并在空闲时直接播放
    pub async fn request_play(&self, url: &str, origin: RequestOrigin) -> anyhow::Result<String> {
        let ytdlp = self.tools.ytdlp.as_deref();
        let ffmpeg = self.tools.ffmpeg.as_deref();
        let resolved = MediaResolver::resolve(url, ytdlp, ffmpeg).await?;

        let track = TrackInfo {
            title: resolved.title.clone(),
            source: resolved.source_url_or_path,
            is_local_file: resolved.is_local_file,
            http_headers: resolved.http_headers,
            source_bitrate_kbps: resolved.source_bitrate_kbps,
            origin,
        };

        let player_opt = self.player.read().await.clone();
        if let Some(player) = player_opt {
            let status = *player.status.read().await;
            if status == PlaybackStatus::Idle {
                // 当前空闲，直接开播
                player.play(track).await?;
                return Ok(format!("正在播放: {}", resolved.title));
            } else {
                // 正在播放，加入排队列表
                self.playlist.lock().await.push_back(track);
                return Ok(format!("已加入播放队列: {}", resolved.title));
            }
        }

        anyhow::bail!("语音推流引擎尚未就绪，请等待假人连接并完成握手！");
    }

    pub async fn pause(&self) -> anyhow::Result<()> {
        if let Some(player) = self.player.read().await.as_ref() {
            player.pause().await?;
        }
        Ok(())
    }

    pub async fn resume(&self) -> anyhow::Result<()> {
        if let Some(player) = self.player.read().await.as_ref() {
            player.resume().await?;
        }
        Ok(())
    }

    pub async fn stop(&self) -> anyhow::Result<()> {
        if let Some(player) = self.player.read().await.as_ref() {
            player.stop().await?;
        }
        self.playlist.lock().await.clear();
        Ok(())
    }

    pub async fn skip(&self) -> anyhow::Result<Option<String>> {
        let mut queue = self.playlist.lock().await;
        if let Some(next_track) = queue.pop_front() {
            let title = next_track.title.clone();
            if let Some(player) = self.player.read().await.as_ref() {
                player.play(next_track).await?;
            }
            Ok(Some(title))
        } else {
            // 队列无歌曲，停止播放
            if let Some(player) = self.player.read().await.as_ref() {
                player.stop().await?;
            }
            Ok(None)
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
