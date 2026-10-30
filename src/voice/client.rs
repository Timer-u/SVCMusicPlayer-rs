use std::io::Cursor;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::UdpSocket;
use tokio::time::sleep;
use tracing::{debug, error, info, trace};
use uuid::Uuid;

use crate::voice::crypto::VoiceCrypto;
use crate::voice::protocol::*;

pub struct VoiceClient {
    socket: Arc<UdpSocket>,
    crypto: Arc<VoiceCrypto>,
    pub player_uuid: Uuid,
    sequence: AtomicI64,
    authenticated: Arc<AtomicBool>,
    connected: Arc<AtomicBool>,
    running: Arc<AtomicBool>,
}

impl VoiceClient {
    pub async fn connect(
        server_addr: SocketAddr,
        player_uuid: Uuid,
        secret_key: [u8; 16],
    ) -> anyhow::Result<Arc<Self>> {
        info!("[VoiceClient] 正在连接 SVC 语音服务 UDP 端口: {}", server_addr);
        let socket = UdpSocket::bind("0.0.0.0:0").await?;
        socket.connect(server_addr).await?;

        let crypto = Arc::new(VoiceCrypto::new(&secret_key));
        let client = Arc::new(Self {
            socket: Arc::new(socket),
            crypto,
            player_uuid,
            sequence: AtomicI64::new(0),
            authenticated: Arc::new(AtomicBool::new(false)),
            connected: Arc::new(AtomicBool::new(false)),
            running: Arc::new(AtomicBool::new(true)),
        });

        // 启动后台网络监听循环
        let client_clone = Arc::clone(&client);
        tokio::spawn(async move {
            client_clone.listen_loop().await;
        });

        // 执行握手流程
        client.perform_handshake(&secret_key).await?;

        Ok(client)
    }

    /// 发送已包装的 SVC 数据包 (外层封包结构: [MAGIC_BYTE] + [UUID(16B)] + [VarInt(len)] + [EncryptedPayload])
    pub async fn send_packet(&self, packet_type: u8, payload: &[u8]) -> anyhow::Result<()> {
        let encrypted = self.crypto.encrypt(packet_type, payload)
            .map_err(|e| anyhow::anyhow!("AES-GCM encrypt error: {:?}", e))?;

        let mut buf = Vec::with_capacity(1 + 16 + 5 + encrypted.len());
        buf.push(MAGIC_BYTE);
        buf.extend_from_slice(self.player_uuid.as_bytes());
        write_varint(encrypted.len() as i32, &mut buf)?;
        buf.extend_from_slice(&encrypted);

        self.socket.send(&buf).await?;
        Ok(())
    }

    /// 发送麦克风音频帧 (MicPacket 0x01)
    pub async fn send_mic_frame(&self, opus_data: &[u8], whispering: bool) -> anyhow::Result<()> {
        let seq = self.sequence.fetch_add(1, Ordering::SeqCst);
        let packet = MicPacket {
            opus_data: opus_data.to_vec(),
            sequence_number: seq,
            whispering,
        };
        let body = packet.to_bytes()?;
        self.send_packet(PKT_MIC, &body).await
    }

    /// 执行双阶段握手：AuthenticatePacket (0x05) -> ConnectionCheckPacket (0x09)
    async fn perform_handshake(&self, secret_key: &[u8; 16]) -> anyhow::Result<()> {
        let auth_pkt = AuthenticatePacket {
            player_uuid: self.player_uuid,
            secret: *secret_key,
        };
        let auth_body = auth_pkt.to_bytes()?;

        info!("[VoiceClient] 开始握手第一阶段: 发送 AuthenticatePacket (0x05)");
        let mut attempts = 0;
        while !self.authenticated.load(Ordering::SeqCst) && attempts < 10 {
            attempts += 1;
            info!("[VoiceClient] 发送 AuthenticatePacket (第 {}/10 次尝试)...", attempts);
            self.send_packet(PKT_AUTHENTICATE, &auth_body).await?;
            sleep(Duration::from_millis(1000)).await;
        }

        if !self.authenticated.load(Ordering::SeqCst) {
            anyhow::bail!("VoiceChat 握手失败: 未收到服务端的 AuthenticateAck (0x06)，请检查 UDP 端口是否可达及 Secret 是否正确！");
        }
        info!("[VoiceClient] 收到 AuthenticateAck，开始握手第二阶段: 发送 ConnectionCheckPacket (0x09)");

        attempts = 0;
        while !self.connected.load(Ordering::SeqCst) && attempts < 10 {
            attempts += 1;
            info!("[VoiceClient] 发送 ConnectionCheckPacket (第 {}/10 次尝试)...", attempts);
            self.send_packet(PKT_CONNECTION_CHECK, &[]).await?;
            sleep(Duration::from_millis(1000)).await;
        }

        if !self.connected.load(Ordering::SeqCst) {
            anyhow::bail!("VoiceChat 连接校验超时: 未收到 ConnectionCheckAck (0x0A)");
        }

        info!("[VoiceClient] VoiceChat UDP 语音信道双向握手成功，推流已就绪！");
        Ok(())
    }

    /// 后台接收监听与保活循环
    async fn listen_loop(&self) {
        let mut buffer = [0u8; 4096];
        while self.running.load(Ordering::SeqCst) {
            match self.socket.recv(&mut buffer).await {
                Ok(len) => {
                    if len < 2 {
                        continue;
                    }
                    if buffer[0] != MAGIC_BYTE {
                        continue;
                    }

                    // 官方协议规范：服务端下发的 UDP 包不包含玩家 UUID，其格式为：
                    // [MAGIC_BYTE (1B: 0xFF)] + [VarInt(len)] + [AES-GCM EncryptedPayload (IV 12B + Ciphertext + Tag 16B)]
                    let mut decrypted_opt = None;

                    // 1. 标准规范：无 UUID（偏移 1 开始读取 VarInt 长度）
                    let mut cursor = Cursor::new(&buffer[1..len]);
                    if let Ok(payload_len) = read_varint(&mut cursor) {
                        let varint_size = cursor.position() as usize;
                        let payload_start = 1 + varint_size;
                        let payload_end = payload_start + (payload_len as usize);
                        if payload_end <= len {
                            let encrypted_slice = &buffer[payload_start..payload_end];
                            if let Ok((pkt_type, body)) = self.crypto.decrypt(encrypted_slice) {
                                decrypted_opt = Some((pkt_type, body));
                            }
                        }
                    }

                    // 2. 备用容错：如果某些特殊代理/中继携带了 16 字节 UUID（偏移 17 开始读取）
                    if decrypted_opt.is_none() && len >= 1 + 16 + 1 {
                        let mut cursor = Cursor::new(&buffer[17..len]);
                        if let Ok(payload_len) = read_varint(&mut cursor) {
                            let varint_size = cursor.position() as usize;
                            let payload_start = 17 + varint_size;
                            let payload_end = payload_start + (payload_len as usize);
                            if payload_end <= len {
                                let encrypted_slice = &buffer[payload_start..payload_end];
                                if let Ok((pkt_type, body)) = self.crypto.decrypt(encrypted_slice) {
                                    decrypted_opt = Some((pkt_type, body));
                                }
                            }
                        }
                    }

                    if let Some((pkt_type, body)) = decrypted_opt {
                        self.handle_packet(pkt_type, &body).await;
                    }
                }
                Err(e) => {
                    if self.running.load(Ordering::SeqCst) {
                        error!("[VoiceClient] UDP 接收循环出错: {:?}", e);
                    }
                    break;
                }
            }
        }
    }

    /// 处理服务端发来的数据包
    async fn handle_packet(&self, pkt_type: u8, data: &[u8]) {
        match pkt_type {
            PKT_AUTHENTICATE_ACK => {
                info!("[VoiceClient] 收到服务端 AuthenticateAck (0x06)，身份认证成功！");
                self.authenticated.store(true, Ordering::SeqCst);
            }
            PKT_CONNECTION_CHECK_ACK => {
                info!("[VoiceClient] 收到服务端 ConnectionCheckAck (0x0A)，连接验证成功！");
                self.connected.store(true, Ordering::SeqCst);
            }
            PKT_KEEP_ALIVE => {
                debug!("[VoiceClient] 收到服务端心跳 KeepAlive (0x08)，自动回包响应");
                let _ = self.send_packet(PKT_KEEP_ALIVE, &[]).await;
            }
            PKT_PING => {
                if let Ok(ping) = PingPacket::from_bytes(data) {
                    debug!("[VoiceClient] 收到服务端 PingPacket (0x07)，回复 Pong: {}", ping.id);
                    let _ = self.send_packet(PKT_PING, data).await;
                }
            }
            _ => {
                trace!("[VoiceClient] 收到其它类型数据包: 0x{:02X}, 大小: {}", pkt_type, data.len());
            }
        }
    }

    #[allow(dead_code)]
    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }

    pub fn close(&self) {
        self.running.store(false, Ordering::SeqCst);
    }
}
