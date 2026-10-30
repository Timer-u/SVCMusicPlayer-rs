use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use tracing::{info, warn};

pub struct ToolsManager {
    pub ffmpeg: Option<PathBuf>,
    pub ytdlp: Option<PathBuf>,
}

impl ToolsManager {
    pub async fn init(
        bin_dir_str: &str,
        custom_ffmpeg: &str,
        custom_ytdlp: &str,
        auto_download: bool,
    ) -> Self {
        let bin_dir = PathBuf::from(bin_dir_str);
        if !bin_dir.exists() {
            let _ = fs::create_dir_all(&bin_dir);
        }

        let ffmpeg = Self::find_tool(custom_ffmpeg, &bin_dir, "ffmpeg");
        if let Some(ref path) = ffmpeg {
            info!("[Tools] FFmpeg 已就绪: {:?}", path);
        } else {
            info!("[Tools] 未检测到 FFmpeg，将自动启用纯 Rust (Symphonia) 音频解码器");
        }

        let mut ytdlp = Self::find_tool(custom_ytdlp, &bin_dir, "yt-dlp");

        if ytdlp.is_none() && auto_download {
            info!("[Tools] 未检测到 yt-dlp，正在尝试自动从 GitHub Release 下载预编译版本...");
            match Self::download_ytdlp(&bin_dir).await {
                Ok(path) => {
                    info!("[Tools] yt-dlp 自动下载成功: {:?}", path);
                    ytdlp = Some(path);
                }
                Err(e) => {
                    warn!("[Tools] yt-dlp 自动下载失败: {:?}。若需解析 B站/YouTube 链接，请手动放置 yt-dlp 到 bin 目录", e);
                }
            }
        }

        if let Some(ref path) = ytdlp {
            info!("[Tools] yt-dlp 已就绪: {:?}", path);
        } else {
            info!("[Tools] 未检测到 yt-dlp，仅支持直接播放本地音频文件与网络音频直链");
        }

        Self { ffmpeg, ytdlp }
    }

    fn find_tool(custom_path: &str, bin_dir: &Path, name: &str) -> Option<PathBuf> {
        let exe_name = if cfg!(windows) {
            format!("{}.exe", name)
        } else {
            name.to_string()
        };

        // 1. 检查配置中的显式路径
        if !custom_path.is_empty() {
            let p = PathBuf::from(custom_path);
            if p.exists() {
                return Some(p);
            }
        }

        // 2. 检查 bin 目录
        let in_bin = bin_dir.join(&exe_name);
        if in_bin.exists() {
            return Some(in_bin);
        }

        // 3. 检查系统全局 PATH (Linux 上使用 which，Windows 上使用 where)
        let which_cmd = if cfg!(windows) { "where" } else { "which" };
        if let Ok(output) = Command::new(which_cmd).arg(name).output() {
            if output.status.success() {
                if let Some(first_line) = String::from_utf8_lossy(&output.stdout).lines().next() {
                    let path = PathBuf::from(first_line.trim());
                    if path.exists() {
                        return Some(path);
                    }
                }
            }
        }

        // 4. 回退：直接测试能否在 PATH 中调用 (ffmpeg 需用 -version, yt-dlp 使用 --version)
        for test_arg in &["-version", "--version"] {
            if let Ok(output) = Command::new(name).arg(test_arg).output() {
                if output.status.success() {
                    return Some(PathBuf::from(name));
                }
            }
        }

        None
    }

    async fn download_ytdlp(bin_dir: &Path) -> anyhow::Result<PathBuf> {
        let (url, filename) = if cfg!(target_os = "windows") {
            (
                "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe",
                "yt-dlp.exe",
            )
        } else if cfg!(target_os = "macos") {
            (
                "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp_macos",
                "yt-dlp",
            )
        } else {
            (
                "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp",
                "yt-dlp",
            )
        };

        let target_path = bin_dir.join(filename);
        let resp = reqwest::get(url).await?;
        let bytes = resp.bytes().await?;

        let mut file = File::create(&target_path)?;
        file.write_all(&bytes)?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&target_path)?.permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&target_path, perms)?;
        }

        Ok(target_path)
    }
}
