use std::sync::Arc;
use tracing::info;

use crate::app::{AppManager, GroupAction};
use crate::audio::{PlayMode, RequestOrigin};

/// 检查发送者是否具有管理员权限
fn check_admin(app: &AppManager, sender: Option<&str>) -> bool {
    let admin_list = &app.config.control.admin_players;
    if admin_list.is_empty() {
        return true;
    }
    if let Some(s) = sender {
        admin_list.iter().any(|adm| adm.eq_ignore_ascii_case(s))
    } else {
        false
    }
}

/// 权限不足时的拒绝提示
async fn reject_permission(app: &AppManager, sender: Option<&str>) {
    if let Some(s) = sender {
        app.send_game_chat(&format!("[Bot] @{} 你没有权限使用该指令！", s)).await;
    } else {
        app.send_game_chat("[Bot] 无法验证指令发送者身份，拒绝执行").await;
    }
}

/// 执行游戏内聊天指令
pub async fn execute_command(
    app: Arc<AppManager>,
    sender: Option<String>,
    cmd: &str,
    args: &[&str],
) {
    let sender_ref = sender.as_deref();

    match cmd {
        "play" => {
            if args.is_empty() {
                app.send_game_chat("[Bot] 用法: !play <歌曲链接/本地文件/B站或YT链接>").await;
                return;
            }
            let target = args.join(" ");
            let sender_tag = sender.as_ref().map(|s| format!("@{} ", s)).unwrap_or_default();
            app.send_game_chat(&format!("[Bot] {}正在解析链接，请稍候...", sender_tag)).await;

            let origin = RequestOrigin::GameChat {
                sender: sender.clone(),
            };
            match app.request_play_with_options(&target, origin, false).await {
                Ok(msg) => {
                    app.send_game_chat(&format!("[Bot] {}{}", sender_tag, msg)).await;
                }
                Err(e) => {
                    app.send_game_chat(&format!("[Bot] {}点歌失败: {}", sender_tag, e)).await;
                }
            }
        }
        "playnow" | "pn" => {
            if args.is_empty() {
                app.send_game_chat("[Bot] 用法: !playnow <歌曲链接/本地文件/B站或YT链接>").await;
                return;
            }
            if !check_admin(&app, sender_ref) {
                reject_permission(&app, sender_ref).await;
                return;
            }
            let target = args.join(" ");
            let sender_tag = sender.as_ref().map(|s| format!("@{} ", s)).unwrap_or_default();
            app.send_game_chat(&format!("[Bot] {}正在解析链接并准备立即播放...", sender_tag)).await;

            let origin = RequestOrigin::GameChat {
                sender: sender.clone(),
            };
            match app.request_play_with_options(&target, origin, true).await {
                Ok(msg) => {
                    app.send_game_chat(&format!("[Bot] {}{}", sender_tag, msg)).await;
                }
                Err(e) => {
                    app.send_game_chat(&format!("[Bot] {}插播失败: {}", sender_tag, e)).await;
                }
            }
        }
        "pause" => {
            if !check_admin(&app, sender_ref) {
                reject_permission(&app, sender_ref).await;
                return;
            }
            if let Err(e) = app.pause().await {
                app.send_game_chat(&format!("[Bot] 暂停失败: {}", e)).await;
            } else {
                app.send_game_chat("[Bot] 播放已暂停").await;
            }
        }
        "resume" => {
            if !check_admin(&app, sender_ref) {
                reject_permission(&app, sender_ref).await;
                return;
            }
            if let Err(e) = app.resume().await {
                app.send_game_chat(&format!("[Bot] 恢复失败: {}", e)).await;
            } else {
                app.send_game_chat("[Bot] 播放已恢复").await;
            }
        }
        "stop" => {
            if !check_admin(&app, sender_ref) {
                reject_permission(&app, sender_ref).await;
                return;
            }
            if let Err(e) = app.stop().await {
                app.send_game_chat(&format!("[Bot] 停止失败: {}", e)).await;
            } else {
                app.send_game_chat("[Bot] 已停止当前播放（歌单已保留，清空请用 !clear）").await;
            }
        }
        "skip" | "next" => {
            if !check_admin(&app, sender_ref) {
                reject_permission(&app, sender_ref).await;
                return;
            }
            match app.skip().await {
                Ok(Some(next)) => {
                    app.send_game_chat(&format!("[Bot] 已切歌，正在播放: {}", next)).await;
                }
                Ok(None) => {
                    app.send_game_chat("[Bot] 歌单已播完").await;
                }
                Err(e) => {
                    app.send_game_chat(&format!("[Bot] 切歌失败: {}", e)).await;
                }
            }
        }
        "prev" => {
            if !check_admin(&app, sender_ref) {
                reject_permission(&app, sender_ref).await;
                return;
            }
            match app.prev().await {
                Ok(Some(prev)) => {
                    app.send_game_chat(&format!("[Bot] 已切回上一首，正在播放: {}", prev)).await;
                }
                Ok(None) => {
                    app.send_game_chat("[Bot] 歌单中无上一首曲目").await;
                }
                Err(e) => {
                    app.send_game_chat(&format!("[Bot] 切回上一首失败: {}", e)).await;
                }
            }
        }
        "mode" => {
            if args.is_empty() {
                let mode = app.get_play_mode().await;
                app.send_game_chat(&format!(
                    "[Bot] 当前播放模式: {} (可用: !mode loop|single|shuffle|seq)",
                    mode.display_cn()
                ))
                .await;
            } else {
                if !check_admin(&app, sender_ref) {
                    reject_permission(&app, sender_ref).await;
                    return;
                }
                if let Some(mode) = PlayMode::from_input(args[0]) {
                    app.set_play_mode(mode).await;
                    app.send_game_chat(&format!("[Bot] 播放模式已切换为: {}", mode.display_cn())).await;
                } else {
                    app.send_game_chat(
                        "[Bot] 无效的播放模式！可用: !mode loop|single|shuffle|seq",
                    )
                    .await;
                }
            }
        }
        "loop" => {
            if !check_admin(&app, sender_ref) {
                reject_permission(&app, sender_ref).await;
                return;
            }
            app.set_play_mode(PlayMode::ListLoop).await;
            app.send_game_chat("[Bot] 播放模式已切换为: 列表循环").await;
        }
        "single" => {
            if !check_admin(&app, sender_ref) {
                reject_permission(&app, sender_ref).await;
                return;
            }
            app.set_play_mode(PlayMode::SingleLoop).await;
            app.send_game_chat("[Bot] 播放模式已切换为: 单曲循环").await;
        }
        "shuffle" | "random" => {
            if !check_admin(&app, sender_ref) {
                reject_permission(&app, sender_ref).await;
                return;
            }
            app.set_play_mode(PlayMode::Shuffle).await;
            app.send_game_chat("[Bot] 播放模式已切换为: 随机播放").await;
        }
        "seq" => {
            if !check_admin(&app, sender_ref) {
                reject_permission(&app, sender_ref).await;
                return;
            }
            app.set_play_mode(PlayMode::Sequential).await;
            app.send_game_chat("[Bot] 播放模式已切换为: 顺序播放").await;
        }
        "list" | "queue" | "playlist" | "pl" => {
            let page_req: usize = args
                .first()
                .and_then(|s| s.parse::<usize>().ok())
                .unwrap_or(1)
                .max(1);

            let page_size = 6;
            let pl = app.playlist.lock().await;
            let total = pl.tracks.len();
            if total == 0 {
                app.send_game_chat("[Bot] 当前歌单为空").await;
                return;
            }

            let total_pages = (total + page_size - 1) / page_size;
            let page = page_req.min(total_pages);
            let start = (page - 1) * page_size;
            let end = (start + page_size).min(total);

            let mode_cn = pl.mode.display_cn();
            let header = format!(
                "[Bot] 当前歌单 [模式: {}, 共 {} 首, 第 {}/{} 页]:",
                mode_cn, total, page, total_pages
            );

            let items: Vec<(usize, bool, String)> = pl.tracks[start..end]
                .iter()
                .enumerate()
                .map(|(offset, t)| {
                    let idx = start + offset;
                    (idx + 1, pl.current_index == Some(idx), t.title.clone())
                })
                .collect();
            drop(pl);

            app.send_game_chat(&header).await;
            for (num, is_cur, title) in items {
                let marker = if is_cur { "▶ " } else { "" };
                app.send_game_chat(&format!("{}. {}{}", num, marker, title)).await;
            }
        }
        "jump" | "skipto" => {
            if args.is_empty() {
                app.send_game_chat("[Bot] 用法: !jump <歌单序号>").await;
                return;
            }
            if !check_admin(&app, sender_ref) {
                reject_permission(&app, sender_ref).await;
                return;
            }
            let idx = match args[0].parse::<usize>() {
                Ok(i) => i,
                Err(_) => {
                    app.send_game_chat("[Bot] 请输入有效的曲目序号（数字）").await;
                    return;
                }
            };
            match app.jump_to_index(idx).await {
                Ok(title) => {
                    app.send_game_chat(&format!("[Bot] 已跳转到第 {} 首，正在播放: {}", idx, title)).await;
                }
                Err(e) => {
                    app.send_game_chat(&format!("[Bot] 跳转失败: {}", e)).await;
                }
            }
        }
        "top" | "pin" => {
            if args.is_empty() {
                app.send_game_chat("[Bot] 用法: !top <歌单序号>").await;
                return;
            }
            if !check_admin(&app, sender_ref) {
                reject_permission(&app, sender_ref).await;
                return;
            }
            let idx = match args[0].parse::<usize>() {
                Ok(i) => i,
                Err(_) => {
                    app.send_game_chat("[Bot] 请输入有效的曲目序号（数字）").await;
                    return;
                }
            };
            match app.move_to_next(idx).await {
                Ok(title) => {
                    app.send_game_chat(&format!("[Bot] 已置顶为下一首: {}", title)).await;
                }
                Err(e) => {
                    app.send_game_chat(&format!("[Bot] 置顶失败: {}", e)).await;
                }
            }
        }
        "rm" | "del" | "remove" => {
            if args.is_empty() {
                app.send_game_chat("[Bot] 用法: !rm <歌单序号>").await;
                return;
            }
            if !check_admin(&app, sender_ref) {
                reject_permission(&app, sender_ref).await;
                return;
            }
            let idx = match args[0].parse::<usize>() {
                Ok(i) => i,
                Err(_) => {
                    app.send_game_chat("[Bot] 请输入有效的曲目序号（数字）").await;
                    return;
                }
            };
            match app.remove_at_index(idx).await {
                Ok(title) => {
                    app.send_game_chat(&format!("[Bot] 已从歌单移除: {}", title)).await;
                }
                Err(e) => {
                    app.send_game_chat(&format!("[Bot] 移除失败: {}", e)).await;
                }
            }
        }
        "clear" => {
            if !check_admin(&app, sender_ref) {
                reject_permission(&app, sender_ref).await;
                return;
            }
            match app.clear_playlist().await {
                Ok(()) => {
                    app.send_game_chat("[Bot] 已停止播放并清空全部歌单").await;
                }
                Err(e) => {
                    app.send_game_chat(&format!("[Bot] 清空歌单失败: {}", e)).await;
                }
            }
        }
        "rg" | "loudnorm" => {
            if args.is_empty() {
                let enabled = app.is_replay_gain_enabled().await;
                let current_gain = app.get_current_gain_db().await;
                let status_str = if enabled { "开启" } else { "关闭" };
                let gain_str = match current_gain {
                    Some(db) => format!(" (实时增益: {:+.1} dB)", db),
                    None => String::new(),
                };
                app.send_game_chat(&format!("[Bot] ReplayGain 响度平衡: {}{}", status_str, gain_str)).await;
            } else {
                if !check_admin(&app, sender_ref) {
                    reject_permission(&app, sender_ref).await;
                    return;
                }
                match args[0].to_lowercase().as_str() {
                    "on" | "true" | "1" | "开启" | "开" => {
                        app.set_replay_gain_enabled(true).await;
                        app.send_game_chat("[Bot] ReplayGain 响度平衡已开启").await;
                    }
                    "off" | "false" | "0" | "关闭" | "关" => {
                        app.set_replay_gain_enabled(false).await;
                        app.send_game_chat("[Bot] ReplayGain 响度平衡已关闭").await;
                    }
                    _ => {
                        app.send_game_chat("[Bot] 用法: !rg [on/off/开启/关闭]").await;
                    }
                }
            }
        }
        "vol" | "volume" => {
            if args.is_empty() {
                let cur = app.get_volume().await;
                app.send_game_chat(&format!("[Bot] 当前音量: {}%", cur)).await;
                return;
            }
            if !check_admin(&app, sender_ref) {
                reject_permission(&app, sender_ref).await;
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
            if !check_admin(&app, sender_ref) {
                reject_permission(&app, sender_ref).await;
                return;
            }
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
                    ))
                    .await;
                } else {
                    app.send_game_chat("[Bot] 当前未加入任何语音群组。可用: !group join <群名> [密码]").await;
                }
                return;
            }
            if !check_admin(&app, sender_ref) {
                reject_permission(&app, sender_ref).await;
                return;
            }
            match args[0].to_lowercase().as_str() {
                "join" => {
                    if args.len() < 2 {
                        app.send_game_chat("[Bot] 用法: !group join <群组名> [密码]").await;
                        return;
                    }
                    let name = args[1].to_string();
                    let password = if args.len() >= 3 {
                        Some(args[2].to_string())
                    } else {
                        None
                    };
                    let group_type = match app.config.voice.group.group_type.to_lowercase().as_str() {
                        "isolated" => 2,
                        "open" => 1,
                        _ => 0,
                    };
                    app.send_group_action(GroupAction::JoinOrCreate {
                        name: name.clone(),
                        password,
                        group_type,
                    })
                    .await;
                    app.send_game_chat(&format!("[Bot] 正在加入或创建群组: \"{}\"...", name)).await;
                }
                "leave" => {
                    app.send_group_action(GroupAction::Leave).await;
                    app.send_game_chat("[Bot] 已发送离开当前群组请求").await;
                }
                _ => {
                    app.send_game_chat("[Bot] 用法: !group [join <群名> [密码] | leave]").await;
                }
            }
        }
        "help" => {
            app.send_game_chat(
                "[Bot] 指令列表: !play <链接>, !playnow <链接>, !pause, !resume, !skip, !prev, !stop, !mode [模式], !list [页码], !jump <序号>, !top <序号>, !rm <序号>, !clear, !vol <0-100>, !rg [on/off], !tpaccept, !group",
            )
            .await;
        }
        _ => {}
    }
}
