# SVCMusicPlayer-rs

基于 Rust 构建的高性能、低开销 Minecraft [Simple Voice Chat (SVC)](https://github.com/henkelmax/simple-voice-chat) 独立语音伴播假人机器人。

单个二进制文件独立运行，无需安装复杂 Python/Node.js 环境。支持全版本连接、智能响度平衡、驻留式歌单自动连播、游戏内聊天点歌以及内嵌水色极简 Web 控制台。

---

## 📌 目录导航

- [✨ 核心特性](#-核心特性)
- [🎮 第一部分：玩家点歌与交互指南](#-第一部分玩家点歌与交互指南)
  - [1.1 支持点播的音频源与格式](#11-支持点播的音频源与格式)
  - [1.2 游戏内聊天指令（拟真交互示例）](#12-游戏内聊天指令拟真交互示例)
  - [1.3 水色极简 Web 控制台使用](#13-水色极简-web-控制台使用)
- [⚙️ 第二部分：运维部署与配置说明](#️-第二部分运维部署与配置说明)
  - [2.1 源码编译与运行](#21-源码编译与运行)
  - [2.2 常见场景快速起步模板](#22-常见场景快速起步模板)
  - [2.3 配置文件全参数解析表](#23-配置文件全参数解析表)
- [🛠️ 第三部分：开发者协议与架构](#️-第三部分开发者协议与架构)
  - [3.1 核心音频引擎与协议实现](#31-核心音频引擎与协议实现)
  - [3.2 Web REST API 端点概要速查](#32-web-rest-api-端点概要速查)
  - [3.3 自动化测试与回归验证](#33-自动化测试与回归验证)

---

## ✨ 核心特性

- 🚀 **极低开销 & 单文件部署**：Rust Edition 2024 编写，常驻内存仅约 20MB ~ 30MB，静态打包无多余依赖。
- 🎙️ **完整的 SVC UDP 协议实现**：严格遵循 Simple Voice Chat 协议，支持 AES-128-GCM 加密握手、心跳保活、SVC 语音群组无距离收听与 20ms 精准 Opus 推流。
- 🔊 **智能 ReplayGain 响度平衡 & 动态限幅**：滑动窗口 EBU R128 追踪与整曲静态均方能量预扫描；搭配软拐点峰值限制器（tanh 软饱和），杜绝大音量削波与忽大忽小。
- 🔁 **驻留式歌单状态机 & 4 种播放模式**：本地 JSON 持久化歌单，支持列表循环、单曲循环、随机洗牌（跨轮不重复）与顺序播放；单曲播完自动切歌，损坏曲目智能熔断。
- 🌊 **内嵌水色极简 Web 控制台**：单页应用在编译期直接打包进二进制，采用清新水色浅色主题，提供声波指示、平滑进度条、模式切换与单曲置顶/删除。
- 🎵 **双轨解码 & 外部工具自愈**：纯 Rust `symphonia` 异步解码器与外部 `yt-dlp` / `ffmpeg` 增强引擎互补，自动适配直链、B站、网易云与 YouTube 等平台。
- 🔐 **全场景认证支持**：兼容离线模式（Offline Mode）、第三方外置登录（authlib-injector / Blessing Skin / LittleSkin）及微软官方正版账号（OAuth）。

---

## 🎮 第一部分：玩家点歌与交互指南

### 1.1 支持点播的音频源与格式

在游戏内公屏发送 `!play <目标>` 或 `!pn <目标>` 时，支持以下输入格式：

| 平台 / 类型 | 支持的输入范例 | 解析行为 |
| :--- | :--- | :--- |
| **网易云音乐** | `!play 186016`<br>`!play 163:186016`<br>`!play wy:186016`<br>`!play https://music.163.com/#/song?id=186016` | 自动请求官方接口提取歌名、歌手与音质，通过直链推流 |
| **Bilibili (B站)** | `!play BV1xx411c7mD`<br>`!play https://www.bilibili.com/video/BV1xx411c7mD` | 自动规范化并通过 yt-dlp 抓取高品质音频流 |
| **Niconico (N站)** | `!play sm46865356`<br>`!play https://www.nicovideo.jp/watch/sm46865356` | 自动识别 sm/so/nm 编号并解析推流 |
| **网络直链** | `!play https://example.com/audio/track.mp3`<br>`!play http://.../song.flac` | 支持常见格式（MP3, WAV, FLAC, OGG, AAC, M4A），纯 Rust 解码 |
| **本地音频文件** | `!play ./music/favorite.wav`<br>`!play favorite.flac` | 自动在项目根目录及 `music/` 文件夹下递归匹配补全 |

---

### 1.2 游戏内聊天指令（拟真交互示例）

在游戏公屏直接发送聊天即可控制机器人（默认前缀为 `!`）：

#### 1. 基础播放与插播
```text
<Alice> !play 186016
[Bot] 已加入播放列表: 群青 - YOASOBI

<Bob> !pn https://www.bilibili.com/video/BV1xx
[Bot] 立即播放: 【东方】Bad Apple!! ＰＶ【影絵】

<Alice> !pause
[Bot] 播放已暂停

<Alice> !resume
[Bot] 播放已恢复

<Bob> !stop
[Bot] 已停止当前播放（歌单已保留，清空请用 !clear）
```

#### 2. 切歌与上一首
```text
<Alice> !skip
[Bot] 已切歌，正在播放: 夜に駆ける - YOASOBI

<Alice> !prev
[Bot] 已切回上一首，正在播放: 群青 - YOASOBI
```

#### 3. 播放模式切换
```text
<Alice> !mode
[Bot] 当前播放模式: 列表循环 (可用: !mode loop|single|shuffle|seq)

<Bob> !mode shuffle
[Bot] 播放模式已切换为: 随机播放

<Alice> !single
[Bot] 播放模式已切换为: 单曲循环
```

#### 4. 歌单与队列管理
```text
<Alice> !list
[Bot] 当前歌单 [模式: 列表循环, 共 8 首, 第 1/2 页]:
 1. ▶ 群青 - YOASOBI
 2. 夜に駆ける - YOASOBI
 3. 怪物 - YOASOBI
 4. 祝福 - YOASOBI
 5. 偶像 - YOASOBI
 6. 勇者 - YOASOBI

<Bob> !top 5
[Bot] 已置顶为下一首: 偶像 - YOASOBI

<Alice> !jump 4
[Bot] 正在播放: 祝福 - YOASOBI

<Bob> !rm 2
[Bot] 已从歌单移除: 夜に駆ける - YOASOBI

<Alice> !clear
[Bot] 已停止播放并清空全部歌单
```

#### 5. 调音、语音群组与系统
```text
<Alice> !vol 75
[Bot] 音量已调整为: 75%

<Bob> !rg
[Bot] ReplayGain 状态: 开启 (实时增益: +1.8 dB)

<Bob> !rg off
[Bot] ReplayGain 响度平衡已关闭

<Alice> !group join Music
[Bot] 正在加入语音群组: Music...

<Alice> !tpa
[Bot] 已同意玩家传送请求

<Alice> !help
[Bot] 指令列表: !play, !pn, !pause, !resume, !skip, !prev, !stop, !mode, !loop, !single, !shuffle, !seq, !list, !jump, !top, !rm, !clear, !vol, !rg, !group, !tpa
```

> [!NOTE]
> 若配置文件中指定了 `admin_players` 管理员名单，非管理员执行 `!stop`、`!clear`、`!vol`、`!rm` 等敏感指令时将被拒绝。

---

### 1.3 水色极简 Web 控制台使用

开启 `web.enabled = true` 后，浏览器访问 `http://127.0.0.1:8080`：

1. **状态栏**：顶栏实时呈现 Minecraft 登录状态、SVC 语音握手状态及当前加入的语音群组；
2. **主播控面板**：
   - 动态声波卡片展示当前曲名与路径；
   - 实时推流码率（如 `96.0 kbps`）与 **ReplayGain 增益标签**（如 `RG: +1.8 dB`）；
   - 平滑水色进度条（已播时长 / 总时长）；
   - 播控按钮组（模式切换、上一首、播放/暂停、下一首、停止、ReplayGain 开关、音量滑块）；
3. **点播面板**：支持输入链接并排提供 **“加入列表”** 与 **“立即播放”** 两个按钮；
4. **歌单面板**：高亮标出当前播放项，每行曲目均可一键 **跳转播放**、**置顶下一首** 或 **删除**，顶部支持一键清空。

---

## ⚙️ 第二部分：运维部署与配置说明

### 2.1 源码编译与运行

本项目采用标准 Rust 工具链，无需额外脚本即可在各平台完成编译：

#### Windows 构建
```powershell
# 编译 Release 高性能版本
cargo build --release

# 运行程序
.\target\release\svc_music_player.exe
```

#### Linux 构建
```bash
# 编译 Release 高性能版本
cargo build --release

# 运行程序
./target/release/svc_music_player
```

> [!TIP]
> 预编译产物可放置于 `dist/windows-x86_64/` 与 `dist/linux-amd64/`，随行附带 `config.example.toml` 即可分发部署。

---

### 2.2 常见场景快速起步模板

首次运行会自动生成 `config.toml`。针对三种最常见的服务器环境，可直接复制以下配置：

#### 场景 1：普通离线服务器（Offline Mode）
```toml
[minecraft]
server_host = "127.0.0.1"
server_port = 25565
bot_name = "VoicePlayer"
auth_type = "offline"
compatibility_version = 20

[voice]
bitrate_kbps = 96
replay_gain = true

[voice.group]
enabled = true
name = "Music"
group_type = "open"

[web]
enabled = true
bind_address = "0.0.0.0:8080"
```

#### 场景 2：第三方外置登录（LittleSkin / Blessing Skin）
```toml
[minecraft]
server_host = "mc.example.com"
server_port = 25565
auth_type = "yggdrasil"
yggdrasil_server = "https://littleskin.cn/api/yggdrasil"
yggdrasil_account = "your_email@example.com"
yggdrasil_password = "your_password"
# 或者使用已有 Token 免密登录：
# yggdrasil_access_token = "..."
# yggdrasil_profile_uuid = "..."
bot_name = "VoicePlayer"
compatibility_version = 20
```

#### 场景 3：微软官方正版登录（Microsoft OAuth）
```toml
[minecraft]
server_host = "mc.hypixel.net"
server_port = 25565
auth_type = "microsoft"
microsoft_email = "your_ms_email@outlook.com"
bot_name = "VoicePlayer"
compatibility_version = 20
```

---

### 2.3 配置文件全参数解析表

| 配置段落 | 键名 | 类型 | 默认值 | 详细含义与说明 |
| :--- | :--- | :--- | :--- | :--- |
| `[minecraft]` | `server_host` | String | `"127.0.0.1"` | Minecraft 服务器 IP 或域名 |
| | `server_port` | u16 | `25565` | Minecraft 服务器端口 |
| | `bot_name` | String | `"VoicePlayer"` | 机器人游戏内昵称 |
| | `auth_type` | String | `"offline"` | 认证方式：`"offline"`（离线）、`"yggdrasil"`（外置）、`"microsoft"`（正版） |
| | `microsoft_email` | String | `""` | 微软正版模式认证邮箱 |
| | `yggdrasil_server`| String | `""` | Yggdrasil 认证端点（如 `https://littleskin.cn/api/yggdrasil`） |
| | `yggdrasil_account`| String| `""` | 外置登录邮箱或用户名 |
| | `yggdrasil_password`| String| `""` | 外置登录密码（若填写 access_token 则无需填写） |
| | `yggdrasil_access_token`| String| `""`| 可选：外置登录持久化访问 Token |
| | `yggdrasil_profile_uuid`| String| `""`| 可选：外置登录角色 UUID |
| | `compatibility_version`| i32 | `20` | SVC 语音协议版本号（Simple Voice Chat 2.6.x 对应 20） |
| `[voice]` | `override_host` | String | `""` | 语音服务器 IP 覆盖（留空则自动从 SVC 握手包获取） |
| | `override_port` | u16 | `0` | 语音服务器端口覆盖（留空/0 则自动从握手包获取） |
| | `bitrate_kbps` | u32 | `96` | 推流码率（推荐 64 ~ 160 kbps） |
| | `replay_gain` | bool | `true` | 是否启用 ReplayGain 智能响度平衡与动态防爆音限幅 |
| | `target_lufs` | f32 | `-16.0` | 目标综合响度标准（LUFS，推荐 `-20.0` ~ `-14.0`） |
| `[voice.group]` | `enabled` | bool | `true` | 是否在进服时自动创建或加入指定 SVC 语音群组 |
| | `name` | String | `"Music"` | 默认目标群组名（群内全服无视距离与维度收听） |
| | `group_type` | String | `"open"` | 群组类型：`"open"`（公开开放）、`"normal"`（普通）、`"isolated"`（私密隔离） |
| | `password` | String | `""` | 群组密码（留空表示无密码公开群组） |
| `[media]` | `auto_download_tools`| bool | `true` | 未检测到 yt-dlp 时是否自动从 GitHub Release 下载 |
| | `bin_dir` | String | `"./bin"` | 外部辅助工具所在目录 |
| | `ffmpeg_path` | String | `""` | 自定义 FFmpeg 路径（留空优先找 `bin_dir` 再找 PATH） |
| | `ytdlp_path` | String | `""` | 自定义 yt-dlp 路径 |
| `[control]` | `command_prefix` | String | `"!"` | 游戏内公屏聊天指令前缀 |
| | `admin_players` | Array | `[]` | 管理员白名单玩家名（留空表示所有玩家均可使用播控指令） |
| | `auto_tpaccept` | bool | `true` | 收到玩家传送请求（CMI TPA）时是否自动防抖同意 |
| `[web]` | `enabled` | bool | `true` | 是否启用内嵌 Web 控制台 |
| | `bind_address` | String | `"127.0.0.1:8080"` | Web 控制台监听地址与端口 |

---

## 🛠️ 第三部分：开发者协议与架构

### 3.1 核心音频引擎与协议实现

1. **SVC UDP 协议与推流时钟**：
   - 监听 Minecraft 客户端信道 `voicechat:secret` 获取密钥与会话 ID；
   - 握手采用 AES-128-GCM 加解密并保持 20ms 时钟周期（每秒 50 帧，每帧 960 样本 @ 48kHz 单声道）；
   - 音频通过纯 Rust `rusty-opus` 动态变码率（VBR）编码推送到服务端。
2. **ReplayGain 响度引擎与软拐点限幅**：
   - **动态跟踪**：使用 EBU R128 一阶平滑能量滤波器平滑过渡，每帧最大增益步进 0.03 dB，避免音量骤变；
   - **静态扫描**：纯 Rust 解码或本地文件通过均方能量分析或标签解析锁定曲目全局增益；
   - **软拐点限幅**：当采样 $|x| \le 0.85$ 时保持线性，当 $|x| > 0.85$ 时使用 $\tanh$ 软饱和渐近限制在 $\pm 0.98$ 内，防止推流削波失真。

---

### 3.2 Web REST API 端点概要速查

内嵌 Web 服务提供极简 REST API，供外部程序或脚本集成：

#### 1. `GET /api/status`
获取播放器完整聚合状态（MC 连接、语音连接、群组名、当前曲目、时长、实时码率、实时 ReplayGain 增益、1-based 歌单项等）。

#### 2. `POST /api/play`
点播曲目或立即插播：
```json
{
  "url": "186016",
  "play_now": false
}
```

#### 3. `POST /api/control`
播控与歌单管理操作：
```json
{
  "action": "pause",
  "value": 80,
  "mode": "shuffle",
  "enabled": true,
  "index": 3
}
```

**`action` 动作参数对照**：
- 基础播控：`"pause"`, `"resume"`, `"stop"`, `"skip"`, `"prev"`
- 音量调节：`"volume"`（配合 `value: 0~100`）
- 播放模式：`"mode"`（配合 `mode: "loop" | "single" | "shuffle" | "seq"`）
- 响度平衡：`"replay_gain"`（配合 `enabled: true | false`）
- 歌单管理：`"playlist_jump"`（配合 `index`）、`"playlist_top"`（配合 `index`）、`"playlist_remove"`（配合 `index`）、`"playlist_clear"`

---

### 3.3 自动化测试与回归验证

执行项目全量测试套件：

```bash
cargo test
```

测试集覆盖 30 项完整单元与协议集成测试：
- **SVC 协议层**：VarInt 编解码、AES-128-GCM 加解密往返、群组数据包、音频麦克风数据包往返；
- **音频引擎层**：Opus 48kHz 正弦波编码、ReplayGain 标签解析、静态均方能量分析、软拐点限幅器边界（$\pm 100$ 输入不超限）；
- **歌单状态机**：列表循环回绕、单曲循环重播与手动前进、顺序播放停机、洗牌防首尾连击、前序删除与当前删除索引自愈；
- **指令与聊天解析**：聊天前缀匹配、发信人清洗、CMI TPA 提取及扩展播控指令解析。
