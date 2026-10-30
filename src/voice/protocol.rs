#![allow(dead_code)]

use std::io::{self, Cursor, Read, Write};
use byteorder::{BigEndian, ReadBytesExt, WriteBytesExt};
use uuid::Uuid;

pub const MAGIC_BYTE: u8 = 0xFF;

// Packet Type IDs defined by Simple Voice Chat protocol
pub const PKT_MIC: u8 = 0x01;
pub const PKT_PLAYER_SOUND: u8 = 0x02;
pub const PKT_GROUP_SOUND: u8 = 0x03;
pub const PKT_LOCATION_SOUND: u8 = 0x04;
pub const PKT_AUTHENTICATE: u8 = 0x05;
pub const PKT_AUTHENTICATE_ACK: u8 = 0x06;
pub const PKT_PING: u8 = 0x07;
pub const PKT_KEEP_ALIVE: u8 = 0x08;
pub const PKT_CONNECTION_CHECK: u8 = 0x09;
pub const PKT_CONNECTION_CHECK_ACK: u8 = 0x0A;

/// 写入 Minecraft 标准 VarInt
pub fn write_varint<W: Write>(val: i32, writer: &mut W) -> io::Result<()> {
    let mut uval = val as u32;
    loop {
        let temp = (uval & 0x7F) as u8;
        uval >>= 7;
        if uval != 0 {
            writer.write_all(&[temp | 0x80])?;
        } else {
            writer.write_all(&[temp])?;
            break;
        }
    }
    Ok(())
}

/// 读取 Minecraft 标准 VarInt
pub fn read_varint<R: Read>(reader: &mut R) -> io::Result<i32> {
    let mut num_read = 0;
    let mut result: u32 = 0;
    loop {
        let mut byte = [0u8; 1];
        reader.read_exact(&mut byte)?;
        let val = byte[0];
        result |= ((val & 0x7F) as u32) << (7 * num_read);
        num_read += 1;
        if num_read > 5 {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "VarInt too long"));
        }
        if (val & 0x80) == 0 {
            break;
        }
    }
    Ok(result as i32)
}

/// 麦克风音频数据包 (Client -> Server: 0x01)
#[derive(Debug, Clone)]
pub struct MicPacket {
    pub opus_data: Vec<u8>,
    pub sequence_number: i64,
    pub whispering: bool,
}

impl MicPacket {
    pub fn to_bytes(&self) -> io::Result<Vec<u8>> {
        let mut buf = Vec::with_capacity(5 + self.opus_data.len() + 8 + 1);
        write_varint(self.opus_data.len() as i32, &mut buf)?;
        buf.write_all(&self.opus_data)?;
        buf.write_i64::<BigEndian>(self.sequence_number)?;
        buf.write_u8(if self.whispering { 1 } else { 0 })?;
        Ok(buf)
    }

    pub fn from_bytes(data: &[u8]) -> io::Result<Self> {
        let mut cursor = Cursor::new(data);
        let opus_len = read_varint(&mut cursor)? as usize;
        let mut opus_data = vec![0u8; opus_len];
        cursor.read_exact(&mut opus_data)?;
        let sequence_number = cursor.read_i64::<BigEndian>()?;
        let whispering = cursor.read_u8()? != 0;
        Ok(Self {
            opus_data,
            sequence_number,
            whispering,
        })
    }
}

/// 客户端认证包 (Client -> Server: 0x05)
#[derive(Debug, Clone)]
pub struct AuthenticatePacket {
    pub player_uuid: Uuid,
    pub secret: [u8; 16],
}

impl AuthenticatePacket {
    pub fn to_bytes(&self) -> io::Result<Vec<u8>> {
        let mut buf = Vec::with_capacity(32);
        buf.write_all(self.player_uuid.as_bytes())?;
        buf.write_all(&self.secret)?;
        Ok(buf)
    }

    pub fn from_bytes(data: &[u8]) -> io::Result<Self> {
        if data.len() < 32 {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "Auth packet too short"));
        }
        let player_uuid = Uuid::from_slice(&data[0..16])
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        let mut secret = [0u8; 16];
        secret.copy_from_slice(&data[16..32]);
        Ok(Self { player_uuid, secret })
    }
}

/// Ping 包 (0x07)
#[derive(Debug, Clone)]
pub struct PingPacket {
    pub id: Uuid,
    pub timestamp: i64,
}

impl PingPacket {
    pub fn to_bytes(&self) -> io::Result<Vec<u8>> {
        let mut buf = Vec::with_capacity(24);
        buf.write_all(self.id.as_bytes())?;
        buf.write_i64::<BigEndian>(self.timestamp)?;
        Ok(buf)
    }

    pub fn from_bytes(data: &[u8]) -> io::Result<Self> {
        if data.len() < 24 {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "Ping packet too short"));
        }
        let id = Uuid::from_slice(&data[0..16])
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
        let mut cursor = Cursor::new(&data[16..24]);
        let timestamp = cursor.read_i64::<BigEndian>()?;
        Ok(Self { id, timestamp })
    }
}

/// 写入 Minecraft 格式字符串 (VarInt 长度 + UTF-8 字节)
pub fn write_mc_string<W: Write>(s: &str, writer: &mut W) -> io::Result<()> {
    let bytes = s.as_bytes();
    write_varint(bytes.len() as i32, writer)?;
    writer.write_all(bytes)?;
    Ok(())
}

/// 读取 Minecraft 格式字符串 (VarInt 长度 + UTF-8 字节)
pub fn read_mc_string<R: Read>(reader: &mut R) -> io::Result<String> {
    let len = read_varint(reader)? as usize;
    if len > 32767 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "String length too large"));
    }
    let mut buf = vec![0u8; len];
    reader.read_exact(&mut buf)?;
    String::from_utf8(buf).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

/// Simple Voice Chat 语音群组信息 (ClientGroup)
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SvcGroup {
    pub id: Uuid,
    pub name: String,
    pub has_password: bool,
    pub persistent: bool,
    pub hidden: bool,
    pub group_type: i16, // 0 = Normal, 1 = Open, 2 = Isolated
}

impl SvcGroup {
    pub fn from_bytes(bytes: &[u8]) -> io::Result<Self> {
        let mut cursor = Cursor::new(bytes);
        let mut uuid_bytes = [0u8; 16];
        cursor.read_exact(&mut uuid_bytes)?;
        let id = Uuid::from_bytes(uuid_bytes);
        let name = read_mc_string(&mut cursor)?;
        let has_password = cursor.read_u8()? != 0;
        let persistent = cursor.read_u8()? != 0;
        let hidden = cursor.read_u8()? != 0;
        let group_type = cursor.read_i16::<BigEndian>()?;

        Ok(Self {
            id,
            name,
            has_password,
            persistent,
            hidden,
            group_type,
        })
    }
}

/// 构造 voicechat:create_group 负载
pub fn create_group_payload(name: &str, password: Option<&str>, group_type: i16) -> io::Result<Vec<u8>> {
    let mut buf = Vec::new();
    write_mc_string(name, &mut buf)?;
    if let Some(pwd) = password {
        buf.write_u8(1)?;
        write_mc_string(pwd, &mut buf)?;
    } else {
        buf.write_u8(0)?;
    }
    buf.write_i16::<BigEndian>(group_type)?;
    Ok(buf)
}

/// 构造 voicechat:set_group 负载
pub fn join_group_payload(group_id: Uuid, password: Option<&str>) -> io::Result<Vec<u8>> {
    let mut buf = Vec::new();
    buf.write_all(group_id.as_bytes())?;
    if let Some(pwd) = password {
        buf.write_u8(1)?;
        write_mc_string(pwd, &mut buf)?;
    } else {
        buf.write_u8(0)?;
    }
    Ok(buf)
}

/// 解析 voicechat:joined_group 响应负载 -> (Option<UUID>, wrong_password)
pub fn parse_joined_group_packet(bytes: &[u8]) -> io::Result<(Option<Uuid>, bool)> {
    let mut cursor = Cursor::new(bytes);
    let has_group = cursor.read_u8()? != 0;
    let group_id = if has_group {
        let mut uuid_bytes = [0u8; 16];
        cursor.read_exact(&mut uuid_bytes)?;
        Some(Uuid::from_bytes(uuid_bytes))
    } else {
        None
    };
    let wrong_password = cursor.read_u8()? != 0;
    Ok((group_id, wrong_password))
}

/// 解析 voicechat:remove_group 负载 -> UUID
pub fn parse_remove_group_packet(bytes: &[u8]) -> io::Result<Uuid> {
    let mut cursor = Cursor::new(bytes);
    let mut uuid_bytes = [0u8; 16];
    cursor.read_exact(&mut uuid_bytes)?;
    Ok(Uuid::from_bytes(uuid_bytes))
}

