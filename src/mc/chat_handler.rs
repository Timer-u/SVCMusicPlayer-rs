use std::sync::Arc;
use tracing::info;

use azalea_chat::click_event::ClickEvent;
use azalea_chat::translatable_component::PrimitiveOrComponent;
use azalea_chat::FormattedText;
use azalea_client::chat::ChatPacket;

use crate::app::AppManager;

pub struct ChatHandler;

impl ChatHandler {
    /// 判断一条接收到的消息是否来自机器人自身或属于机器人系统回显（防止自触发与回环）
    pub fn is_bot_message(raw_text: &str, bot_name: &str) -> bool {
        let cleaned = strip_mc_formatting(raw_text);
        let trimmed = cleaned.trim();
        let lower = trimmed.to_lowercase();

        // 1. 消息直接以 [bot] 或 【bot】 开头（如 "[Bot] 指令列表..."）
        if lower.starts_with("[bot]") || lower.starts_with("【bot】") {
            return true;
        }

        // 2. 消息内容中包含带有前置分隔的 [bot] / 【bot】 标签
        // 例如 "doveoverthere: [Bot] ..."、"[VIP] doveoverthere: [Bot] ..."、"doveoverthere » [Bot] ..."
        if lower.contains(": [bot]")
            || lower.contains(":[bot]")
            || lower.contains("» [bot]")
            || lower.contains("»[bot]")
            || lower.contains("> [bot]")
            || lower.contains(">[bot]")
            || lower.contains("] [bot]")
            || lower.contains("][bot]")
            || lower.contains(": 【bot】")
            || lower.contains(":【bot】")
            || lower.contains("» 【bot】")
            || lower.contains("> 【bot】")
            || lower.contains("] 【bot】")
        {
            return true;
        }

        // 3. 检查发言者是否为机器人自身 (bot_name)
        if !bot_name.is_empty() {
            let lower_bot = bot_name.to_lowercase();
            // 直接以 bot_name 开头（如 "doveoverthere: xxx"）
            if lower.starts_with(&lower_bot) {
                let remainder = &lower[lower_bot.len()..];
                if remainder.starts_with(':')
                    || remainder.starts_with(' ')
                    || remainder.starts_with('>')
                    || remainder.starts_with('»')
                {
                    return true;
                }
            }
            // 原版聊天格式 "<doveoverthere> ..."
            if lower.starts_with(&format!("<{}>", lower_bot)) {
                return true;
            }
            // 包含带分隔符的 bot_name 发言格式，例如 "[VIP] doveoverthere: ..."
            if lower.contains(&format!(" {}:", lower_bot))
                || lower.contains(&format!("]{}:", lower_bot))
                || lower.contains(&format!("] {}:", lower_bot))
                || lower.contains(&format!(" {} »", lower_bot))
                || lower.contains(&format!("] {} »", lower_bot))
            {
                return true;
            }
        }

        false
    }

    /// 统一处理游戏内接收到的聊天数据包（包括交互点击事件、系统通知、玩家发言）
    pub async fn handle_chat_packet(app: Arc<AppManager>, chat_pkt: &ChatPacket) {
        let formatted = chat_pkt.message();
        let raw_text = extract_chat_text_with_links(&formatted);
        info!("[MC-Chat] 收到游戏内聊天消息: {}", raw_text);

        let bot_name = &app.config.minecraft.bot_name;

        // 0. 过滤机器人自身消息与包含 [Bot] 的回显消息，防止自触发与回环
        if Self::is_bot_message(&raw_text, bot_name) {
            return;
        }

        // 1. 检查是否开启了自动同意 TPA
        if app.config.control.auto_tpaccept {
            if let Some(accept_cmd) =
                Self::detect_tpa_request(&formatted, &raw_text, bot_name)
            {
                // 检查防抖 (避免短时间内连续多条消息重复执行)
                let should_accept = {
                    let mut last = app.last_tpa_accepted.lock().await;
                    let now = tokio::time::Instant::now();
                    if let Some(prev) = *last {
                        if now.duration_since(prev) < tokio::time::Duration::from_millis(1500) {
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
                    let app_clone = Arc::clone(&app);
                    tokio::spawn(async move {
                        // 稍作延时 300ms 保证服务端与客户端状态同步
                        tokio::time::sleep(tokio::time::Duration::from_millis(300)).await;
                        app_clone.send_game_chat(&accept_cmd).await;
                    });
                }
            }
        }

        // 2. 处理常规机器人控制指令 (!play, !pause 等)
        Self::handle_message(app, &raw_text).await;
    }

    /// 检测文本或组件是否为来自 CMI 的 TPA 传送请求
    pub fn detect_tpa_request(
        component: &FormattedText,
        raw_text: &str,
        bot_name: &str,
    ) -> Option<String> {
        // 忽略机器人自身发言或包含 [Bot] 的消息，防止回环
        if Self::is_bot_message(raw_text, bot_name) {
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

    pub async fn handle_message(app: Arc<AppManager>, raw_message: &str) {
        let bot_name = &app.config.minecraft.bot_name;

        // 过滤来自机器人自身或以 [Bot] 开头的提示消息
        if Self::is_bot_message(raw_message, bot_name) {
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

        // 权限检查
        let admin_list = &app.config.control.admin_players;
        if !admin_list.is_empty() {
            if let Some(ref sender) = extracted_sender {
                if !admin_list.iter().any(|adm| adm.eq_ignore_ascii_case(sender)) {
                    app.send_game_chat(&format!("[Bot] @{} 你没有权限使用该指令！", sender)).await;
                    return;
                }
            } else {
                app.send_game_chat("[Bot] 无法验证指令发送者身份，拒绝执行").await;
                return;
            }
        }

        match cmd.as_str() {
            "play" => {
                if args.is_empty() {
                    app.send_game_chat("[Bot] 用法: !play <歌曲链接/本地文件/B站或YT链接>").await;
                    return;
                }
                let target = args.join(" ");
                let sender_tag = extracted_sender.as_ref().map(|s| format!("@{} ", s)).unwrap_or_default();
                app.send_game_chat(&format!("[Bot] {}正在解析链接，请稍候...", sender_tag)).await;

                let origin = crate::audio::RequestOrigin::GameChat { sender: extracted_sender.clone() };
                match app.request_play(&target, origin).await {
                    Ok(msg) => {
                        app.send_game_chat(&format!("[Bot] {}{}", sender_tag, msg)).await;
                    }
                    Err(e) => {
                        app.send_game_chat(&format!("[Bot] {}点歌失败: {}", sender_tag, e)).await;
                    }
                }
            }
            "pause" => {
                if let Err(e) = app.pause().await {
                    app.send_game_chat(&format!("[Bot] 暂停失败: {}", e)).await;
                } else {
                    app.send_game_chat("[Bot] 已暂停播放").await;
                }
            }
            "resume" => {
                if let Err(e) = app.resume().await {
                    app.send_game_chat(&format!("[Bot] 恢复失败: {}", e)).await;
                } else {
                    app.send_game_chat("[Bot] 已恢复播放").await;
                }
            }
            "skip" => {
                match app.skip().await {
                    Ok(Some(next)) => {
                        app.send_game_chat(&format!("[Bot] 已切歌，正在播放: {}", next)).await;
                    }
                    Ok(None) => {
                        app.send_game_chat("[Bot] 队列已空，已停止播放").await;
                    }
                    Err(e) => {
                        app.send_game_chat(&format!("[Bot] 切歌失败: {}", e)).await;
                    }
                }
            }
            "stop" => {
                if let Err(e) = app.stop().await {
                    app.send_game_chat(&format!("[Bot] 停止失败: {}", e)).await;
                } else {
                    app.send_game_chat("[Bot] 播放已停止，播放队列已清空").await;
                }
            }
            "vol" | "volume" => {
                if args.is_empty() {
                    let cur = app.get_volume().await;
                    app.send_game_chat(&format!("[Bot] 当前音量: {}%", cur)).await;
                    return;
                }
                if let Ok(val) = args[0].parse::<u32>() {
                    app.set_volume(val).await;
                    app.send_game_chat(&format!("[Bot] 音量已设为: {}%", val.min(100))).await;
                } else {
                    app.send_game_chat("[Bot] 用法: !vol <0-100>").await;
                }
            }
            "tpaccept" | "tpa" => {
                app.send_game_chat("/cmi tpaccept").await;
                info!("[MC-Chat] 收到玩家指令 !tpaccept，已发送 /cmi tpaccept");
            }
            "group" => {
                if args.is_empty() {
                    if let Some(cur) = app.get_current_group().await {
                        let type_str = match cur.group_type {
                            1 => "Open (开放群组)",
                            2 => "Isolated (隔离群组)",
                            _ => "Normal (普通群组)",
                        };
                        app.send_game_chat(&format!(
                            "[Bot] 当前所在语音群组: \"{}\" [{}]",
                            cur.name, type_str
                        )).await;
                    } else {
                        app.send_game_chat("[Bot] 当前未加入任何语音群组。可用: !group join <群名> [密码]").await;
                    }
                    return;
                }
                match args[0].to_lowercase().as_str() {
                    "join" => {
                        if args.len() < 2 {
                            app.send_game_chat("[Bot] 用法: !group join <群组名> [密码]").await;
                            return;
                        }
                        let name = args[1].to_string();
                        let password = if args.len() >= 3 { Some(args[2].to_string()) } else { None };
                        let group_type = match app.config.voice.group.group_type.to_lowercase().as_str() {
                            "isolated" => 2,
                            "open" => 1,
                            _ => 0,
                        };
                        app.send_group_action(crate::app::GroupAction::JoinOrCreate {
                            name: name.clone(),
                            password,
                            group_type,
                        }).await;
                        app.send_game_chat(&format!("[Bot] 正在加入或创建群组: \"{}\"...", name)).await;
                    }
                    "leave" => {
                        app.send_group_action(crate::app::GroupAction::Leave).await;
                        app.send_game_chat("[Bot] 已发送离开当前群组请求").await;
                    }
                    _ => {
                        app.send_game_chat("[Bot] 用法: !group [join <群名> [密码] | leave]").await;
                    }
                }
            }
            "help" => {
                app.send_game_chat("[Bot] 指令列表: !play <链接>, !pause, !resume, !skip, !stop, !vol <0-100>, !tpaccept, !group").await;
            }
            _ => {}
        }
    }
}

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

/// 剥离所有 Minecraft 样式代码（§0-§f, §k-§o, §r, §x 等）
pub fn strip_mc_formatting(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '§' {
            // 跳过紧随其后的格式字符
            chars.next();
        } else {
            result.push(c);
        }
    }
    result
}

/// 从任意格式的聊天文本中提取指令内容与发送者名称
/// 例如：
/// - "!play 186016" -> (None, "play 186016")
/// - "<Alice> !play 186016" -> (Some("Alice"), "play 186016")
/// - "[玩家] Alice: !play 186016" -> (Some("Alice"), "play 186016")
/// - "§7[§a生存§7] §fAlice §8» §f!play 186016" -> (Some("Alice"), "play 186016")
/// - "[VIP] Bob : !vol 50" -> (Some("Bob"), "vol 50")
pub fn extract_command(raw_text: &str, prefix: &str) -> Option<(Option<String>, String)> {
    let cleaned = strip_mc_formatting(raw_text);
    let trimmed = cleaned.trim();
    let lower_trimmed = trimmed.to_lowercase();

    // 1. 如果整条消息以 [bot] 或 【bot】 开头，直接判定为机器人提示消息
    if lower_trimmed.starts_with("[bot]") || lower_trimmed.starts_with("【bot】") {
        return None;
    }

    let mut search_idx = 0;
    while let Some(rel_pos) = trimmed[search_idx..].find(prefix) {
        let prefix_pos = search_idx + rel_pos;
        let prefix_end = prefix_pos + prefix.len();

        let prefix_slice = &trimmed[..prefix_pos];
        let lower_prefix = prefix_slice.to_lowercase();

        // 2. 如果前缀之前的文本（发言者或消息前缀）包含 [bot] 或 【bot】，说明指令出现在机器人自身的输出或帮助文本中
        if lower_prefix.contains("[bot]") || lower_prefix.contains("【bot】") {
            return None;
        }

        // 验证这个 prefix 前面是否是合法的分隔（行首，或空白，或 ':' / '»' / '>' / ']' 等）
        let is_valid_start = if prefix_pos == 0 {
            true
        } else {
            let prev_slice = prefix_slice.trim_end();
            if prev_slice.is_empty() {
                true
            } else {
                let last_char = prev_slice.chars().last().unwrap();
                matches!(last_char, ':' | '>' | '»' | ']' | ')' | ' ' | '\t' | '-')
            }
        };

        if is_valid_start {
            let command_and_args = trimmed[prefix_end..].trim().to_string();
            // 如果指令内容为空（例如只是单独打了个 "!"），则继续查找或忽略
            if !command_and_args.is_empty() {
                let sender = extract_sender(prefix_slice);
                return Some((sender, command_and_args));
            }
        }

        search_idx = prefix_end;
    }

    None
}

/// 判断字符是否为合法 Minecraft 玩家名字符（Java 版原生为 a-z, A-Z, 0-9, _；基岩版/Geyser 允许前缀 . 或 *）
fn is_valid_player_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '*'
}

/// 验证字符串是否为合法的 Minecraft 玩家名（必须包含至少一个字母数字，且不可包含中文字符等）
fn is_valid_player_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 20
        && name.chars().all(is_valid_player_char)
        && name.chars().any(|c| c.is_ascii_alphanumeric())
}

/// 从前缀之前的字符串中提取疑似玩家名
fn extract_sender(prefix_text: &str) -> Option<String> {
    let trimmed = prefix_text.trim();
    if trimmed.is_empty() {
        return None;
    }

    // 移除末尾的分隔符，如 ':', '»', '>', ']'
    let stripped = trimmed.trim_end_matches(|c: char| c == ':' || c == '»' || c == '>' || c.is_whitespace());
    
    // 如果是 <PlayerName> 格式
    if let Some(start) = stripped.rfind('<') {
        let inner = stripped[start + 1..].trim();
        if is_valid_player_name(inner) {
            return Some(inner.to_string());
        }
    }

    // 提取最后一个以空格或括号分隔的单词（如 "[VIP] Alice" -> "Alice"）
    let candidate = stripped
        .rsplit(|c: char| c.is_whitespace() || c == ']' || c == ')' || c == '>')
        .find(|s| !s.is_empty())?;

    // 过滤符号并保留合法的玩家名字符
    let candidate = candidate.trim_matches(|c: char| !is_valid_player_char(c));
    if is_valid_player_name(candidate) {
        Some(candidate.to_string())
    } else {
        None
    }
}

/// 判断文本是否为常见的链接占位符（如 [链接]、[link]、[URL] 等）
pub fn is_link_placeholder(text: &str) -> bool {
    let cleaned = strip_mc_formatting(text);
    let trimmed = cleaned.trim();
    if trimmed.is_empty() {
        return false;
    }
    let lower = trimmed.to_lowercase();
    matches!(
        lower.as_str(),
        "[链接]"
            | "【链接】"
            | "[link]"
            | "【link】"
            | "[url]"
            | "【url】"
            | "[网页]"
            | "【网页】"
            | "[网址]"
            | "【网址】"
            | "链接"
            | "link"
            | "url"
            | "网页"
            | "网址"
            | "[click]"
            | "[点击]"
    ) || (lower.starts_with('[') && lower.ends_with(']') && (lower.contains("链") || lower.contains("link") || lower.contains("url")))
      || (lower.starts_with('【') && lower.ends_with('】') && (lower.contains("链") || lower.contains("link") || lower.contains("url")))
}

/// 递归遍历 Minecraft 文本组件树，将附带 ClickEvent::OpenUrl 的链接占位符替换为真实 URL，
/// 同时保留原有普通文本与兄弟节点，还原完整的消息内容
pub fn extract_chat_text_with_links(component: &FormattedText) -> String {
    let mut out = String::new();
    append_component_with_links(component, &mut out);
    out
}

fn append_component_with_links(component: &FormattedText, out: &mut String) {
    let base = component.get_base();

    let open_url = if let Some(ref ce) = base.style.click_event {
        match ce {
            ClickEvent::OpenUrl { url } => Some(url.as_str()),
            _ => None,
        }
    } else {
        None
    };

    match component {
        FormattedText::Text(t) => {
            if let Some(url) = open_url {
                if is_link_placeholder(&t.text)
                    || (!t.text.trim().starts_with("http://")
                        && !t.text.trim().starts_with("https://"))
                {
                    out.push_str(url);
                } else {
                    out.push_str(&t.text);
                }
            } else {
                out.push_str(&t.text);
            }
        }
        FormattedText::Translatable(t) => {
            for arg in &t.args {
                match arg {
                    PrimitiveOrComponent::FormattedText(inner) => {
                        append_component_with_links(inner, out);
                    }
                    PrimitiveOrComponent::String(s) => {
                        if let Some(url) = open_url {
                            if is_link_placeholder(s) {
                                out.push_str(url);
                            } else {
                                out.push_str(s);
                            }
                        } else {
                            out.push_str(s);
                        }
                    }
                    PrimitiveOrComponent::Boolean(b) => {
                        out.push_str(&b.to_string());
                    }
                    PrimitiveOrComponent::Short(s) => {
                        out.push_str(&s.to_string());
                    }
                    PrimitiveOrComponent::Integer(i) => {
                        out.push_str(&i.to_string());
                    }
                    PrimitiveOrComponent::Long(l) => {
                        out.push_str(&l.to_string());
                    }
                    PrimitiveOrComponent::Float(f) => {
                        out.push_str(&f.to_string());
                    }
                    PrimitiveOrComponent::Double(d) => {
                        out.push_str(&d.to_string());
                    }
                }
            }
        }
    }

    for sibling in &base.siblings {
        append_component_with_links(sibling, out);
    }
}
