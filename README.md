# SVCMusicPlayer-rs

基于 Rust 构建的高性能、低开销 Minecraft [Simple Voice Chat (SVC)](https://github.com/henkelmax/simple-voice-chat) 独立语音伴播假人机器人。

脱离传统的复杂 Python/Node.js 运行时环境，单个二进制可执行文件即可独立运行，支持跨版本连接 Minecraft 服务器，提供游戏内公屏点歌和内嵌现代化 Web 控制台。

---

## 核心特性

- 🚀 **极低开销 & 单文件部署**：基于 Rust 编写，常驻内存仅约 20MB ~ 30MB。单二进制可执行文件开箱即用。
- 🎙️ **完整的 SVC UDP 协议实现**：严格依照 Simple Voice Chat 协议规范实现，支持 AES-128-GCM 加密握手、会话认证、心跳保活与 20ms 精准时钟推流。
- 🌐 **内嵌现代化 Web 控制台**：单页应用（HTML5 / Tailwind CSS）在编译期直接打包进二进制程序中，无需附带额外静态资源目录，开箱即用。
- 🎵 **双轨解码 & 外部工具自愈**：
  - **纯 Rust 引擎**：内嵌 `symphonia` 解码器与 `rusty-opus` 纯 Rust 编码器，本地音频（MP3、WAV、FLAC、OGG 等）与网络音频直链**无需安装 FFmpeg 即可直接播放**。
  - **流媒体增强引擎**：支持自动检测或一键从源拉取 `yt-dlp` 与 `ffmpeg`，直接解析播放 B站、YouTube、网易云等复杂平台的音视频链接。
- 🎮 **游戏内聊天指令控制**：玩家无需切出游戏，直接在 Minecraft 公屏打字即可交互点歌、切歌、暂停与调音。
- 🔐 **双认证模式**：支持离线服务器（Offline Mode）免认证登录，同时支持微软正版账号（Microsoft OAuth）。

---

## 快速上手

### 1. 编译构建

确保系统已安装 Rust 环境（推荐 1.80+）：

```bash
cd E:\qqbot\nonebot\MineGroupBridge\SVCMusicPlayer-rs
cargo build --release
```

编译完成后，可执行文件位于 `target/release/svc_music_player.exe` (或 Linux 平台的 `svc_music_player`)。

### 2. 配置文件说明

首次运行程序将自动生成 `config.toml`。主要配置项如下：

```toml
[minecraft]
server_host = "127.0.0.1"      # Minecraft 服务器地址
server_port = 25565            # Minecraft 服务器端口
bot_name = "VoicePlayer"       # 假人游戏昵称
online_mode = false            # 离线模式为 false；微软正版登录设为 true
microsoft_email = ""           # 正版模式时的微软邮箱
compatibility_version = 20     # SVC 协议兼容版本 (SVC 2.6.x 对应 20)

[voice]
override_host = ""             # 仅在穿透或使用独立语音代理时强制覆盖
override_port = 0

[media]
auto_download_tools = true     # 未检测到 yt-dlp 时是否自动从 GitHub 下载
bin_dir = "./bin"              # 外部工具存放目录
ffmpeg_path = ""               # 自定义 ffmpeg 绝对路径 (留空优先找 ./bin 再找 PATH)
ytdlp_path = ""                # 自定义 yt-dlp 绝对路径

[control]
command_prefix = "!"           # 游戏内指令前缀 (默认 !)
admin_players = []             # 管理员白名单，留空表示全服可用

[web]
enabled = true                 # 启用内嵌网页控制台
bind_address = "127.0.0.1:8080"# 控制台监听地址
```

### 3. 运行程序

```bash
cargo run
# 或者运行 Release 二进制文件
./target/release/svc_music_player
```

---

## 游戏内指令

在 Minecraft 服务器中，站在假人附近，在公屏聊天框发送：

| 指令 | 说明 | 示例 |
| :--- | :--- | :--- |
| `!play <链接/路径>` | 提交点播曲目，空闲时立即播放，忙碌时排入队列 | `!play https://.../song.mp3`<br>`!play https://www.bilibili.com/video/BV...`<br>`!play ./music/test.wav` |
| `!pause` | 暂停当前播放 | `!pause` |
| `!resume` | 恢复播放 | `!resume` |
| `!skip` | 跳过当前曲目，自动播放队列下一首 | `!skip` |
| `!stop` | 停止播放并清空整个待播队列 | `!stop` |
| `!vol <0-100>` | 调整音量百分比 | `!vol 50` |
| `!help` | 查看帮助提示 | `!help` |

---

## Web 控制台使用

当开启 `web.enabled = true` 时，用浏览器访问：
👉 `http://127.0.0.1:8080` (或服务器配置的 IP:端口)

控制台界面提供：
1. **连接状态指示**：直观展示 Minecraft TCP 登录状态与 SVC 语音通道 UDP 握手状态。
2. **实时播控**：当前曲目显示、播放/暂停/停播、即时音量推子。
3. **点播与队列管理**：输入链接一键提交，实时查看待播排队列表。
