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
use crate::audio::{PlaybackStatus, RequestOrigin};

#[derive(RustEmbed)]
#[folder = "web/"]
struct Assets;

#[derive(Serialize)]
struct StatusResponse {
    mc_connected: bool,
    voice_connected: bool,
    bot_name: String,
    status: String,
    current_track: Option<String>,
    current_source: Option<String>,
    source_bitrate_kbps: Option<f64>,
    current_bitrate_kbps: Option<f64>,
    volume: u32,
    queue: Vec<String>,
    last_error: Option<String>,
}

#[derive(Deserialize)]
struct PlayRequest {
    url: String,
}

#[derive(Deserialize)]
struct ControlRequest {
    action: String,
    value: Option<u32>,
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

    let mut status_str = "Idle".to_string();
    let mut current_track = None;
    let mut current_source = None;
    let mut source_bitrate_kbps = None;
    let mut current_bitrate_kbps = None;
    let mut last_error = None;
    let volume = app.get_volume().await;

    if let Some(player) = app.player.read().await.as_ref() {
        let st = *player.status.read().await;
        status_str = match st {
            PlaybackStatus::Idle => "Idle".to_string(),
            PlaybackStatus::Playing => "Playing".to_string(),
            PlaybackStatus::Paused => "Paused".to_string(),
        };

        if let Some(track) = player.current_track.read().await.as_ref() {
            current_track = Some(track.title.clone());
            current_source = Some(track.source.clone());
            source_bitrate_kbps = track.source_bitrate_kbps;
        }

        let cur_bps = player.current_bitrate_kbps.load(Ordering::Relaxed);
        if cur_bps > 0 {
            current_bitrate_kbps = Some(cur_bps as f64 / 10.0);
        }

        last_error = player.last_error.read().await.clone();
    }

    let queue_guard = app.playlist.lock().await;
    let queue: Vec<String> = queue_guard.iter().map(|t| t.title.clone()).collect();

    Json(StatusResponse {
        mc_connected,
        voice_connected,
        bot_name,
        status: status_str,
        current_track,
        current_source,
        source_bitrate_kbps,
        current_bitrate_kbps,
        volume,
        queue,
        last_error,
    })
}

async fn play_handler(
    State(app): State<Arc<AppManager>>,
    Json(payload): Json<PlayRequest>,
) -> Result<String, (StatusCode, String)> {
    match app.request_play(&payload.url, RequestOrigin::WebConsole).await {
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
) -> StatusCode {
    match payload.action.as_str() {
        "pause" => {
            let _ = app.pause().await;
        }
        "resume" => {
            let _ = app.resume().await;
        }
        "stop" => {
            let _ = app.stop().await;
        }
        "skip" => {
            let _ = app.skip().await;
        }
        "volume" => {
            if let Some(val) = payload.value {
                app.set_volume(val).await;
            }
        }
        _ => return StatusCode::BAD_REQUEST,
    }
    StatusCode::OK
}
