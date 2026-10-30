use azalea::prelude::*;
use azalea::Identifier;
use azalea_buf::UnsizedByteArray;
use azalea_protocol::packets::game::s_custom_payload::ServerboundCustomPayload;
use azalea_protocol::packets::game::ClientboundGamePacket;
use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};
use std::io::{Cursor, Read};
use std::str::FromStr;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::app::{AppManager, GroupAction};
use crate::audio::AudioPlayer;
use crate::mc::chat_handler::ChatHandler;
use crate::voice::protocol::{
    create_group_payload, join_group_payload, parse_joined_group_packet,
    parse_remove_group_packet, read_varint, SvcGroup,
};
use crate::voice::VoiceClient;

#[derive(Clone, Component, Default)]
pub struct BotState {
    pub app: Option<Arc<AppManager>>,
}

pub struct SessionManager;

impl SessionManager {
    pub async fn run_bot(app: Arc<AppManager>) -> anyhow::Result<()> {
        let host = app.config.minecraft.server_host.clone();
        let port = app.config.minecraft.server_port;
        let bot_name = app.config.minecraft.bot_name.clone();
        let is_online = app.config.minecraft.online_mode;
        let email = app.config.minecraft.microsoft_email.clone();

        let auth_type = app.config.minecraft.auth_type.to_lowercase();
        let account: Account = match auth_type.as_str() {
            "yggdrasil" | "authlib" | "authlib-injector" => {
                let server_url = &app.config.minecraft.yggdrasil_server;
                let username = &app.config.minecraft.yggdrasil_account;
                let password = &app.config.minecraft.yggdrasil_password;
                let token = &app.config.minecraft.yggdrasil_access_token;
                let profile_uuid_str = &app.config.minecraft.yggdrasil_profile_uuid;
                let char_name = if !app.config.minecraft.yggdrasil_character_name.is_empty() {
                    Some(app.config.minecraft.yggdrasil_character_name.as_str())
                } else {
                    None
                };

                if !token.is_empty() && !profile_uuid_str.is_empty() {
                    info!("[MC] 正在使用预设 Yggdrasil Token 登录: {}", username);
                    let uuid = Uuid::parse_str(profile_uuid_str)?;
                    let ygg = crate::mc::yggdrasil::YggdrasilAccount::with_token(
                        server_url,
                        if let Some(cn) = char_name { cn } else { username },
                        uuid,
                        token,
                    );
                    Account::from(ygg)
                } else {
                    info!(
                        "[MC] 正在使用第三方 Yggdrasil 服务器认证: {} (用户: {})",
                        server_url, username
                    );
                    let ygg = crate::mc::yggdrasil::YggdrasilAccount::authenticate(
                        server_url,
                        username,
                        password,
                        char_name,
                    )
                    .await?;
                    Account::from(ygg)
                }
            }
            "microsoft" => {
                info!("[MC] 正在使用微软官方正版账号登录: {}", email);
                Account::microsoft(&email).await?
            }
            _ => {
                if is_online {
                    info!("[MC] 正在使用微软官方正版账号登录: {}", email);
                    Account::microsoft(&email).await?
                } else {
                    info!("[MC] 正在使用离线模式登录: {}", bot_name);
                    Account::offline(&bot_name)
                }
            }
        };

        let server_address = format!("{}:{}", host, port);
        info!("[MC] 正在连接 Minecraft 服务器: {}", server_address);

        let state = BotState {
            app: Some(Arc::clone(&app)),
        };

        let _ = ClientBuilder::new()
            .set_handler(handle_bot_event)
            .set_state(state)
            .start(account, server_address.as_str())
            .await;

        Ok(())
    }
}

async fn handle_bot_event(
    bot: Client,
    event: Event,
    state: BotState,
) -> anyhow::Result<()> {
    let app = match &state.app {
        Some(a) => Arc::clone(a),
        None => return Ok(()),
    };

    match event {
        Event::Init => {
            info!("[MC] 假人客户端上下文已创建，正在配置内部消息发信队列...");

            // 创建通道将应用层的聊天发信绑定到 bot
            let (chat_tx, mut chat_rx) = mpsc::channel::<String>(32);
            app.set_chat_sender(chat_tx).await;

            let bot_clone = bot.clone();
            tokio::spawn(async move {
                while let Some(msg) = chat_rx.recv().await {
                    bot_clone.chat(&msg);
                }
            });

            // 创建通道将应用层的 SVC 群组操作指令绑定到 bot
            let (group_tx, mut group_rx) = mpsc::channel::<GroupAction>(16);
            app.set_group_action_sender(group_tx).await;

            let bot_clone_grp = bot.clone();
            let app_clone_grp = Arc::clone(&app);
            tokio::spawn(async move {
                while let Some(action) = group_rx.recv().await {
                    match action {
                        GroupAction::JoinOrCreate { name, password, group_type } => {
                            if let Some(existing) = app_clone_grp.find_group_by_name(&name).await {
                                info!(
                                    "[MC-Group] 发现已存在同名群组 \"{}\" (UUID: {})，直接加入...",
                                    existing.name, existing.id
                                );
                                send_join_group(&bot_clone_grp, existing.id, password.as_deref());
                            } else {
                                info!(
                                    "[MC-Group] 未检测到名为 \"{}\" 的群组，正在创建并加入公开群组...",
                                    name
                                );
                                send_create_group(&bot_clone_grp, &name, password.as_deref(), group_type);
                            }
                        }
                        GroupAction::JoinById { id, password } => {
                            info!("[MC-Group] 正在加入群组 (UUID: {})...", id);
                            send_join_group(&bot_clone_grp, id, password.as_deref());
                        }
                        GroupAction::Leave => {
                            info!("[MC-Group] 正在离开当前群组...");
                            send_leave_group(&bot_clone_grp);
                        }
                    }
                }
            });
        }
        Event::Spawn => {
            info!("[MC] 假人已成功在世界中生成（进入 Play 阶段）！正在向服务端发送 SVC 插件通道握手...");
            app.mc_connected.store(true, Ordering::SeqCst);

            let compat_version = app.config.minecraft.compatibility_version;
            send_voicechat_handshake(&bot, compat_version);

            // 启动定时重试守护任务（若未连接语音则每隔 3 秒重试握手，最多 8 次）
            let app_clone = Arc::clone(&app);
            let bot_clone = bot.clone();
            tokio::spawn(async move {
                for attempt in 1..=8 {
                    tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;
                    if app_clone.secret_received.load(Ordering::SeqCst)
                        || app_clone.voice_connected.load(Ordering::SeqCst)
                    {
                        break;
                    }
                    info!("[MC] 尚未收到 voicechat:secret 响应，进行第 {} 次握手重试...", attempt);
                    send_voicechat_handshake(&bot_clone, compat_version);
                }
            });
        }
        Event::Packet(packet) => {
            if let ClientboundGamePacket::CustomPayload(cp) = packet.as_ref() {
                let channel = cp.identifier.to_string();
                if channel.starts_with("voicechat:") {
                    info!("[MC] 收到 SVC 插件通道消息: channel={}, 数据大小={} 字节", channel, cp.data.len());
                }
                if channel == "voicechat:secret" {
                    app.secret_received.store(true, Ordering::SeqCst);
                    info!("[MC] 成功捕获服务端下发的 voicechat:secret 插件消息！");
                    if let Ok(secret_data) = parse_secret_packet(&cp.data) {
                        info!(
                            "[MC] 解析 SVC 协商凭证成功: port={}, uuid={}, host={}",
                            secret_data.server_port, secret_data.player_uuid, secret_data.voice_host
                        );

                        // 自适应解析 UDP 真实连接地址（防止服务端下发带端口导致双重端口解析错误）
                        let voice_addr_str = resolve_voice_address(
                            &app.config.voice.override_host,
                            app.config.voice.override_port,
                            &secret_data.voice_host,
                            secret_data.server_port,
                            &app.config.minecraft.server_host,
                        );
                        info!("[MC] 计算语音服务目标地址为: {}", voice_addr_str);

                        match tokio::net::lookup_host(&voice_addr_str).await {
                            Ok(mut addrs) => {
                                if let Some(addr) = addrs.next() {
                                    let app_clone = Arc::clone(&app);
                                    tokio::spawn(async move {
                                        match VoiceClient::connect(addr, secret_data.player_uuid, secret_data.secret).await {
                                            Ok(voice_client) => {
                                                let player = Arc::new(AudioPlayer::new(
                                                    Arc::clone(&voice_client),
                                                    app_clone.tools.ffmpeg.clone(),
                                                    app_clone.config.voice.bitrate_kbps,
                                                    Arc::clone(&app_clone.chat_sender),
                                                ));
                                                app_clone.bind_voice_player(voice_client, player).await;
                                                info!("[MC] 语音伴播服务整体初始化成功！已成功连接 SVC 语音通道。");
                                            }
                                            Err(e) => {
                                                error!("[MC] 连接 SVC 语音服务失败: {:?}", e);
                                            }
                                        }
                                    });
                                } else {
                                    error!("[MC] 无法解析语音服务地址: {}", voice_addr_str);
                                }
                            }
                            Err(e) => {
                                error!("[MC] DNS 解析语音服务失败: {:?}", e);
                            }
                        }
                    }

                    // 启动自动加入或创建群组任务
                    if app.config.voice.group.enabled {
                        let app_grp = Arc::clone(&app);
                        tokio::spawn(async move {
                            // 等待 800ms 让服务端将全部已存在的 add_group 包下发完成
                            tokio::time::sleep(tokio::time::Duration::from_millis(800)).await;
                            let target_name = app_grp.config.voice.group.name.clone();
                            let password = if app_grp.config.voice.group.password.is_empty() {
                                None
                            } else {
                                Some(app_grp.config.voice.group.password.clone())
                            };
                            let group_type = match app_grp.config.voice.group.group_type.to_lowercase().as_str() {
                                "isolated" => 2,
                                "open" => 1,
                                _ => 0,
                            };

                            app_grp.send_group_action(GroupAction::JoinOrCreate {
                                name: target_name,
                                password,
                                group_type,
                            }).await;
                        });
                    }
                } else if channel == "voicechat:add_group" {
                    if let Ok(group) = SvcGroup::from_bytes(&cp.data) {
                        info!(
                            "[MC-Group] 收到服务端群组通知: \"{}\" (UUID: {}, 类型: {})",
                            group.name, group.id, group.group_type
                        );
                        app.update_known_group(group).await;
                    }
                } else if channel == "voicechat:remove_group" {
                    if let Ok(id) = parse_remove_group_packet(&cp.data) {
                        info!("[MC-Group] 服务端通知群组已解散或移除 (UUID: {})", id);
                        app.remove_known_group(id).await;
                    }
                } else if channel == "voicechat:joined_group" {
                    if let Ok((group_id, wrong_pwd)) = parse_joined_group_packet(&cp.data) {
                        if wrong_pwd {
                            warn!("[MC-Group] 加入群组失败: 密码错误！");
                        } else if let Some(id) = group_id {
                            let group_name = {
                                let known = app.known_groups.read().await;
                                known.get(&id).map(|g| g.name.clone()).unwrap_or_else(|| "未知群组".to_string())
                            };
                            info!(
                                "[MC-Group] 假人已成功加入 Simple Voice Chat 群组: \"{}\" (UUID: {})！群成员可全服无视距离收听音乐。",
                                group_name, id
                            );
                            let current = {
                                let known = app.known_groups.read().await;
                                known.get(&id).cloned()
                            };
                            app.set_current_group(current).await;
                        } else {
                            info!("[MC-Group] 假人已退出当前语音群组");
                            app.set_current_group(None).await;
                        }
                    }
                }
            }
        }
        Event::Chat(chat_pkt) => {
            ChatHandler::handle_chat_packet(app, &chat_pkt).await;
        }
        Event::Disconnect(_) => {
            warn!("[MC] 假人已与 Minecraft 服务器断开连接");
            app.mc_connected.store(false, Ordering::SeqCst);
            app.voice_connected.store(false, Ordering::SeqCst);
        }
        _ => {}
    }

    Ok(())
}

/// 向 Minecraft 服务端发送 Simple Voice Chat 插件通道注册及握手请求
fn send_voicechat_handshake(bot: &Client, compat_version: i32) {
    // 1. 发送通道注册包 minecraft:register
    if let Ok(ident) = Identifier::from_str("minecraft:register") {
        let payload = ServerboundCustomPayload {
            identifier: ident,
            data: UnsizedByteArray::from(
                b"voicechat:secret\0voicechat:request_secret\0voicechat:player_state\0voicechat:player_states\0voicechat:add_group\0voicechat:remove_group\0voicechat:joined_group\0voicechat:create_group\0voicechat:set_group\0voicechat:leave_group".to_vec(),
            ),
        };
        bot.write_packet(payload);
    }

    // 2. 发送 voicechat:request_secret (携带 compatibilityVersion)
    let mut req_data = Vec::new();
    let _ = req_data.write_i32::<BigEndian>(compat_version);

    if let Ok(ident) = Identifier::from_str("voicechat:request_secret") {
        let payload = ServerboundCustomPayload {
            identifier: ident,
            data: UnsizedByteArray::from(req_data),
        };
        bot.write_packet(payload);
        info!("[MC] 已发送 voicechat:request_secret (版本号: {})", compat_version);
    }
}

struct ParsedSecret {
    secret: [u8; 16],
    server_port: u16,
    player_uuid: Uuid,
    voice_host: String,
}

fn parse_secret_packet(data: &[u8]) -> anyhow::Result<ParsedSecret> {
    let mut cursor = Cursor::new(data);

    let mut secret = [0u8; 16];
    cursor.read_exact(&mut secret)?;

    let server_port = cursor.read_i32::<BigEndian>()? as u16;

    let mut uuid_bytes = [0u8; 16];
    cursor.read_exact(&mut uuid_bytes)?;
    let player_uuid = Uuid::from_bytes(uuid_bytes);

    let _codec = cursor.read_u8()?;
    let _mtu_size = cursor.read_i32::<BigEndian>()?;
    let _voice_distance = cursor.read_f64::<BigEndian>()?;
    let _keep_alive = cursor.read_i32::<BigEndian>()?;
    let _groups_enabled = cursor.read_u8()? != 0;

    let host_len = read_varint(&mut cursor)? as usize;
    let mut host_bytes = vec![0u8; host_len];
    cursor.read_exact(&mut host_bytes)?;
    let voice_host = String::from_utf8(host_bytes).unwrap_or_default();

    Ok(ParsedSecret {
        secret,
        server_port,
        player_uuid,
        voice_host,
    })
}

/// 自适应解析并格式化 UDP 语音服务目标地址
/// 避免服务端下发带端口的域名导致二次拼接出类似 "mc.vlssu.cn:45114:45114" 的错误
pub fn resolve_voice_address(
    override_host: &str,
    override_port: u16,
    secret_host: &str,
    secret_port: u16,
    default_mc_host: &str,
) -> String {
    let raw_host = if !override_host.trim().is_empty() {
        override_host.trim()
    } else if !secret_host.trim().is_empty() {
        secret_host.trim()
    } else {
        default_mc_host.trim()
    };

    // 1. 如果 raw_host 本身就能成功解析为 SocketAddr（例如 "127.0.0.1:24454" 或 "[::1]:24454"）
    if let Ok(socket_addr) = raw_host.parse::<std::net::SocketAddr>() {
        let final_port = if override_port != 0 {
            override_port
        } else {
            socket_addr.port()
        };
        return format!("{}:{}", socket_addr.ip(), final_port);
    }

    // 2. 如果包含冒号，且不是裸 IPv6，尝试剥离已有的端口（例如 "mc.vlssu.cn:45114"）
    if let Some((host_part, port_part)) = raw_host.rsplit_once(':') {
        if !host_part.contains(':') {
            if let Ok(parsed_port) = port_part.parse::<u16>() {
                let final_port = if override_port != 0 {
                    override_port
                } else {
                    parsed_port
                };
                return format!("{}:{}", host_part, final_port);
            }
        }
    }

    // 3. 纯域名或未带端口的主机名，附加端口
    let final_port = if override_port != 0 {
        override_port
    } else {
        secret_port
    };

    format!("{}:{}", raw_host, final_port)
}

/// 发送 voicechat:create_group 创建并加入新群组
pub fn send_create_group(bot: &Client, name: &str, password: Option<&str>, group_type: i16) {
    if let Ok(ident) = Identifier::from_str("voicechat:create_group") {
        if let Ok(data) = create_group_payload(name, password, group_type) {
            let payload = ServerboundCustomPayload {
                identifier: ident,
                data: UnsizedByteArray::from(data),
            };
            bot.write_packet(payload);
            info!(
                "[MC-Group] 已向服务端发送 voicechat:create_group (name: \"{}\", type: {})",
                name, group_type
            );
        }
    }
}

/// 发送 voicechat:set_group 加入已存在的群组
pub fn send_join_group(bot: &Client, group_id: Uuid, password: Option<&str>) {
    if let Ok(ident) = Identifier::from_str("voicechat:set_group") {
        if let Ok(data) = join_group_payload(group_id, password) {
            let payload = ServerboundCustomPayload {
                identifier: ident,
                data: UnsizedByteArray::from(data),
            };
            bot.write_packet(payload);
            info!("[MC-Group] 已向服务端发送 voicechat:set_group (group_id: {})", group_id);
        }
    }
}

/// 发送 voicechat:leave_group 离开群组
pub fn send_leave_group(bot: &Client) {
    if let Ok(ident) = Identifier::from_str("voicechat:leave_group") {
        let payload = ServerboundCustomPayload {
            identifier: ident,
            data: UnsizedByteArray::from(Vec::new()),
        };
        bot.write_packet(payload);
        info!("[MC-Group] 已向服务端发送 voicechat:leave_group");
    }
}

