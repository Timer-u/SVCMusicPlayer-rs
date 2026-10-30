use aes_gcm::{
    aead::{Aead, AeadCore, KeyInit, OsRng},
    Aes128Gcm, Nonce,
};

pub struct VoiceCrypto {
    cipher: Aes128Gcm,
}

impl VoiceCrypto {
    pub fn new(secret_key: &[u8; 16]) -> Self {
        let cipher = Aes128Gcm::new_from_slice(secret_key)
            .expect("Secret key must be 16 bytes for AES-128");
        Self { cipher }
    }

    /// 加密内层负载: packet_type (1 byte) + data
    /// 返回拼接后的字节流: [12 字节 IV] + [密文] + [16 字节 Tag]
    pub fn encrypt(&self, packet_type: u8, data: &[u8]) -> Result<Vec<u8>, aes_gcm::Error> {
        let mut plaintext = Vec::with_capacity(1 + data.len());
        plaintext.push(packet_type);
        plaintext.extend_from_slice(data);

        let nonce = Aes128Gcm::generate_nonce(&mut OsRng); // 12 字节随机 Nonce
        let ciphertext = self.cipher.encrypt(&nonce, plaintext.as_ref())?;

        let mut output = Vec::with_capacity(12 + ciphertext.len());
        output.extend_from_slice(nonce.as_slice());
        output.extend_from_slice(&ciphertext);
        Ok(output)
    }

    /// 解密接收到的负载: [12 字节 IV] + [密文] + [16 字节 Tag]
    /// 返回: (packet_type, payload_bytes)
    pub fn decrypt(&self, payload: &[u8]) -> Result<(u8, Vec<u8>), aes_gcm::Error> {
        if payload.len() < 12 + 16 {
            return Err(aes_gcm::Error);
        }

        let nonce = Nonce::from_slice(&payload[0..12]);
        let ciphertext = &payload[12..];

        let plaintext = self.cipher.decrypt(nonce, ciphertext)?;
        if plaintext.is_empty() {
            return Err(aes_gcm::Error);
        }

        let packet_type = plaintext[0];
        let body = plaintext[1..].to_vec();
        Ok((packet_type, body))
    }
}
