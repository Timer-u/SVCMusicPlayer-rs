use axum::{
    extract::State,
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use rust_embed::RustEmbed;
use serde::{Deserialize, Serialize};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tracing::{info, warn};

use crate::app::AppManager;
use crate::audio::{PlaybackStatus, PlayMode, RequestOrigin};

#[derive(RustEmbed)]
#[folder = "web/"]
struct Assets;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaylistItemDto {
    pub index: usize, // 1-based 序号
    pub id: String,
    pub title: String,
    pub duration_secs: Option<u64>,
    pub origin: String, // 例如 "Web" 或 "MC: Alice"
    pub is_current: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusResponse {
    pub mc_connected: bool,
    pub voice_connected: bool,
    pub bot_name: String,
    pub current_group: Option<String>,
    pub status: String, // "Idle" | "Playing" | "Paused"
    pub play_mode: String, // "ListLoop" | "SingleLoop" | "Shuffle" | "Sequential"
    pub play_mode_cn: String,
    pub current_track: Option<String>,
    pub current_source: Option<String>,
    pub source_bitrate_kbps: Option<f64>,
    pub current_bitrate_kbps: Option<f64>,
    pub elapsed_secs: u32,
    pub duration_secs: Option<u64>,
    pub volume: u32,
    pub replay_gain_enabled: bool,
    pub current_gain_db: Option<f32>,
    pub current_index: Option<usize>, // 1-based
    pub playlist: Vec<PlaylistItemDto>,
    pub queue: Vec<String>, // 向后兼容字段
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PlayRequest {
    pub url: String,
    #[serde(default)]
    pub play_now: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ControlRequest {
    pub action: String,
    pub value: Option<u32>,
    pub mode: Option<String>,
    pub enabled: Option<bool>,
    pub index: Option<usize>, // 1-based 序号
}

pub async fn start_web_server(app: Arc<AppManager>, bind_address: &str) -> anyhow::Result<()> {
    let router = Router::new()
        .route("/", get(index_handler))
        .route("/api/status", get(status_handler))
        .route("/api/play", post(play_handler))
        .route("/api/control", post(control_handler))
        .with_state(app);

    let listener = tokio::net::TcpListener::bind(bind_address).await?;
    info!("[Web] 内嵌控制台已启动，访问地址: http://{}", bind_address);

    axum::serve(listener, router).await?;
    Ok(())
}

async fn index_handler() -> Response {
    match Assets::get("index.html") {
        Some(content) => (
            [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
            content.data,
        )
            .into_response(),
        None => (StatusCode::NOT_FOUND, "index.html not found").into_response(),
    }
}

async fn status_handler(State(app): State<Arc<AppManager>>) -> Json<StatusResponse> {
    let mc_connected = app.mc_connected.load(Ordering::SeqCst);
    let voice_connected = app.voice_connected.load(Ordering::SeqCst);
    let bot_name = app.config.minecraft.bot_name.clone();
    let current_group = app.get_current_group().await.map(|g| g.name);

    let mode = app.get_play_mode().await;
    let play_mode = mode.as_str().to_string();
    let play_mode_cn = mode.display_cn().to_string();

    let mut status_str = "Idle".to_string();
    let mut current_track = None;
    let mut current_source = None;
    let mut source_bitrate_kbps = None;
    let mut current_bitrate_kbps = None;
    let mut elapsed_secs = 0;
    let mut duration_secs = None;
    let mut last_error = None;
    let volume = app.get_volume().await;
    let replay_gain_enabled = app.is_replay_gain_enabled().await;
    let current_gain_db = app.get_current_gain_db().await;

    if let Some(player) = app.player.read().await.as_ref() {
        let st = *player.status.read().await;
        status_str = match st {
            PlaybackStatus::Idle => "Idle".to_string(),
            PlaybackStatus::Playing => "Playing".to_string(),
            PlaybackStatus::Paused => "Paused".to_string(),
        };

        elapsed_secs = player.elapsed_secs.load(Ordering::Relaxed);
        let d = player.duration_secs.load(Ordering::Relaxed);
        if d > 0 {
            duration_secs = Some(d as u64);
        }

        if let Some(track) = player.current_track.read().await.as_ref() {
            current_track = Some(track.title.clone());
            current_source = Some(track.source.clone());
            source_bitrate_kbps = track.source_bitrate_kbps;
            if duration_secs.is_none() {
                duration_secs = track.duration_secs;
            }
        }

        let cur_bps = player.current_bitrate_kbps.load(Ordering::Relaxed);
        if cur_bps > 0 {
            current_bitrate_kbps = Some(cur_bps as f64 / 10.0);
        }

        last_error = player.last_error.read().await.clone();
    }

    let pl = app.playlist.lock().await;
    let current_index = pl.current_index.map(|idx| idx + 1);

    let mut playlist = Vec::with_capacity(pl.tracks.len());
    let mut queue = Vec::with_capacity(pl.tracks.len());
    for (i, t) in pl.tracks.iter().enumerate() {
        let is_current = pl.current_index == Some(i);
        let origin = match &t.origin {
            RequestOrigin::GameChat { sender: Some(s) } => format!("MC: {}", s),
            RequestOrigin::GameChat { sender: None } => "MC".to_string(),
            RequestOrigin::WebConsole => "Web".to_string(),
            RequestOrigin::Internal => "Internal".to_string(),
        };
        queue.push(t.title.clone());
        playlist.push(PlaylistItemDto {
            index: i + 1,
            id: t.id.to_string(),
            title: t.title.clone(),
            duration_secs: t.duration_secs,
            origin,
            is_current,
        });
    }

    Json(StatusResponse {
        mc_connected,
        voice_connected,
        bot_name,
        current_group,
        status: status_str,
        play_mode,
        play_mode_cn,
        current_track,
        current_source,
        source_bitrate_kbps,
        current_bitrate_kbps,
        elapsed_secs,
        duration_secs,
        volume,
        replay_gain_enabled,
        current_gain_db,
        current_index,
        playlist,
        queue,
        last_error,
    })
}

async fn play_handler(
    State(app): State<Arc<AppManager>>,
    Json(payload): Json<PlayRequest>,
) -> Result<String, (StatusCode, String)> {
    match app
        .request_play_with_options(&payload.url, RequestOrigin::WebConsole, payload.play_now)
        .await
    {
        Ok(msg) => {
            info!("[Web] 点播请求成功: \"{}\" -> {}", payload.url, msg);
            Ok(msg)
        }
        Err(e) => {
            let err_msg = e.to_string();
            warn!("[Web] 点播请求失败: \"{}\" -> {}", payload.url, err_msg);
            Err((StatusCode::BAD_REQUEST, err_msg))
        }
    }
}

async fn control_handler(
    State(app): State<Arc<AppManager>>,
    Json(payload): Json<ControlRequest>,
) -> Result<String, (StatusCode, String)> {
    match payload.action.as_str() {
        "pause" => {
            app.pause()
                .await
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
            Ok("已暂停".to_string())
        }
        "resume" => {
            app.resume()
                .await
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
            Ok("已恢复播放".to_string())
        }
        "stop" => {
            app.stop()
                .await
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
            Ok("已停止播放".to_string())
        }
        "skip" => {
            let res = app
                .skip()
                .await
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
            Ok(match res {
                Some(title) => format!("已切至下一首: {}", title),
                None => "已切至下一首".to_string(),
            })
        }
        "prev" => {
            let res = app
                .prev()
                .await
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
            Ok(match res {
                Some(title) => format!("已切至上一首: {}", title),
                None => "已切至上一首".to_string(),
            })
        }
        "volume" => {
            if let Some(val) = payload.value {
                app.set_volume(val).await;
                Ok(format!("音量已设为 {}%", val))
            } else {
                Err((StatusCode::BAD_REQUEST, "缺少 volume 参数".to_string()))
            }
        }
        "mode" => {
            if let Some(ref m_str) = payload.mode {
                if let Some(mode) = PlayMode::from_input(m_str) {
                    let cn = mode.display_cn();
                    app.set_play_mode(mode).await;
                    Ok(format!("播放模式已切换为: {}", cn))
                } else {
                    Err((StatusCode::BAD_REQUEST, format!("未知播放模式: {}", m_str)))
                }
            } else {
                Err((StatusCode::BAD_REQUEST, "缺少 mode 参数".to_string()))
            }
        }
        "replay_gain" => {
            if let Some(e) = payload.enabled {
                app.set_replay_gain_enabled(e).await;
                Ok(if e {
                    "ReplayGain 已启用".to_string()
                } else {
                    "ReplayGain 已关闭".to_string()
                })
            } else {
                Err((StatusCode::BAD_REQUEST, "缺少 enabled 参数".to_string()))
            }
        }
        "playlist_jump" => {
            if let Some(idx) = payload.index {
                let title = app
                    .jump_to_index(idx)
                    .await
                    .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
                Ok(format!("正在播放: {}", title))
            } else {
                Err((StatusCode::BAD_REQUEST, "缺少 index 参数".to_string()))
            }
        }
        "playlist_top" => {
            if let Some(idx) = payload.index {
                let title = app
                    .move_to_next(idx)
                    .await
                    .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
                Ok(format!("已置顶为下一首: {}", title))
            } else {
                Err((StatusCode::BAD_REQUEST, "缺少 index 参数".to_string()))
            }
        }
        "playlist_remove" => {
            if let Some(idx) = payload.index {
                let title = app
                    .remove_at_index(idx)
                    .await
                    .map_err(|e| (StatusCode::BAD_REQUEST, e.to_string()))?;
                Ok(format!("已删除曲目: {}", title))
            } else {
                Err((StatusCode::BAD_REQUEST, "缺少 index 参数".to_string()))
            }
        }
        "playlist_clear" => {
            app.clear_playlist()
                .await
                .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
            Ok("已清空歌单".to_string())
        }
        _ => Err((StatusCode::BAD_REQUEST, format!("未知操作: {}", payload.action))),
    }
}
