use std::sync::Arc;
use tracing::info;

use azalea_chat::FormattedText;
use azalea_client::chat::ChatPacket;

use crate::app::AppManager;

pub use crate::mc::commands::*;
pub use crate::mc::parser::*;
pub use crate::mc::tpa::*;

pub struct ChatHandler;

impl ChatHandler {
    /// 判断一条接收到的消息是否来自机器人自身或属于机器人系统回显（防止自触发与回环）
    #[inline]
    pub fn is_bot_message(raw_text: &str, bot_name: &str) -> bool {
        crate::mc::parser::is_bot_message(raw_text, bot_name)
    }

    /// 检测文本或组件是否为来自 CMI 的 TPA 传送请求
    #[inline]
    pub fn detect_tpa_request(
        component: &FormattedText,
        raw_text: &str,
        bot_name: &str,
    ) -> Option<String> {
        crate::mc::tpa::detect_tpa_request(component, raw_text, bot_name)
    }

    /// 统一处理游戏内接收到的聊天数据包（包括交互点击事件、系统通知、玩家发言）
    pub async fn handle_chat_packet(app: Arc<AppManager>, chat_pkt: &ChatPacket) {
        let formatted = chat_pkt.message();
        let raw_text = extract_chat_text_with_links(&formatted);
        info!("[MC-Chat] 收到游戏内聊天消息: {}", raw_text);

        let bot_name = &app.config.minecraft.bot_name;

        // 0. 过滤机器人自身消息与包含 [Bot] 的回显消息，防止自触发与回环
        if is_bot_message(&raw_text, bot_name) {
            return;
        }

        // 1. 检查并自动同意 CMI TPA
        crate::mc::tpa::check_and_handle_tpa(&app, &formatted, &raw_text).await;

        // 2. 处理常规机器人控制指令
        Self::handle_message(app, &raw_text).await;
    }

    /// 解析并处理玩家发送的消息指令
    pub async fn handle_message(app: Arc<AppManager>, raw_message: &str) {
        let bot_name = &app.config.minecraft.bot_name;

        // 过滤来自机器人自身或以 [Bot] 开头的提示消息
        if is_bot_message(raw_message, bot_name) {
            return;
        }

        let prefix = &app.config.control.command_prefix;

        // 自适应解析 CMI / Essentials / 原版等各种聊天格式中的指令与发送者
        let (extracted_sender, cmd_text) = match extract_command(raw_message, prefix) {
            Some(res) => res,
            None => return,
        };

        // 再次确认发言者不是机器人自身
        if let Some(ref sender) = extracted_sender {
            if sender.eq_ignore_ascii_case(bot_name) {
                return;
            }
        }

        info!(
            "[MC-Chat] 识别到有效指令: \"{}\" (发言人: {})",
            cmd_text,
            extracted_sender.as_deref().unwrap_or("未知")
        );

        let mut parts = cmd_text.split_whitespace();
        let cmd = parts.next().unwrap_or("").to_lowercase();
        let args: Vec<&str> = parts.collect();

        crate::mc::commands::execute_command(app, extracted_sender, &cmd, &args).await;
    }
}
