use std::collections::HashMap;
use std::path::Path;
use std::process::Stdio;
use tokio::io::AsyncReadExt;
use tokio::process::{Child, Command};
use tracing::{debug, info};

use crate::audio::loudness::LoudnessProcessor;

pub struct AudioDecoder {
    ffmpeg_child: Option<Child>,
    ffmpeg_stdout: Option<tokio::process::ChildStdout>,
    symphonia_samples: Option<std::vec::IntoIter<i16>>,
    pub duration_secs: Option<u64>,
    pub precomputed_gain_db: Option<f32>,
}

impl AudioDecoder {
    /// 构造适用于当前流媒体源的 HTTP Headers（自动为 B站与网易云补全 Referer 与 User-Agent）
    pub fn build_headers(
        source: &str,
        custom_headers: Option<&HashMap<String, String>>,
    ) -> (String, HashMap<String, String>) {
        let mut map = HashMap::new();
        let default_ua = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36";
        map.insert("User-Agent".to_string(), default_ua.to_string());

        if source.contains("bilibili.com") || source.contains("bilivideo.com") || source.contains("hdslb.com") {
            map.insert("Referer".to_string(), "https://www.bilibili.com/".to_string());
        } else if source.contains("163.com") || source.contains("126.net") {
            map.insert("Referer".to_string(), "http://music.163.com/".to_string());
        }

        if let Some(custom) = custom_headers {
            for (k, v) in custom {
                map.insert(k.clone(), v.clone());
            }
        }

        let mut ffmpeg_str = String::new();
        for (k, v) in &map {
            ffmpeg_str.push_str(&format!("{}: {}\r\n", k, v));
        }

        (ffmpeg_str, map)
    }

    pub async fn open(
        source: &str,
        is_local_file: bool,
        http_headers: Option<&HashMap<String, String>>,
        ffmpeg_path: Option<&Path>,
    ) -> anyhow::Result<Self> {
        let (ffmpeg_headers, header_map) = Self::build_headers(source, http_headers);

        if let Some(ffmpeg) = ffmpeg_path {
            info!("[Decoder] 使用 FFmpeg 管道解码: {}", source);
            let mut cmd = Command::new(ffmpeg);
            if !is_local_file && (source.starts_with("http://") || source.starts_with("https://")) {
                cmd.args([
                    "-reconnect", "1",
                    "-reconnect_streamed", "1",
                    "-reconnect_delay_max", "5",
                    "-headers", &ffmpeg_headers,
                ]);
            }
            cmd.args([
                "-i", source,
                "-f", "s16le",       // 16-bit 有符号整数
                "-ar", "48000",      // Simple Voice Chat 要求的 48kHz
                "-ac", "1",          // 单声道
                "-vn",               // 丢弃视频
                "-loglevel", "error",
                "pipe:1",
            ]);
            cmd.stdout(Stdio::piped());
            cmd.stderr(Stdio::inherit());

            let mut child = cmd.spawn()?;
            let stdout = child.stdout.take().ok_or_else(|| {
                anyhow::anyhow!("无法获取 FFmpeg 标准输出管道")
            })?;

            // 针对本地文件尝试探针快速提取时长与 ReplayGain 元数据
            let (mut duration_secs, mut precomputed_gain_db) = (None, None);
            if is_local_file {
                let (d, g) = Self::probe_local_file_metadata(source);
                duration_secs = d;
                precomputed_gain_db = g;
            }

            return Ok(Self {
                ffmpeg_child: Some(child),
                ffmpeg_stdout: Some(stdout),
                symphonia_samples: None,
                duration_secs,
                precomputed_gain_db,
            });
        }

        // 纯 Rust Symphonia 解码通道
        info!("[Decoder] 使用纯 Rust (Symphonia) 解码: {}", source);

        let downloaded_bytes = if !is_local_file {
            let mut client_builder = reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(120));
            if let Some(ua) = header_map.get("User-Agent") {
                client_builder = client_builder.user_agent(ua);
            }
            let client = client_builder.build()?;
            let mut req = client.get(source);
            for (k, v) in &header_map {
                req = req.header(k, v);
            }
            let resp = req.send().await?;
            if !resp.status().is_success() {
                anyhow::bail!("下载网络音频流失败 (HTTP {})", resp.status());
            }
            Some(resp.bytes().await?)
        } else {
            None
        };

        let src = source.to_string();
        let (samples, duration_secs, precomputed_gain_db) = tokio::task::spawn_blocking(move || {
            Self::decode_with_symphonia(&src, is_local_file, downloaded_bytes)
        }).await??;

        Ok(Self {
            ffmpeg_child: None,
            ffmpeg_stdout: None,
            symphonia_samples: Some(samples.into_iter()),
            duration_secs,
            precomputed_gain_db,
        })
    }

    /// 尝试读取指定数量的 16-bit 单声道采样 (例如 960 个采样对应 20ms)
    pub async fn read_samples(&mut self, count: usize) -> anyhow::Result<Option<Vec<i16>>> {
        if let Some(stdout) = &mut self.ffmpeg_stdout {
            let bytes_to_read = count * 2; // 16-bit = 2 bytes
            let mut byte_buf = vec![0u8; bytes_to_read];
            let mut total_read = 0;

            while total_read < bytes_to_read {
                let n = stdout.read(&mut byte_buf[total_read..]).await?;
                if n == 0 {
                    break;
                }
                total_read += n;
            }

            if total_read == 0 {
                return Ok(None); // EOF
            }

            let samples_read = total_read / 2;
            let mut samples = Vec::with_capacity(samples_read);
            for chunk in byte_buf[..samples_read * 2].chunks_exact(2) {
                samples.push(i16::from_le_bytes([chunk[0], chunk[1]]));
            }

            // 若末尾不足 count，补零对齐
            if samples.len() < count {
                samples.resize(count, 0);
            }

            return Ok(Some(samples));
        }

        if let Some(iter) = &mut self.symphonia_samples {
            let mut chunk = Vec::with_capacity(count);
            for _ in 0..count {
                if let Some(sample) = iter.next() {
                    chunk.push(sample);
                } else {
                    break;
                }
            }

            if chunk.is_empty() {
                return Ok(None);
            }

            if chunk.len() < count {
                chunk.resize(count, 0);
            }

            return Ok(Some(chunk));
        }

        Ok(None)
    }

    fn extract_replaygain_from_tags(tags: &[symphonia::core::meta::Tag]) -> Option<f32> {
        for tag in tags {
            let key = tag.key.to_uppercase();
            if key.contains("REPLAYGAIN_TRACK_GAIN") || key.contains("REPLAYGAIN_ALBUM_GAIN") {
                if let Some(g) = LoudnessProcessor::parse_replaygain_tag(&tag.value.to_string()) {
                    return Some(g);
                }
            }
        }
        None
    }

    fn probe_local_file_metadata(path_str: &str) -> (Option<u64>, Option<f32>) {
        use symphonia::core::formats::FormatOptions;
        use symphonia::core::io::MediaSourceStream;
        use symphonia::core::meta::MetadataOptions;
        use symphonia::core::probe::Hint;

        let file = match std::fs::File::open(path_str) {
            Ok(f) => f,
            Err(_) => return (None, None),
        };
        let mss = MediaSourceStream::new(Box::new(file), Default::default());
        let mut hint = Hint::new();
        if let Some(ext) = Path::new(path_str).extension().and_then(|s| s.to_str()) {
            hint.with_extension(ext);
        }

        let mut probed = match symphonia::default::get_probe().format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        ) {
            Ok(p) => p,
            Err(_) => return (None, None),
        };

        let mut duration_secs = None;
        if let Some(track) = probed
            .format
            .tracks()
            .iter()
            .find(|t| t.codec_params.codec != symphonia::core::codecs::CODEC_TYPE_NULL)
        {
            if let (Some(n_frames), Some(sample_rate)) =
                (track.codec_params.n_frames, track.codec_params.sample_rate)
            {
                if sample_rate > 0 {
                    duration_secs = Some(n_frames / sample_rate as u64);
                }
            }
        }

        let mut gain_db = None;
        if let Some(meta) = probed.metadata.get().as_ref().and_then(|m| m.current()) {
            gain_db = Self::extract_replaygain_from_tags(meta.tags());
        }
        if gain_db.is_none() {
            if let Some(meta) = probed.format.metadata().current() {
                gain_db = Self::extract_replaygain_from_tags(meta.tags());
            }
        }

        (duration_secs, gain_db)
    }

    fn decode_with_symphonia(
        source: &str,
        is_local_file: bool,
        downloaded_bytes: Option<bytes::Bytes>,
    ) -> anyhow::Result<(Vec<i16>, Option<u64>, Option<f32>)> {
        use symphonia::core::audio::SampleBuffer;
        use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
        use symphonia::core::formats::FormatOptions;
        use symphonia::core::io::MediaSourceStream;
        use symphonia::core::meta::MetadataOptions;
        use symphonia::core::probe::Hint;

        let mss = if let Some(b) = downloaded_bytes {
            let cursor = std::io::Cursor::new(b);
            MediaSourceStream::new(Box::new(cursor), Default::default())
        } else if is_local_file {
            let file = std::fs::File::open(source)?;
            MediaSourceStream::new(Box::new(file), Default::default())
        } else {
            anyhow::bail!("未提供本地文件或下载字节");
        };

        let mut hint = Hint::new();
        let clean_path = source.split('?').next().unwrap_or(source);
        if let Some(ext) = Path::new(clean_path).extension().and_then(|s| s.to_str()) {
            let lower_ext = ext.to_lowercase();
            if lower_ext == "m4s" || lower_ext == "m4a" || lower_ext == "mp4" {
                hint.with_extension("m4a");
            } else {
                hint.with_extension(&lower_ext);
            }
        }

        let mut probed = symphonia::default::get_probe().format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )?;

        let mut precomputed_gain_db = None;
        if let Some(meta) = probed.metadata.get().as_ref().and_then(|m| m.current()) {
            precomputed_gain_db = Self::extract_replaygain_from_tags(meta.tags());
        }

        let mut format = probed.format;
        if precomputed_gain_db.is_none() {
            if let Some(meta) = format.metadata().current() {
                precomputed_gain_db = Self::extract_replaygain_from_tags(meta.tags());
            }
        }

        let track = format
            .tracks()
            .iter()
            .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
            .ok_or_else(|| anyhow::anyhow!("未在媒体中找到可解码的音频轨道"))?;

        let sample_rate = track.codec_params.sample_rate.unwrap_or(48000);
        let channels = track.codec_params.channels.map(|c| c.count()).unwrap_or(2);

        let mut decoder = symphonia::default::get_codecs()
            .make(&track.codec_params, &DecoderOptions::default())?;

        let track_id = track.id;
        let mut raw_mono_samples: Vec<i16> = Vec::new();

        while let Ok(packet) = format.next_packet() {
            if packet.track_id() != track_id {
                continue;
            }

            match decoder.decode(&packet) {
                Ok(decoded) => {
                    let mut sample_buf = SampleBuffer::<i16>::new(
                        decoded.capacity() as u64,
                        *decoded.spec(),
                    );
                    sample_buf.copy_interleaved_ref(decoded);
                    let interleaved = sample_buf.samples();

                    if channels == 1 {
                        raw_mono_samples.extend_from_slice(interleaved);
                    } else {
                        // 混音多声道到单声道
                        for frame in interleaved.chunks_exact(channels) {
                            let sum: i32 = frame.iter().map(|&s| s as i32).sum();
                            let mono = (sum / channels as i32) as i16;
                            raw_mono_samples.push(mono);
                        }
                    }
                }
                Err(e) => {
                    debug!("[Decoder] 解码音频包异常: {:?}", e);
                    break;
                }
            }
        }

        let final_samples = if sample_rate != 48000 && !raw_mono_samples.is_empty() {
            debug!("[Decoder] 重采样从 {}Hz 至 48000Hz", sample_rate);
            let ratio = 48000.0 / sample_rate as f64;
            let target_len = (raw_mono_samples.len() as f64 * ratio) as usize;
            let mut resampled = Vec::with_capacity(target_len);
            for i in 0..target_len {
                let src_idx = (i as f64 / ratio) as usize;
                if src_idx < raw_mono_samples.len() {
                    resampled.push(raw_mono_samples[src_idx]);
                }
            }
            resampled
        } else {
            raw_mono_samples
        };

        let duration_secs = if !final_samples.is_empty() {
            Some((final_samples.len() / 48000) as u64)
        } else {
            None
        };

        if precomputed_gain_db.is_none() && !final_samples.is_empty() {
            precomputed_gain_db = LoudnessProcessor::analyze_static_gain_db(&final_samples, -16.0);
        }

        Ok((final_samples, duration_secs, precomputed_gain_db))
    }
}

impl Drop for AudioDecoder {
    fn drop(&mut self) {
        if let Some(mut child) = self.ffmpeg_child.take() {
            let _ = child.start_kill();
        }
    }
}
