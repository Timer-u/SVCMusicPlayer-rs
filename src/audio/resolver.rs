use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tokio::process::Command;
use tracing::{info, warn};

#[derive(Debug, Clone)]
pub struct ResolvedMedia {
    pub title: String,
    pub raw_query: String,
    pub source_url_or_path: String,
    pub is_local_file: bool,
    pub http_headers: Option<HashMap<String, String>>,
    pub source_bitrate_kbps: Option<f64>,
    pub duration_secs: Option<u64>,
    pub replay_gain_db: Option<f32>,
}

/// 安全解码外部子进程的标准输出（优先 UTF-8，自动支持 Windows 中文环境下的 GBK / GB18030 回退）
pub fn decode_output_string(bytes: &[u8]) -> String {
    if let Ok(s) = std::str::from_utf8(bytes) {
        return s.trim().to_string();
    }
    let (cow, _, had_errors) = encoding_rs::GBK.decode(bytes);
    if !had_errors {
        return cow.trim().to_string();
    }
    String::from_utf8_lossy(bytes).trim().to_string()
}

pub struct MediaResolver;

impl MediaResolver {
    pub async fn resolve(
        raw_input: &str,
        ytdlp_path: Option<&Path>,
        ffmpeg_path: Option<&Path>,
    ) -> anyhow::Result<ResolvedMedia> {
        let input = raw_input.trim();

        // 1. 判断是否为本地文件（支持绝对路径、相对路径、以及 music/ 目录下自动查找与扩展名补全）
        if let Some((found_path, title)) = Self::find_local_file(input) {
            info!("[Resolver] 找到本地音频文件: {} -> {:?}", input, found_path);
            return Ok(ResolvedMedia {
                title,
                raw_query: input.to_string(),
                source_url_or_path: found_path.to_string_lossy().to_string(),
                is_local_file: true,
                http_headers: None,
                source_bitrate_kbps: None,
                duration_secs: None,
                replay_gain_db: None,
            });
        }

        // 2. 判断是否为网易云音乐 (ID / 链接 / 快捷前缀，参照 TartaricAcid/NetMusic 处理)
        if let Some(song_id) = Self::parse_netease_id(input) {
            info!("[Resolver] 识别到网易云音乐歌曲 ID: {}", song_id);
            return Self::resolve_netease_song(song_id, input).await;
        }

        // 3. 判断是否为 Bilibili BV 号 / AV 号，若是则规范化为完整 Bilibili 链接
        let normalized_input = if let Some(bili_url) = Self::normalize_bilibili(input) {
            info!("[Resolver] 识别到 Bilibili 视频识别码: {} -> {}", input, bili_url);
            bili_url
        } else {
            input.to_string()
        };

        // 3.1 判断是否为 Niconico 识别码或链接（支持 sm/so/nm、nico.ms 及 www.video.nicovideo.jp 等域），规范化为标准链接
        let normalized_input = if let Some(nico_url) = Self::normalize_niconico(&normalized_input) {
            info!("[Resolver] 识别到 Niconico 识别码/链接: {} -> {}", input, nico_url);
            nico_url
        } else {
            normalized_input
        };

        // 4. 判断是否为常规直链（以常见音频扩展名结尾）
        let lower = normalized_input.to_lowercase();
        let is_direct_audio_ext = lower.ends_with(".mp3")
            || lower.ends_with(".wav")
            || lower.ends_with(".flac")
            || lower.ends_with(".ogg")
            || lower.ends_with(".aac")
            || lower.ends_with(".m4a");

        if (normalized_input.starts_with("http://") || normalized_input.starts_with("https://"))
            && is_direct_audio_ext
        {
            let filename = normalized_input
                .split('/')
                .last()
                .unwrap_or("网络音频")
                .split('?')
                .next()
                .unwrap_or("网络音频")
                .to_string();
            return Ok(ResolvedMedia {
                title: filename,
                raw_query: input.to_string(),
                source_url_or_path: normalized_input,
                is_local_file: false,
                http_headers: None,
                source_bitrate_kbps: None,
                duration_secs: None,
                replay_gain_db: None,
            });
        }

        // 5. 外部流媒体平台链接（B站/YouTube/Niconico等），调用 yt-dlp 下载缓存并提取元数据
        if let Some(ytdlp) = ytdlp_path {
            info!(
                "[Resolver] 正在调用 yt-dlp 解析流媒体平台链接: {}",
                normalized_input
            );

            let _ = std::fs::create_dir_all("cache");

            // 优先下载/解析兼容 Symphonia 的音频格式 (优先 AAC/M4A/MP4，避免 Opus 格式在无 FFmpeg 时 Symphonia 无法解码)
            let mut cmd = Command::new(ytdlp);
            cmd.env("PYTHONIOENCODING", "utf-8")
                .env("PYTHONUTF8", "1")
                .env("LANG", "zh_CN.UTF-8")
                .env("LC_ALL", "zh_CN.UTF-8");

            if let Some(ffmpeg) = ffmpeg_path {
                cmd.arg("--ffmpeg-location").arg(ffmpeg);
            }

            cmd.args([
                "--print-json",
                "-f",
                "ba[ext=m4a]/ba[acodec^=mp4a]/ba[acodec^=aac]/ba/b",
                "--no-warnings",
                "--no-playlist",
                "--paths",
                "cache",
                "-o",
                "%(id)s.%(ext)s",
                &normalized_input,
            ]);

            let timeout_duration = std::time::Duration::from_secs(35);
            let child_res = tokio::time::timeout(timeout_duration, cmd.output()).await;

            let output = match child_res {
                Ok(Ok(out)) => out,
                Ok(Err(e)) => {
                    warn!("[Resolver] 启动 yt-dlp 子进程失败: {:?}", e);
                    anyhow::bail!("启动 yt-dlp 失败: {}", e);
                }
                Err(_) => {
                    warn!("[Resolver] yt-dlp 解析/下载超时 (超过 35 秒): {}", normalized_input);
                    anyhow::bail!("yt-dlp 解析/下载超时 (超过 35 秒)");
                }
            };

            if !output.status.success() {
                let err_msg = decode_output_string(&output.stderr);
                warn!("[Resolver] yt-dlp 解析链接失败: {}", err_msg);
                anyhow::bail!("yt-dlp 解析链接失败: {}", err_msg);
            }

            let json_text = decode_output_string(&output.stdout);
            let json_line = json_text
                .lines()
                .find(|line| line.trim_start().starts_with('{') && line.trim_end().ends_with('}'))
                .unwrap_or(&json_text);

            let parsed: serde_json::Value = serde_json::from_str(json_line)
                .map_err(|e| anyhow::anyhow!("解析 yt-dlp 元数据 JSON 失败: {}", e))?;

            let title = parsed
                .get("title")
                .and_then(|v| v.as_str())
                .unwrap_or("未知网络曲目")
                .to_string();

            let source_bitrate_kbps = parsed
                .get("abr")
                .and_then(|v| v.as_f64())
                .or_else(|| parsed.get("tbr").and_then(|v| v.as_f64()));

            let duration_secs = parsed
                .get("duration")
                .and_then(|v| v.as_f64())
                .map(|d| d.round() as u64);

            // 检查 yt-dlp 是否已将音频直接下载并缓存到本地
            let filename_opt = parsed
                .get("filename")
                .or_else(|| parsed.get("_filename"))
                .and_then(|v| v.as_str());

            let mut local_cached_path = None;
            if let Some(fname) = filename_opt {
                let p = Path::new(fname);
                if p.is_file() {
                    local_cached_path = Some(p.to_string_lossy().to_string());
                } else {
                    let cached_p = Path::new("cache").join(p.file_name().unwrap_or(p.as_os_str()));
                    if cached_p.is_file() {
                        local_cached_path = Some(cached_p.to_string_lossy().to_string());
                    }
                }
            }

            if let Some(cached_file) = local_cached_path {
                info!(
                    "[Resolver] yt-dlp 解析就绪，已缓存至本地: \"{}\" -> {} (源音频码率: {:?} kbps)",
                    title, cached_file, source_bitrate_kbps
                );
                return Ok(ResolvedMedia {
                    title,
                    raw_query: input.to_string(),
                    source_url_or_path: cached_file,
                    is_local_file: true,
                    http_headers: None,
                    source_bitrate_kbps,
                    duration_secs,
                    replay_gain_db: None,
                });
            }

            // 若未生成本地文件，则回退至网络流直链
            let stream_url = parsed
                .get("url")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            if stream_url.is_empty() {
                warn!("[Resolver] yt-dlp 未能获取到有效的音频流或本地缓存");
                anyhow::bail!("yt-dlp 未能获取到有效的音频地址");
            }

            let mut headers = HashMap::new();
            if let Some(h_obj) = parsed.get("http_headers").and_then(|v| v.as_object()) {
                for (k, v) in h_obj {
                    if let Some(v_str) = v.as_str() {
                        headers.insert(k.clone(), v_str.to_string());
                    }
                }
            }

            if (stream_url.contains("bilivideo.com") || stream_url.contains("bilibili.com"))
                && !headers.contains_key("Referer")
            {
                headers.insert("Referer".to_string(), "https://www.bilibili.com/".to_string());
            }

            info!(
                "[Resolver] yt-dlp 直链解析成功: \"{}\" (源音频码率: {:?} kbps)",
                title, source_bitrate_kbps
            );
            return Ok(ResolvedMedia {
                title,
                raw_query: input.to_string(),
                source_url_or_path: stream_url,
                is_local_file: false,
                http_headers: if headers.is_empty() { None } else { Some(headers) },
                source_bitrate_kbps,
                duration_secs,
                replay_gain_db: None,
            });
        }

        // 没有 yt-dlp 时的直接尝试
        if normalized_input.starts_with("http://") || normalized_input.starts_with("https://") {
            warn!("[Resolver] 未检测到 yt-dlp，尝试将输入作为直接音频流处理: {}", normalized_input);
            return Ok(ResolvedMedia {
                title: "网络音频流".to_string(),
                raw_query: input.to_string(),
                source_url_or_path: normalized_input,
                is_local_file: false,
                http_headers: None,
                source_bitrate_kbps: None,
                duration_secs: None,
                replay_gain_db: None,
            });
        }

        anyhow::bail!("无法解析输入目标: {}", input)
    }

    /// 提取网易云音乐歌曲 ID (支持纯数字、163:id、wy:id、netmusic:id 以及完整网页链接)
    pub fn parse_netease_id(input: &str) -> Option<u64> {
        let trimmed = input.trim();

        // 纯数字 ID，通常在 4 到 12 位之间
        if trimmed.chars().all(|c| c.is_ascii_digit()) && trimmed.len() >= 4 && trimmed.len() <= 12 {
            return trimmed.parse().ok();
        }

        // 带有前缀：163:186016, wy:186016, netmusic:186016
        let lower = trimmed.to_lowercase();
        for prefix in &["163:", "wy:", "netmusic:"] {
            if lower.starts_with(prefix) {
                let rest = lower[prefix.len()..].trim();
                if let Ok(id) = rest.parse::<u64>() {
                    return Some(id);
                }
            }
        }

        // 网页链接匹配：https://music.163.com/#/song?id=186016 或 song?id=186016
        if lower.contains("music.163.com") || lower.contains("163cn.tv") {
            if let Some(idx) = lower.find("id=") {
                let after = &lower[idx + 3..];
                let digits: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
                if let Ok(id) = digits.parse::<u64>() {
                    return Some(id);
                }
            }
        }

        None
    }

    /// 解析网易云歌曲详细元数据（歌曲名、歌手）与直链音频流（参照 TartaricAcid/NetMusic）
    async fn resolve_netease_song(id: u64, raw_query: &str) -> anyhow::Result<ResolvedMedia> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(6))
            .build()?;

        // 1. 请求官方详情接口获取歌曲名与歌手
        let detail_url = format!("http://music.163.com/api/song/detail/?id={}&ids=%5B{}%5D", id, id);
        let mut title = format!("网易云音乐 - ID:{}", id);
        let mut source_bitrate_kbps = Some(320.0); // 默认高品质码率
        let mut duration_secs = None;

        if let Ok(resp) = client.get(&detail_url)
            .header("Referer", "http://music.163.com/")
            .header("User-Agent", "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36")
            .send()
            .await
        {
            if resp.status().is_success() {
                if let Ok(json) = resp.json::<serde_json::Value>().await {
                    if let Some(songs) = json["songs"].as_array() {
                        if let Some(song) = songs.first() {
                            let name = song["name"].as_str().unwrap_or("未知曲目");
                            let artists = song["artists"]
                                .as_array()
                                .map(|arr| {
                                    arr.iter()
                                        .filter_map(|a| a["name"].as_str())
                                        .collect::<Vec<_>>()
                                        .join(" / ")
                                })
                                .unwrap_or_default();

                            let fee = song["fee"].as_i64().unwrap_or(0);
                            let is_vip = fee == 1 || fee == 4;

                            if let Some(ms) = song.get("duration").and_then(|d| d.as_u64()) {
                                duration_secs = Some(ms / 1000);
                            } else if let Some(ms) = song.get("dt").and_then(|d| d.as_u64()) {
                                duration_secs = Some(ms / 1000);
                            }

                            if let Some(br) = song.get("h").and_then(|h| h.get("br")).and_then(|b| b.as_f64()) {
                                source_bitrate_kbps = Some(br / 1000.0);
                            } else if let Some(br) = song.get("m").and_then(|m| m.get("br")).and_then(|b| b.as_f64()) {
                                source_bitrate_kbps = Some(br / 1000.0);
                            } else if let Some(br) = song.get("l").and_then(|l| l.get("br")).and_then(|b| b.as_f64()) {
                                source_bitrate_kbps = Some(br / 1000.0);
                            }

                            if !artists.is_empty() {
                                title = format!("{} - {}{}", name, artists, if is_vip { " [VIP/付费]" } else { "" });
                            } else {
                                title = format!("{}{}", name, if is_vip { " [VIP/付费]" } else { "" });
                            }
                        }
                    }
                }
            }
        }

        // 2. TartaricAcid/NetMusic 标准直链格式，会被网易云 302 重定向至实际 CDN 音频流
        let stream_url = format!("https://music.163.com/song/media/outer/url?id={}.mp3", id);

        info!(
            "[Resolver] 网易云音乐解析就绪: {} (音质: ~{:.0} kbps, 直链: {})",
            title,
            source_bitrate_kbps.unwrap_or(320.0),
            stream_url
        );
        Ok(ResolvedMedia {
            title,
            raw_query: raw_query.to_string(),
            source_url_or_path: stream_url,
            is_local_file: false,
            http_headers: None,
            source_bitrate_kbps,
            duration_secs,
            replay_gain_db: None,
        })
    }

    /// 规范化 Bilibili 识别码（BV 号 / AV 号）
    pub fn normalize_bilibili(input: &str) -> Option<String> {
        let trimmed = input.trim();

        // 匹配标准 BV 号: BV[a-zA-Z0-9]{10} 或 bv[a-zA-Z0-9]{10}
        if (trimmed.starts_with("BV") || trimmed.starts_with("bv")) && trimmed.len() == 12 {
            return Some(format!("https://www.bilibili.com/video/{}", trimmed));
        }

        // 匹配标准 AV 号: av123456 或 AV123456
        if (trimmed.starts_with("av") || trimmed.starts_with("AV"))
            && trimmed.len() >= 3
            && trimmed[2..].chars().all(|c| c.is_ascii_digit())
        {
            return Some(format!("https://www.bilibili.com/video/{}", trimmed));
        }

        None
    }

    /// 规范化 Niconico 识别码与链接（支持 sm/so/nm 编号、nico: 别名、nico.ms 短链，以及 www.video.nicovideo.jp 等非标/新域名）
    pub fn normalize_niconico(input: &str) -> Option<String> {
        let trimmed = input.trim();
        let lower = trimmed.to_lowercase();

        // 1. 直传视频 ID: sm12345, so12345, nm12345 (不区分大小写，后跟纯数字)
        for prefix in &["sm", "so", "nm"] {
            if lower.starts_with(prefix) && lower.len() > prefix.len() {
                let rest = &lower[prefix.len()..];
                if rest.chars().all(|c| c.is_ascii_digit()) {
                    return Some(format!("https://www.nicovideo.jp/watch/{}", lower));
                }
            }
        }

        // 2. 快捷前缀: nico:sm12345, nico:12345, nicovideo:sm12345
        for prefix in &["nico:", "nicovideo:"] {
            if lower.starts_with(prefix) {
                let rest = lower[prefix.len()..].trim();
                let id = if rest.chars().all(|c| c.is_ascii_digit()) {
                    format!("sm{}", rest)
                } else {
                    rest.to_string()
                };
                return Some(format!("https://www.nicovideo.jp/watch/{}", id));
            }
        }

        // 3. nico.ms 短链接: https://nico.ms/sm46865356 或 nico.ms/sm46865356
        if lower.contains("nico.ms/") {
            if let Some(pos) = lower.find("nico.ms/") {
                let after = &trimmed[pos + 8..];
                let id = after
                    .split(|c| c == '?' || c == '#' || c == '/')
                    .next()
                    .unwrap_or("")
                    .trim();
                if !id.is_empty() {
                    return Some(format!("https://www.nicovideo.jp/watch/{}", id.to_lowercase()));
                }
            }
        }

        // 4. 任意含有 nicovideo.jp/watch/ 的链接（修复 www.video.nicovideo.jp, video.nicovideo.jp 等导致 yt-dlp 报 Unsupported URL 的问题）
        if lower.contains("nicovideo.jp/watch/") {
            if let Some(pos) = lower.find("nicovideo.jp/watch/") {
                let after = &trimmed[pos + 19..];
                let id = after
                    .split(|c| c == '?' || c == '#' || c == '/')
                    .next()
                    .unwrap_or("")
                    .trim();
                if !id.is_empty() {
                    return Some(format!("https://www.nicovideo.jp/watch/{}", id.to_lowercase()));
                }
            }
        }

        None
    }

    /// 智能解析本地音频文件路径（支持完整路径、相对路径、省略扩展名以及在 music/ 目录中检索）
    pub fn find_local_file(input: &str) -> Option<(PathBuf, String)> {
        // 去除外层引号（如 Windows 复制路径常见的情况: "D:\music\song.mp3"）
        let clean = input.trim_matches('"').trim_matches('\'').trim();
        if clean.is_empty() || clean.starts_with("http://") || clean.starts_with("https://") {
            return None;
        }

        let exts = ["", ".mp3", ".flac", ".wav", ".ogg", ".aac", ".m4a", ".opus", ".ape", ".wma"];
        let base_dirs = [Path::new(""), Path::new("music"), Path::new("songs"), Path::new("audio")];

        for base in &base_dirs {
            for ext in &exts {
                let candidate_str = if ext.is_empty() {
                    clean.to_string()
                } else if clean.to_lowercase().ends_with(ext) {
                    continue; // 已经自带该后缀，跳过避免双重后缀
                } else {
                    format!("{}{}", clean, ext)
                };

                let path = if base.as_os_str().is_empty() {
                    PathBuf::from(&candidate_str)
                } else {
                    base.join(&candidate_str)
                };

                if path.is_file() {
                    let title = path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or("本地音频")
                        .to_string();
                    return Some((path, title));
                }
            }
        }

        None
    }
}
