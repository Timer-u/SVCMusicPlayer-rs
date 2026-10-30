# SVCMusicPlayer-rs

基于 Rust 与 [Azalea](https://github.com/azalea-rs/azalea) 实现的 Minecraft [Simple Voice Chat (SVC)](https://github.com/henkelmax/simple-voice-chat) 独立音乐播放机器人。

单个二进制文件即可独立运行，无需配置复杂的运行时环境。支持网易云/B站/N站/直链点歌、EBU R128 响度自动均衡与软限幅防爆音、无缝连播与多种播放模式，并内置 Web 控制台。

## 特性亮点

- **轻量独立**：Rust 编写，编译为单个二进制文件，常驻内存仅约 20MB ~ 30MB。
- **完整 SVC 协议支持**：支持 AES-128-GCM 加密握手与 20ms 恒定时钟推流（Opus 48kHz 单声道），原生兼容 SVC 语音群组（全服无视距离收听）。
- **响度均衡与防爆音**：结合 EBU R128 动态响度追踪与静态能量扫描，搭配 tanh 软拐点峰值限幅，避免不同音源音量忽大忽小或大音量削波失真。
- **持久化歌单与多种播放模式**：本地 JSON 持久化，支持列表循环、单曲循环、随机播放（跨轮不重复）与顺序播放；单曲播完自动切歌，遇到损坏曲目自动跳过。
- **内置 Web 控制台**：单页应用在编译期直接打包进二进制文件，提供动态声波指示、平滑进度条、播放控制、音量调节与歌单置顶/删除操作。
- **多平台与格式解析**：集成纯 Rust `symphonia` 解码器与 `yt-dlp` / `ffmpeg`，支持网易云歌曲 ID/链接、B站 BV 号、Niconico、网络音频直链（MP3/WAV/FLAC/OGG/AAC）以及本地音频文件。
- **全登录模式兼容**：支持离线模式（Offline）、第三方外置登录（LittleSkin / Blessing Skin 等 Yggdrasil 服务）以及微软官方正版账号（OAuth）。

## 快速起步

### 1. 编译或下载

#### 从源码编译
确保已安装 Rust 工具链（1.85+ / 2024 edition）：

```bash
# 编译 Release 高性能版本
cargo build --release

# 编译产物位于 target/release/
# Windows: .\target\release\svc_music_player.exe
# Linux:   ./target/release/svc_music_player
```

### 2. 编写配置并启动

在程序同目录下创建 `config.toml`（或运行一次自动生成）：

```toml
[minecraft]
server_host = "127.0.0.1"
server_port = 25565
bot_name = "VoicePlayer"
auth_type = "offline"         # 登录模式: "offline" | "yggdrasil" | "microsoft"
compatibility_version = 20    # SVC 协议版本 (SVC 2.6.x 对应 20)

[voice]
bitrate_kbps = 96             # Opus 推流码率 (推荐 64 ~ 128)
replay_gain = true            # 开启响度自动均衡与防爆音
target_lufs = -16.0           # 目标综合响度标准 (LUFS，推荐 -20.0 ~ -14.0)

[voice.group]
enabled = true                # 进服自动创建或加入指定群组
name = "Music"                # 群名 (群成员可全服无视距离收听)
group_type = "open"           # "open" (附近非群员也能听到) | "normal" | "isolated"

[control]
command_prefix = "!"          # 聊天指令前缀
admin_players = []            # 管理员白名单（留空代表所有人均可控制）

[web]
enabled = true                # 启用内置 Web 控制台
bind_address = "127.0.0.1:8080"
```

启动程序后，机器人将自动连接 Minecraft 服务器并完成 SVC 语音握手。若启用了 Web 控制台，浏览器访问 `http://127.0.0.1:8080` 即可查看状态与控制播放。

> 完整参数选项及微软/外置登录配置示例，请参考同目录下的 [config.example.toml](config.example.toml)。

## 游戏内指令

在游戏公屏直接发送聊天即可操作机器人（默认前缀为 `!`）：

### 点歌与输入格式

点歌支持歌曲 ID、网址或本地相对路径：

| 平台 / 类型 | 输入示例 | 说明 |
| :--- | :--- | :--- |
| **网易云音乐** | `!play 186016`<br>`!play wy:186016`<br>`!play https://music.163.com/#/song?id=186016` | 提取歌曲元数据并通过直链推流 |
| **Bilibili (B站)** | `!play BV1xx411c7mD`<br>`!play https://www.bilibili.com/video/BV1xx411c7mD` | 自动调用 yt-dlp 解析高品质音轨 |
| **Niconico (N站)** | `!play sm46865356`<br>`!play https://www.nicovideo.jp/watch/sm46865356` | 自动提取 sm/so/nm 编号并解析 |
| **网络直链** | `!play https://example.com/audio/track.mp3` | 支持 MP3, WAV, FLAC, OGG, AAC, M4A |
| **本地音频文件** | `!play favorite.flac`<br>`!play ./music/track.wav` | 自动在项目根目录及 `music/` 目录中匹配 |

### 指令速查表

> 权限说明：若 `config.toml` 中配置了 `admin_players`，标记为 **需管理员** 的指令仅白名单玩家可执行；若未配置管理员，则所有玩家均可使用。

| 指令 | 别名 | 参数 | 说明 | 权限 |
| :--- | :--- | :--- | :--- | :--- |
| `!play` | - | `<链接/ID/路径>` | 将曲目追加至歌单末尾 | 全员 |
| `!playnow` | `!pn` | `<链接/ID/路径>` | 插播并立即播放 | 需管理员 |
| `!pause` | - | 无 | 暂停播放 | 需管理员 |
| `!resume` | - | 无 | 恢复播放 | 需管理员 |
| `!stop` | - | 无 | 停止当前播放（保留歌单） | 需管理员 |
| `!skip` | `!next` | 无 | 切至下一首 | 需管理员 |
| `!prev` | - | 无 | 切回上一首 | 需管理员 |
| `!mode` | - | `[loop\|single\|shuffle\|seq]` | 查看当前模式；加参数可直接切换模式 | 查看全员 / 切换需管理员 |
| `!loop` | - | 无 | 快捷切换为「列表循环」 | 需管理员 |
| `!single` | - | 无 | 快捷切换为「单曲循环」 | 需管理员 |
| `!shuffle` | `!random` | 无 | 快捷切换为「随机播放」 | 需管理员 |
| `!seq` | - | 无 | 快捷切换为「顺序播放」 | 需管理员 |
| `!list` | `!pl`, `!queue` | `[页码]` | 查看当前歌单（每页 6 首，标注当前播放曲） | 全员 |
| `!jump` | `!skipto` | `<序号>` | 跳转播放指定序号歌曲 | 需管理员 |
| `!top` | `!pin` | `<序号>` | 将指定序号歌曲置顶为下一首 | 需管理员 |
| `!rm` | `!del`, `!remove` | `<序号>` | 从歌单中移除指定歌曲 | 需管理员 |
| `!clear` | - | 无 | 清空全部歌单并停止播放 | 需管理员 |
| `!vol` | `!volume` | `[0-100]` | 查看当前音量；加参数调整音量 | 查看全员 / 调整需管理员 |
| `!rg` | `!loudnorm` | `[on\|off]` | 查看或开关 ReplayGain 响度均衡 | 查看全员 / 开关需管理员 |
| `!group` | - | `[join <群名> [密码] \| leave]` | 查看当前语音群组，或加入/离开群组 | 查看全员 / 操作需管理员 |
| `!tpaccept` | `!tpa` | 无 | 执行 `/cmi tpaccept` 同意玩家传送请求 | 需管理员 |
| `!help` | - | 无 | 显示可用指令简要列表 | 全员 |

## Web 控制台

当配置中 `web.enabled = true` 时，浏览器访问 `http://127.0.0.1:8080`（或自定义绑定地址）：

- **状态展示**：顶栏显示 MC 连线、SVC 语音握手状态及当前所在语音群组。
- **实时播控**：动态展示当前曲目名称、来源路径、推流码率、实时 ReplayGain 增益及平滑进度条；支持一键暂停/恢复、上一首/下一首、播放模式切换、音量滑块及响度平衡开关。
- **在线点歌与歌单管理**：支持输入链接/ID加入队列或立即插播，歌单每行支持一键置顶、跳转播放或删除。

## 进阶与开发

### REST API

内置 Web 服务提供 RESTful API，方便与外部机器人（如 QQ 机器人/NoneBot）、脚本或自动化工作流集成：

#### 1. 获取播放器状态
- **端点**：`GET /api/status`
- **返回**：包含连接状态、群组信息、当前播放曲目详情、推流码率、实时增益、音量、播放模式以及完整歌单列表。

#### 2. 点播曲目
- **端点**：`POST /api/play`
- **载荷**：
  ```json
  {
    "url": "https://music.163.com/#/song?id=186016",
    "play_now": false
  }
  ```
  `play_now` 为 `true` 时为立即插播，`false` 为追加至队列末尾。

#### 3. 播控与歌单操作
- **端点**：`POST /api/control`
- **载荷说明**：通过 `action` 字段指定动作：
  ```json
  {
    "action": "pause"
  }
  ```
  支持的动作对照：
  - 基础播控：`"pause"`, `"resume"`, `"stop"`, `"skip"`, `"prev"`
  - 音量调节：`"volume"`（配合 `"value": 0~100`）
  - 播放模式：`"mode"`（配合 `"mode": "loop" | "single" | "shuffle" | "seq"`）
  - 响度平衡：`"replay_gain"`（配合 `"enabled": true | false`）
  - 歌单管理：`"playlist_jump"`, `"playlist_top"`, `"playlist_remove"`（配合 1-based `"index"`）；`"playlist_clear"`

### 音频引擎与协议实现

1. **SVC 协议握手与推流**：
   - 监听客户端 `voicechat:secret` 插件信道获取密钥与会话 ID；
   - 采用 AES-128-GCM 加密握手并维持 UDP 心跳包；
   - 严格维持 20ms 推流周期（每秒 50 帧，单声道 48kHz 下每帧 960 个采样），使用 `rusty-opus` 变码率（VBR）编码推流。
2. **响度均衡与防爆音**：
   - **动态跟踪**：使用 EBU R128 一阶平滑能量滤波，帧间最大调整步进限制为 0.03 dB，消除音量突变感；
   - **软拐点限幅**：当输入采样绝对值 $|x| \le 0.85$ 时保持线性响应；当 $|x| > 0.85$ 时平滑接入 $\tanh$ 软饱和曲线渐近限制在 $\pm 0.98$ 内，防止推流削波失真。

### 自动化测试

运行项目测试套件：

```bash
cargo test
```

测试集涵盖 VarInt 编解码、AES-128-GCM 加解密往返、音频麦克风数据包封包、Opus 编码、ReplayGain 能量分析与限幅边界验证、歌单状态机边界（循环/随机/删除恢复）及聊天指令解析。
