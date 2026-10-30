use azalea_chat::click_event::ClickEvent;
use azalea_chat::translatable_component::PrimitiveOrComponent;
use azalea_chat::FormattedText;

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

/// 判断字符是否为合法 Minecraft 玩家名字符（Java 版原生为 a-z, A-Z, 0-9, _；基岩版/Geyser 允许前缀 . 或 *）
pub fn is_valid_player_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '*'
}

/// 验证字符串是否为合法的 Minecraft 玩家名（必须包含至少一个字母数字，且不可包含中文字符等）
pub fn is_valid_player_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 20
        && name.chars().all(is_valid_player_char)
        && name.chars().any(|c| c.is_ascii_alphanumeric())
}

/// 从前缀之前的字符串中提取疑似玩家名
pub fn extract_sender(prefix_text: &str) -> Option<String> {
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

pub fn append_component_with_links(component: &FormattedText, out: &mut String) {
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
