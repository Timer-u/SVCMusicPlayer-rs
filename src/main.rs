use std::sync::Arc;
use svc_music_player::app::AppManager;
use svc_music_player::config::Config;
use svc_music_player::mc::SessionManager;
use svc_music_player::tools::ToolsManager;
use svc_music_player::web;
use tracing::{error, info};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 初始化日志记录器
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "svc_music_player=info,azalea=warn".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    info!("========================================================");
    info!("        SVCMusicPlayer-rs 语音伴播机器人启动中...        ");
    info!("========================================================");

    // 1. 加载或生成配置文件
    let config = Config::load_or_create("config.toml")?;
    info!("[Config] 目标服务器: {}:{}", config.minecraft.server_host, config.minecraft.server_port);
    info!("[Config] 假人名称: {}", config.minecraft.bot_name);

    // 确保本地音乐存放目录存在
    let _ = std::fs::create_dir_all("music");

    // 2. 检测并自愈外部工具链 (yt-dlp, ffmpeg)
    let tools = ToolsManager::init(
        &config.media.bin_dir,
        &config.media.ffmpeg_path,
        &config.media.ytdlp_path,
        config.media.auto_download_tools,
    )
    .await;

    // 3. 构建核心上下文
    let app = AppManager::new(Arc::new(config.clone()), Arc::new(tools));

    // 4. 启动内嵌 Web 控制台 (若启用)
    if config.web.enabled {
        let app_web = Arc::clone(&app);
        let bind_addr = config.web.bind_address.clone();
        tokio::spawn(async move {
            if let Err(e) = web::start_web_server(app_web, &bind_addr).await {
                error!("[Web] 控制台服务异常退出: {:?}", e);
            }
        });
    }

    // 5. 启动 Minecraft 客户端会话 (Azalea 内部包含 LocalSet，使用独立专用线程运行当前线程运行时)
    let app_bot = Arc::clone(&app);
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("Failed to build local Tokio runtime for Azalea");

        rt.block_on(async move {
            if let Err(e) = SessionManager::run_bot(app_bot).await {
                error!("[MC] 假人会话异常终止: {:?}", e);
            }
        });
    });

    info!("[Main] 服务已全部挂起运行，按下 Ctrl+C 即可安全退出程序。");
    tokio::signal::ctrl_c().await?;

    info!("[Main] 收到退出信号，正在优雅关闭语音服务...");
    if let Some(vc) = app.voice_client.read().await.as_ref() {
        vc.close();
    }
    if let Some(player) = app.player.read().await.as_ref() {
        let _ = player.stop().await;
    }

    info!("[Main] 程序已完全退出。");
    Ok(())
}
