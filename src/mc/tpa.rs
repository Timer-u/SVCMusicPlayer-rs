use std::sync::Arc;
use std::time::Duration;
use tracing::info;

use azalea_chat::click_event::ClickEvent;
use azalea_chat::translatable_component::PrimitiveOrComponent;
use azalea_chat::FormattedText;

use crate::app::AppManager;
use crate::mc::parser::{is_bot_message, strip_mc_formatting};

/// 递归遍历 Minecraft 文本组件，寻找包含 tpaccept 的交互点击事件指令 (run_command / suggest_command)
pub fn find_tpa_click_command(component: &FormattedText) -> Option<String> {
    let base = component.get_base();
    if let Some(ref ce) = base.style.click_event {
        let cmd_opt = match ce {
            ClickEvent::RunCommand { command } => Some(command.as_str()),
            ClickEvent::SuggestCommand { command } => Some(command.as_str()),
            _ => None,
        };
        if let Some(cmd) = cmd_opt {
            let lower = cmd.to_lowercase();
            if lower.contains("tpaccept") || lower.contains("tpyes") {
                let formatted = if cmd.starts_with('/') {
                    cmd.to_string()
                } else {
                    format!("/{}", cmd)
                };
                return Some(formatted);
            }
        }
    }

    for sibling in &base.siblings {
        if let Some(cmd) = find_tpa_click_command(sibling) {
            return Some(cmd);
        }
    }

    if let FormattedText::Translatable(t) = component {
        for arg in &t.args {
            if let PrimitiveOrComponent::FormattedText(inner) = arg {
                if let Some(cmd) = find_tpa_click_command(inner) {
                    return Some(cmd);
                }
            }
        }
    }

    None
}

/// 检测文本或组件是否为来自 CMI 的 TPA 传送请求
pub fn detect_tpa_request(
    component: &FormattedText,
    raw_text: &str,
    bot_name: &str,
) -> Option<String> {
    // 忽略机器人自身发言或包含 [Bot] 的消息，防止回环
    if is_bot_message(raw_text, bot_name) {
        return None;
    }

    let cleaned = strip_mc_formatting(raw_text);

    // 1. 优先从交互式组件 ClickEvent 中提取最准确的执行指令（如 /cmi tpaccept <player> 或 /tpaccept）
    if let Some(click_cmd) = find_tpa_click_command(component) {
        return Some(click_cmd);
    }

    // 2. 若无 ClickEvent，则根据 CMI 消息的文本特征进行匹配
    let lower = cleaned.to_lowercase();

    let is_tpa = (
        // 中文提示词（如：“玩家 xxx 请求传送到你这里”、“想要传送到你”、“请求传送到您的位置”等）
        (lower.contains("请求传送") || lower.contains("想要传送") || lower.contains("传送请求"))
            && (lower.contains("到你")
                || lower.contains("至你")
                || lower.contains("你这里")
                || lower.contains("您的位置")
                || lower.contains("接受")
                || lower.contains("同意")
                || lower.contains("tpaccept"))
    ) || (
        // 英文提示词（CMI 默认: "xxx has requested to teleport to you", "Type /cmi tpaccept to accept"）
        lower.contains("has requested to teleport")
            || lower.contains("has requested you to teleport")
            || (lower.contains("teleport request")
                && (lower.contains("accept") || lower.contains("received")))
    ) || (
        // 明确带有接受传送提示的指令行
        (lower.contains("/tpaccept") || lower.contains("/cmi tpaccept"))
            && (lower.contains("输入")
                || lower.contains("type")
                || lower.contains("点击")
                || lower.contains("click")
                || lower.contains("接受")
                || lower.contains("accept")
                || lower.contains("同意"))
    );

    if is_tpa {
        // CMI 默认原生接受指令为 /cmi tpaccept
        Some("/cmi tpaccept".to_string())
    } else {
        None
    }
}

/// 检查并自动同意 CMI TPA 传送请求（带 1.5s 防抖）
pub async fn check_and_handle_tpa(
    app: &Arc<AppManager>,
    formatted: &FormattedText,
    raw_text: &str,
) {
    if !app.config.control.auto_tpaccept {
        return;
    }

    let bot_name = &app.config.minecraft.bot_name;
    if let Some(accept_cmd) = detect_tpa_request(formatted, raw_text, bot_name) {
        // 检查防抖 (避免 1.5s 内短时间内连续多条消息重复执行)
        let should_accept = {
            let mut last = app.last_tpa_accepted.lock().await;
            let now = tokio::time::Instant::now();
            if let Some(prev) = *last {
                if now.duration_since(prev) < Duration::from_millis(1500) {
                    false
                } else {
                    *last = Some(now);
                    true
                }
            } else {
                *last = Some(now);
                true
            }
        };

        if should_accept {
            info!(
                "[MC-TPA] 检测到来自 CMI 的传送请求，正在自动同意: \"{}\"",
                accept_cmd
            );
            let app_clone = Arc::clone(app);
            tokio::spawn(async move {
                // 稍作延时 300ms 保证服务端与客户端状态同步
                tokio::time::sleep(Duration::from_millis(300)).await;
                app_clone.send_game_chat(&accept_cmd).await;
            });
        }
    }
}
