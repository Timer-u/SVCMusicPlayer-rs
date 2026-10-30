# SVCMusicPlayer-rs

基于 Rust 构建的高性能、低开销 Minecraft [Simple Voice Chat (SVC)](https://github.com/henkelmax/simple-voice-chat) 独立语音伴播假人机器人。

脱离传统的复杂 Python/Node.js 运行时环境，单个二进制可执行文件即可独立运行，支持跨版本连接 Minecraft 服务器，提供游戏内公屏点歌和内嵌现代化水色极简 Web 控制台。

---

## 核心特性

- 🚀 **极低开销 & 单文件部署**：基于 Rust Edition 2024 编写，常驻内存仅约 20MB ~ 30MB。单二进制可执行文件开箱即用。
- 🎙️ **完整的 SVC UDP 协议实现**：严格依照 Simple Voice Chat 协议规范实现，支持 AES-128-GCM 加密握手、会话认证、心跳保活、SVC 语音群组无距离收听与 20ms 精准时钟推流。
- 🔊 **智能 ReplayGain 响度平衡与防爆音限幅**：
  - 动态 EBU R128 滑动窗口响度追踪与元数据静态增益预计算；
  - 软拐点峰值限制器（Soft-Knee Peak Limiter，tanh 软饱和渐近），杜绝忽大忽小与大音量削波爆音。
- 🔁 **驻留式歌单状态机与多模式自动连播**：
  - 歌单 JSON 本地持久化，支持 4 种播放模式：**列表循环**（默认）、**单曲循环**、**随机播放**（洗牌不重复）、**顺序播放**（播完停机）；
  - 单曲播放完毕自动切歌，单首损坏智能熔断跳过；
  - 具备插播、置顶下一首、切回上一首、跳转与单曲删除等完整状态机管理。
- 🌊 **内嵌水色极简 Web 控制台与 REST API**：
  - 单页应用（HTML5 / Tailwind CSS）在编译期直接打包进二进制程序中，无需附带额外静态资源目录；
  - 明亮清爽的水色极简设计（声波封面、状态胶囊、播放进度条与时间显示、实时 ReplayGain 增益徽章与开关、模式切换、即时调音与歌单管理）。
- 🎵 **双轨解码 & 外部工具自愈**：
  - **纯 Rust 引擎**：内嵌 `symphonia` 异步解码器与 `rusty-opus` 纯 Rust 编码器，本地音频（MP3、WAV、FLAC、OGG 等）与网络音频直链**无需安装 FFmpeg 即可直接播放**；
  - **流媒体增强引擎**：支持自动检测或一键从源拉取 `yt-dlp` 与 `ffmpeg`，直接解析播放 B站、YouTube、网易云等复杂平台的音视频链接。
- 🎮 **游戏内聊天指令解耦与全功能扩展**：玩家无需切出游戏，直接在 Minecraft 公屏打字即可交互点歌、模式切换、分页看歌单、置顶、删除、调音与传送自动同意（CMI TPA 支持）。
- 🔐 **多元认证模式**：支持离线服务器（Offline Mode）、第三方外置登录（authlib-injector / Blessing Skin / LittleSkin），以及微软官方正版登录（Microsoft OAuth）。

---

## 快速上手

### 1. 编译构建

项目内置了 PowerShell 双端自动化极速编译脚本 `build.ps1`：

```powershell
# 编译双端交付产物（Windows + WSL Linux 高速构建）
./build.ps1 -Target all

# 仅编译 Windows 版本
./build.ps1 -Target windows

# 仅编译 Linux 版本 (利用 WSL 原生 ext4 缓存，绕开 9P 跨文件系统瓶颈)
./build.ps1 -Target linux
```

编译完成后，交付文件将自动拷贝至 `dist/windows-x86_64/` 与 `dist/linux-amd64/`。

### 2. 配置文件说明

首次运行程序将自动生成 `config.toml`。主要配置项如下：

```toml
[minecraft]
server_host = "127.0.0.1"      # Minecraft 服务器地址
server_port = 25565            # Minecraft 服务器端口
bot_name = "VoicePlayer"       # 假人游戏昵称
auth_type = "offline"          # 认证模式: "offline", "microsoft", "yggdrasil"
online_mode = false            # 兼容旧项
compatibility_version = 20     # SVC 协议兼容版本 (SVC 2.6.x 对应 20)

[voice]
override_host = ""             # 仅在穿透或使用独立语音代理时强制覆盖
override_port = 0
bitrate_kbps = 96              # Opus 音频推流码率 (默认 96 kbps)
replay_gain = true             # 是否启用 ReplayGain 智能响度平衡与动态范围限幅
target_lufs = -16.0            # 目标综合响度标准 (单位: LUFS，推荐 -20.0 ~ -14.0)

[voice.group]
enabled = true                 # 进服是否自动创建或加入 SVC 语音群组
name = "Music"                 # 语音群组名称 (全服群成员无视距离与维度收听)
group_type = "open"            # open (开放), normal (普通), isolated (私密)
password = ""

[media]
auto_download_tools = true     # 未检测到 yt-dlp 时是否自动从 GitHub 下载
bin_dir = "./bin"              # 外部工具存放目录
ffmpeg_path = ""               # 自定义 ffmpeg 绝对路径 (留空优先找 ./bin 再找 PATH)
ytdlp_path = ""                # 自定义 yt-dlp 绝对路径

[control]
command_prefix = "!"           # 游戏内指令前缀 (默认 !)
admin_players = []             # 管理员白名单，留空表示全服可用
auto_tpaccept = true           # 是否自动同意玩家 TPA 传送请求 (CMI 支持)

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

## 游戏内聊天指令

在 Minecraft 服务器中，站在假人附近，在公屏聊天框发送（前缀支持自定义，默认为 `!`）：

| 指令 | 说明 | 示例 |
| :--- | :--- | :--- |
| `!play <链接/歌名/ID>` | 提交点播曲目；空闲时立即播放，播放中追加至歌单末尾 | `!play 186016`<br>`!play BV1xx`<br>`!play https://.../song.mp3` |
| `!playnow <目标>` 或 `!pn <目标>` | 立即插播曲目（插入当前曲目下一位并立刻切歌） | `!pn 186016` |
| `!pause` / `!resume` | 暂停当前播放 / 恢复播放 | `!pause` / `!resume` |
| `!stop` | 停止当前播放（**歌单保持保留**，清空请用 `!clear`） | `!stop` |
| `!skip` / `!next` | 手动跳到下一首（单曲循环下手动跳歌仍会前进至下一曲） | `!skip` |
| `!prev` | 手动切回上一首 | `!prev` |
| `!mode [loop\|single\|shuffle\|seq]` | 查询或切换播放模式（列表循环/单曲循环/随机播放/顺序播放） | `!mode shuffle` |
| `!loop` / `!single` / `!shuffle` / `!seq` | 快捷切换播放模式 | `!shuffle` |
| `!list [页码]` / `!queue` / `!pl` | 分页查看歌单（每页 6 首，显示当前播放标记 `▶` 与总曲目） | `!list 2` |
| `!jump <序号>` / `!skipto <序号>` | 直接跳转播放指定序号 (1-based) 的曲目 | `!jump 3` |
| `!top <序号>` / `!pin <序号>` | 将指定序号曲目置顶为下一首播放 | `!top 5` |
| `!rm <序号>` / `!del <序号>` | 从歌单中移除指定序号的曲目 | `!rm 2` |
| `!clear` | 停止播放并清空整个歌单 | `!clear` |
| `!rg [on\|off]` | 查看或开关 ReplayGain 智能响度平衡与实时增益 | `!rg on` |
| `!vol <0-100>` | 调整音量百分比 | `!vol 80` |
| `!group join\|leave\|list` | 查看或加入指定的 SVC 语音群组 | `!group join Music` |
| `!tpaccept` / `!tpa` | 手动同意玩家传送请求 | `!tpa` |
| `!help` | 查看完整指令帮助 | `!help` |

---

## Web 控制台与 REST API

当开启 `web.enabled = true` 时，用浏览器访问：
👉 `http://127.0.0.1:8080` (或服务器配置的 IP:端口)

### 1. 现代化水色极简界面
- **顶栏状态徽章**：实时监控 Minecraft TCP 在线、SVC 语音握手及当前群组。
- **主播控卡片**：
  - 动态声波封面、曲目标题与路径、实时推流码率（如 `96.0 kbps`）；
  - **实时 ReplayGain 响度增益标签**（如 `RG: +1.8 dB` 或 `RG: 已关闭`）；
  - 平滑水色进度条与时间刻度（`MM:SS / MM:SS`）；
  - 播放模式循环切换按钮、上一首、播放/暂停、下一首、停止、ReplayGain 开关与音量滑块。
- **点播输入卡片**：支持回车提交，并排提供“加入列表”与“立即播放”两个入口。
- **驻留歌单卡片**：
  - 歌单总曲目与模式徽章、“清空歌单”按钮；
  - 列表项高亮指示正在播放的曲目，支持单曲跳转播放、置顶为下一首与删除。

### 2. REST API 接口列表
- `GET /api/status`: 获取播放器完整聚合状态（MC/语音状态、当前曲目、时长、实时增益、歌单列表 `playlist` 等）。
- `POST /api/play`: 点播曲目（Body: `{"url": "...", "play_now": false}`）。
- `POST /api/control`: 播控与歌单管理（Body: `{"action": "pause|resume|stop|skip|prev|volume|mode|replay_gain|playlist_jump|playlist_top|playlist_remove|playlist_clear", ...}`）。

---

## 协议与自动化测试

运行完整自动化测试套件：

```bash
cargo test
```

测试集涵盖：
- SVC UDP VarInt 编解码、AES-128-GCM 加密握手与音频数据包往返；
- Opus 48kHz 正弦波编码推流与码率统计；
- `LoudnessProcessor` 标签解析、静态分析与软拐点防爆音限幅纯函数测试；
- `PlaylistManager` 四种播放模式流转、循环回绕、索引修正与洗牌不重复验证；
- 扩展聊天指令识别与解析。
