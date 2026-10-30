use std::fmt::Debug;
use std::future::Future;
use std::pin::Pin;
use azalea_client::account::AccountTrait;
use azalea::auth::sessionserver::ClientSessionServerError;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};
use uuid::Uuid;

#[derive(Clone, Debug)]
pub struct YggdrasilAccount {
    pub username: String,
    pub uuid: Uuid,
    pub access_token: String,
    pub auth_server_url: String,
}

#[derive(Serialize)]
struct AuthAgent {
    name: String,
    version: u32,
}

#[derive(Serialize)]
struct AuthRequest<'a> {
    agent: AuthAgent,
    username: &'a str,
    password: &'a str,
    #[serde(rename = "requestUser")]
    request_user: bool,
}

#[derive(Deserialize)]
struct ProfileResponse {
    id: String,
    name: String,
}

#[derive(Deserialize)]
struct AuthResponse {
    #[serde(rename = "accessToken")]
    access_token: String,
    #[serde(rename = "selectedProfile")]
    selected_profile: Option<ProfileResponse>,
    #[serde(rename = "availableProfiles")]
    available_profiles: Option<Vec<ProfileResponse>>,
}

#[derive(Serialize)]
struct JoinRequest<'a> {
    #[serde(rename = "accessToken")]
    access_token: &'a str,
    #[serde(rename = "selectedProfile")]
    selected_profile: &'a str,
    #[serde(rename = "serverId")]
    server_id: &'a str,
}

impl YggdrasilAccount {
    /// 构造已有 Token 的 Yggdrasil 账号（无需明文保存密码）
    pub fn with_token(
        auth_server_url: &str,
        username: &str,
        uuid: Uuid,
        access_token: &str,
    ) -> Self {
        Self {
            username: username.to_string(),
            uuid,
            access_token: access_token.to_string(),
            auth_server_url: auth_server_url.trim_end_matches('/').to_string(),
        }
    }

    /// 使用账号密码登录第三方 Yggdrasil 验证服务器 (如 LittleSkin / Blessing Skin / 自建 authlib-injector 服务)
    pub async fn authenticate(
        auth_server_url: &str,
        username: &str,
        password: &str,
        character_name: Option<&str>,
    ) -> anyhow::Result<Self> {
        let base_url = auth_server_url.trim_end_matches('/');
        let auth_url = format!("{}/authserver/authenticate", base_url);

        info!("[Yggdrasil] 正在向第三方认证服务器发起登录: {}", auth_url);

        let req_body = AuthRequest {
            agent: AuthAgent {
                name: "Minecraft".to_string(),
                version: 1,
            },
            username,
            password,
            request_user: true,
        };

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()?;

        let resp = client.post(&auth_url).json(&req_body).send().await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let err_text = resp.text().await.unwrap_or_default();
            anyhow::bail!("第三方认证服务器登录失败 (HTTP {}): {}", status, err_text);
        }

        let auth_resp: AuthResponse = resp.json().await?;
        let access_token = auth_resp.access_token;

        // 获取角色 Profile
        let profile = if let Some(p) = auth_resp.selected_profile {
            p
        } else if let Some(profiles) = auth_resp.available_profiles {
            if let Some(target_name) = character_name {
                profiles
                    .into_iter()
                    .find(|p| p.name.eq_ignore_ascii_case(target_name))
                    .ok_or_else(|| anyhow::anyhow!("未找到指定角色名称: {}", target_name))?
            } else {
                profiles
                    .into_iter()
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("该账号下没有可用的角色 Profile"))?
            }
        } else {
            anyhow::bail!("认证服务器未返回有效的角色信息");
        };

        let uuid = Uuid::parse_str(&profile.id)
            .map_err(|e| anyhow::anyhow!("解析角色 UUID '{}' 失败: {}", profile.id, e))?;

        info!(
            "[Yggdrasil] 登录成功！角色名称: {}, UUID: {}",
            profile.name, uuid
        );

        Ok(Self {
            username: profile.name,
            uuid,
            access_token,
            auth_server_url: base_url.to_string(),
        })
    }
}

impl AccountTrait for YggdrasilAccount {
    fn username(&self) -> &str {
        &self.username
    }

    fn uuid(&self) -> Uuid {
        self.uuid
    }

    fn access_token(&self) -> Option<String> {
        Some(self.access_token.clone())
    }

    fn join<'a>(
        &'a self,
        public_key: &'a [u8],
        private_key: &'a [u8; 16],
        server_id: &'a str,
        _proxy: Option<reqwest::Proxy>,
    ) -> Pin<Box<dyn Future<Output = Result<(), ClientSessionServerError>> + Send + 'a>> {
        Box::pin(async move {
            let server_hash = azalea_crypto::hex_digest(&azalea_crypto::digest_data(
                server_id.as_bytes(),
                public_key,
                private_key,
            ));

            let mut encode_buffer = Uuid::encode_buffer();
            let undashed_uuid = self.uuid.simple().encode_lower(&mut encode_buffer);

            let join_url = format!(
                "{}/sessionserver/session/minecraft/join",
                self.auth_server_url
            );

            info!(
                "[Yggdrasil] 正在向第三方 SessionServer 提交加入请求: {}",
                join_url
            );

            let req_body = JoinRequest {
                access_token: &self.access_token,
                selected_profile: undashed_uuid,
                server_id: &server_hash,
            };

            let client = reqwest::Client::new();
            let resp = client
                .post(&join_url)
                .json(&req_body)
                .send()
                .await
                .map_err(ClientSessionServerError::HttpError)?;

            if resp.status().is_success() || resp.status() == reqwest::StatusCode::NO_CONTENT {
                info!("[Yggdrasil] SessionServer 验证通过！允许加入服务器。");
                Ok(())
            } else {
                let status = resp.status().as_u16();
                let body = resp.text().await.unwrap_or_default();
                warn!(
                    "[Yggdrasil] SessionServer 返回错误: status={}, body={}",
                    status, body
                );
                Err(ClientSessionServerError::UnexpectedResponse {
                    status_code: status,
                    body,
                })
            }
        })
    }
}
