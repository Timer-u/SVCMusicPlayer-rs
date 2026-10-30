use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub minecraft: MinecraftConfig,
    pub voice: VoiceConfig,
    pub media: MediaConfig,
    pub control: ControlConfig,
    pub web: WebConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MinecraftConfig {
    pub server_host: String,
    pub server_port: u16,
    pub bot_name: String,
    pub online_mode: bool,
    #[serde(default = "default_auth_type")]
    pub auth_type: String, // "offline", "microsoft", "yggdrasil"
    #[serde(default)]
    pub microsoft_email: String,
    #[serde(default)]
    pub yggdrasil_server: String,
    #[serde(default)]
    pub yggdrasil_account: String,
    #[serde(default)]
    pub yggdrasil_password: String,
    #[serde(default)]
    pub yggdrasil_access_token: String,
    #[serde(default)]
    pub yggdrasil_profile_uuid: String,
    #[serde(default)]
    pub yggdrasil_character_name: String,
    #[serde(default = "default_compatibility_version")]
    pub compatibility_version: i32,
}

fn default_auth_type() -> String {
    "offline".to_string()
}

fn default_compatibility_version() -> i32 {
    20
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoiceConfig {
    #[serde(default)]
    pub override_host: String,
    #[serde(default)]
    pub override_port: u16,
    #[serde(default = "default_voice_bitrate")]
    pub bitrate_kbps: u32,
    #[serde(default = "default_true")]
    pub replay_gain: bool,
    #[serde(default = "default_target_lufs")]
    pub target_lufs: f32,
    #[serde(default)]
    pub group: GroupConfig,
}

fn default_target_lufs() -> f32 {
    -16.0
}

fn default_voice_bitrate() -> u32 {
    96
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_group_name")]
    pub name: String,
    #[serde(default = "default_group_type")]
    pub group_type: String, // "open", "normal", "isolated"
    #[serde(default)]
    pub password: String,
}

impl Default for GroupConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            name: default_group_name(),
            group_type: default_group_type(),
            password: String::new(),
        }
    }
}

fn default_group_name() -> String {
    "Music".to_string()
}

fn default_group_type() -> String {
    "open".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaConfig {
    #[serde(default = "default_true")]
    pub auto_download_tools: bool,
    #[serde(default = "default_bin_dir")]
    pub bin_dir: String,
    #[serde(default)]
    pub ffmpeg_path: String,
    #[serde(default)]
    pub ytdlp_path: String,
}

fn default_true() -> bool {
    true
}

fn default_bin_dir() -> String {
    "./bin".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControlConfig {
    #[serde(default = "default_command_prefix")]
    pub command_prefix: String,
    #[serde(default)]
    pub admin_players: Vec<String>,
    #[serde(default = "default_true")]
    pub auto_tpaccept: bool,
}

fn default_command_prefix() -> String {
    "!".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_bind_address")]
    pub bind_address: String,
}

fn default_bind_address() -> String {
    "127.0.0.1:8080".to_string()
}

impl Config {
    pub fn load_or_create(path_str: &str) -> anyhow::Result<Self> {
        let path = Path::new(path_str);
        if !path.exists() {
            let example_content = include_str!("../config.example.toml");
            fs::write(path, example_content)?;
            println!("[Config] 配置文件不存在，已自动生成默认配置: {}", path_str);
        }

        let content = fs::read_to_string(path)?;
        let config: Config = toml::from_str(&content)?;
        Ok(config)
    }
}
