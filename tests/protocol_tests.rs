use azalea_chat::FormattedText;
use rusty_opus::{Application, OpusEncoder};
use std::io::Cursor;
use svc_music_player::mc::chat_handler::ChatHandler;
use svc_music_player::voice::crypto::VoiceCrypto;
use svc_music_player::voice::protocol::*;
use uuid::Uuid;

#[test]
fn test_varint_codec() {
    let test_values = [0, 1, 2, 127, 128, 255, 256, 65535, 2147483647, -1, -2147483648];
    for &val in &test_values {
        let mut buf = Vec::new();
        write_varint(val, &mut buf).expect("Failed to write VarInt");
        let mut cursor = Cursor::new(&buf);
        let decoded = read_varint(&mut cursor).expect("Failed to read VarInt");
        assert_eq!(val, decoded, "VarInt mismatch for value: {}", val);
    }
}

#[test]
fn test_mic_packet_roundtrip() {
    let dummy_opus = vec![0xFC, 0xFF, 0xFE, 0x01, 0x02, 0x03, 0x04];
    let packet = MicPacket {
        opus_data: dummy_opus.clone(),
        sequence_number: 42,
        whispering: true,
    };

    let serialized = packet.to_bytes().expect("Failed to serialize MicPacket");
    let deserialized = MicPacket::from_bytes(&serialized).expect("Failed to deserialize MicPacket");

    assert_eq!(deserialized.sequence_number, 42);
    assert_eq!(deserialized.whispering, true);
    assert_eq!(deserialized.opus_data, dummy_opus);
}

#[test]
fn test_authenticate_packet_roundtrip() {
    let uuid = Uuid::new_v4();
    let secret = [0xAB; 16];
    let packet = AuthenticatePacket {
        player_uuid: uuid,
        secret,
    };

    let serialized = packet.to_bytes().expect("Failed to serialize AuthenticatePacket");
    let deserialized = AuthenticatePacket::from_bytes(&serialized).expect("Failed to deserialize AuthenticatePacket");

    assert_eq!(deserialized.player_uuid, uuid);
    assert_eq!(deserialized.secret, secret);
}

#[test]
fn test_aes_gcm_crypto_roundtrip() {
    let secret_key = [0x42u8; 16];
    let crypto = VoiceCrypto::new(&secret_key);

    let packet_type = PKT_MIC;
    let payload = b"Hello Simple Voice Chat in Rust!";

    let encrypted = crypto.encrypt(packet_type, payload).expect("Encryption failed");
    // [12 字节 Nonce] + [密文] + [16 字节 Tag]
    assert!(encrypted.len() >= 12 + payload.len() + 16);

    let (decrypted_type, decrypted_body) = crypto.decrypt(&encrypted).expect("Decryption failed");
    assert_eq!(decrypted_type, packet_type);
    assert_eq!(decrypted_body, payload);
}

#[test]
fn test_opus_encoder_sine_wave() {
    let mut encoder = OpusEncoder::new(48000, 1, Application::Audio)
        .expect("Failed to create Opus encoder");
    encoder.bitrate_bps = 96000;
    println!("Configured bitrate_bps: {}", encoder.bitrate_bps);

    let frame_size = 960; // 20ms @ 48000Hz
    let mut pcm_frame = vec![0.0f32; frame_size];

    // 生成 440Hz 标准音 (A4) 正弦波
    let freq = 440.0;
    for (i, sample) in pcm_frame.iter_mut().enumerate() {
        let t = (i as f32) / 48000.0;
        *sample = (2.0 * std::f32::consts::PI * freq * t).sin() * 0.8;
    }

    let mut opus_output = vec![0u8; 1024];
    let encoded_len = encoder
        .encode(&pcm_frame, frame_size, &mut opus_output)
        .expect("Opus encoding failed");

    assert!(encoded_len > 0, "Encoded Opus frame should not be empty");
    assert!(encoded_len < 1024, "Encoded Opus frame within buffer bounds");
}

#[test]
fn test_netease_id_parsing() {
    use svc_music_player::audio::resolver::MediaResolver;

    assert_eq!(MediaResolver::parse_netease_id("33894312"), Some(33894312));
    assert_eq!(MediaResolver::parse_netease_id("186016"), Some(186016));
    assert_eq!(MediaResolver::parse_netease_id("163: 186016"), Some(186016));
    assert_eq!(MediaResolver::parse_netease_id("wy:33894312"), Some(33894312));
    assert_eq!(MediaResolver::parse_netease_id("netmusic: 444269135"), Some(444269135));
    assert_eq!(
        MediaResolver::parse_netease_id("https://music.163.com/#/song?id=33894312"),
        Some(33894312)
    );
    assert_eq!(
        MediaResolver::parse_netease_id("https://music.163.com/song?id=186016&userid=123"),
        Some(186016)
    );
    assert_eq!(MediaResolver::parse_netease_id("not_a_song_id"), None);
}

#[test]
fn test_bilibili_bv_normalization() {
    use svc_music_player::audio::resolver::MediaResolver;

    assert_eq!(
        MediaResolver::normalize_bilibili("BV1xx411c7mD"),
        Some("https://www.bilibili.com/video/BV1xx411c7mD".to_string())
    );
    assert_eq!(
        MediaResolver::normalize_bilibili("bv17x411w7KC"),
        Some("https://www.bilibili.com/video/bv17x411w7KC".to_string())
    );
    assert_eq!(
        MediaResolver::normalize_bilibili("av170001"),
        Some("https://www.bilibili.com/video/av170001".to_string())
    );
    assert_eq!(
        MediaResolver::normalize_bilibili("https://www.bilibili.com/video/BV1xx411c7mD"),
        None
    );
}

#[test]
fn test_niconico_normalization() {
    use svc_music_player::audio::resolver::MediaResolver;

    // 1. 直传视频 ID
    assert_eq!(
        MediaResolver::normalize_niconico("sm46865356"),
        Some("https://www.nicovideo.jp/watch/sm46865356".to_string())
    );
    assert_eq!(
        MediaResolver::normalize_niconico("SM9"),
        Some("https://www.nicovideo.jp/watch/sm9".to_string())
    );
    assert_eq!(
        MediaResolver::normalize_niconico("so123456"),
        Some("https://www.nicovideo.jp/watch/so123456".to_string())
    );
    assert_eq!(
        MediaResolver::normalize_niconico("nm654321"),
        Some("https://www.nicovideo.jp/watch/nm654321".to_string())
    );

    // 2. 快捷前缀
    assert_eq!(
        MediaResolver::normalize_niconico("nico:sm46865356"),
        Some("https://www.nicovideo.jp/watch/sm46865356".to_string())
    );
    assert_eq!(
        MediaResolver::normalize_niconico("nico:46865356"),
        Some("https://www.nicovideo.jp/watch/sm46865356".to_string())
    );

    // 3. nico.ms 短链接
    assert_eq!(
        MediaResolver::normalize_niconico("https://nico.ms/sm46865356"),
        Some("https://www.nicovideo.jp/watch/sm46865356".to_string())
    );
    assert_eq!(
        MediaResolver::normalize_niconico("nico.ms/sm9"),
        Some("https://www.nicovideo.jp/watch/sm9".to_string())
    );

    // 4. www.video.nicovideo.jp / video.nicovideo.jp 等非标子域名 (修复 yt-dlp Unsupported URL)
    assert_eq!(
        MediaResolver::normalize_niconico("https://www.video.nicovideo.jp/watch/sm46865356"),
        Some("https://www.nicovideo.jp/watch/sm46865356".to_string())
    );
    assert_eq!(
        MediaResolver::normalize_niconico("https://video.nicovideo.jp/watch/sm46865356?ref=search_key_video"),
        Some("https://www.nicovideo.jp/watch/sm46865356".to_string())
    );
    assert_eq!(
        MediaResolver::normalize_niconico("https://sp.nicovideo.jp/watch/sm46865356"),
        Some("https://www.nicovideo.jp/watch/sm46865356".to_string())
    );
    assert_eq!(
        MediaResolver::normalize_niconico("https://www.nicovideo.jp/watch/sm46865356"),
        Some("https://www.nicovideo.jp/watch/sm46865356".to_string())
    );

    // 5. 非 Niconico 目标
    assert_eq!(MediaResolver::normalize_niconico("BV1xx411c7mD"), None);
    assert_eq!(MediaResolver::normalize_niconico("https://www.youtube.com/watch?v=123"), None);
    assert_eq!(MediaResolver::normalize_niconico("smooth_criminal.mp3"), None);
}

#[test]
fn test_chat_command_extraction() {
    use svc_music_player::mc::chat_handler::{extract_command, strip_mc_formatting};

    // 1. 测试样式代码清洗
    let colored = "§7[§a生存服§7] §fAlice §8» §f!play 186016";
    assert_eq!(strip_mc_formatting(colored), "[生存服] Alice » !play 186016");

    // 2. 测试纯原生指令
    let (sender, cmd) = extract_command("!play 186016", "!").unwrap();
    assert_eq!(cmd, "play 186016");
    assert_eq!(sender, None);

    // 3. 测试原版聊天格式 <Alice> !play 186016
    let (sender, cmd) = extract_command("<Alice> !play 186016", "!").unwrap();
    assert_eq!(cmd, "play 186016");
    assert_eq!(sender, Some("Alice".to_string()));

    // 4. 测试 CMI 常用箭头格式带颜色
    let (sender, cmd) = extract_command("§7[§a生存服§7] §fAlice §8» §f!play 186016", "!").unwrap();
    assert_eq!(cmd, "play 186016");
    assert_eq!(sender, Some("Alice".to_string()));

    // 5. 测试冒号格式 [VIP] Bob: !vol 50
    let (sender, cmd) = extract_command("[VIP] Bob: !vol 50", "!").unwrap();
    assert_eq!(cmd, "vol 50");
    assert_eq!(sender, Some("Bob".to_string()));

    // 6. 测试无空格冒号 Charlie:!pause
    let (sender, cmd) = extract_command("Charlie:!pause", "!").unwrap();
    assert_eq!(cmd, "pause");
    assert_eq!(sender, Some("Charlie".to_string()));

    // 7. 测试带空格冒号与中文头衔
    let (sender, cmd) = extract_command("[世界公屏] Dave : !skip", "!").unwrap();
    assert_eq!(cmd, "skip");
    assert_eq!(sender, Some("Dave".to_string()));

    // 8. 非指令普通发言不应被误判
    assert_eq!(extract_command("Hello world! How are you?", "!"), None);
    assert_eq!(extract_command("<Bob> I love this!play later", "!"), None);

    // 9. 过滤机器人自身的 [Bot] 提示与回环消息
    let bot_help = "doveoverthere: [Bot] 指令列表: !play <链接>, !pause, !resume, !skip, !stop, !vol <0-100>, !tpaccept, !group";
    assert_eq!(extract_command(bot_help, "!"), None);
    assert_eq!(extract_command("[Bot] 指令列表: !play <链接>, !pause", "!"), None);
    assert_eq!(extract_command("<doveoverthere> [Bot] 用法: !play <链接>", "!"), None);
    assert_eq!(extract_command("doveoverthere: [Bot] 当前未加入任何语音群组。可用: !group join <群名> [密码]", "!"), None);
    assert_eq!(extract_command("§7[§a生存服§7] §fdoveoverthere §8» §f[Bot] 提示: !vol 50", "!"), None);

    // 10. 玩家点播包含 [Bot] 字样的歌曲名不应被误拦截
    let (sender, cmd) = extract_command("<Alice> !play [Bot] Bad Apple", "!").unwrap();
    assert_eq!(cmd, "play [Bot] Bad Apple");
    assert_eq!(sender, Some("Alice".to_string()));

    // 11. ChatHandler::is_bot_message 测试
    use svc_music_player::mc::chat_handler::ChatHandler;
    assert!(ChatHandler::is_bot_message(bot_help, "doveoverthere"));
    assert!(ChatHandler::is_bot_message("[Bot] 正在解析链接，请稍候...", "doveoverthere"));
    assert!(ChatHandler::is_bot_message("<doveoverthere> test message", "doveoverthere"));
    assert!(ChatHandler::is_bot_message("[VIP] doveoverthere: hello", "doveoverthere"));
    assert!(!ChatHandler::is_bot_message("<Alice> !play 186016", "doveoverthere"));
    assert!(!ChatHandler::is_bot_message("Alice: !play [Bot] Bad Apple", "doveoverthere"));
}

#[test]
fn test_resolve_voice_address() {
    use svc_music_player::mc::session::resolve_voice_address;

    // 1. 服务端下发的 host 已经包含端口（用户日志中的真实情况）
    let addr = resolve_voice_address("", 0, "mc.vlssu.cn:45114", 45114, "warma.fans");
    assert_eq!(addr, "mc.vlssu.cn:45114");

    // 2. 服务端下发的 host 是纯域名，未带端口
    let addr = resolve_voice_address("", 0, "voice.example.com", 24454, "warma.fans");
    assert_eq!(addr, "voice.example.com:24454");

    // 3. 服务端下发的 host 为空，使用 MC 服务器域名
    let addr = resolve_voice_address("", 0, "", 45114, "warma.fans");
    assert_eq!(addr, "warma.fans:45114");

    // 4. 用户指定了 override_host 与 override_port
    let addr = resolve_voice_address("1.2.3.4", 24454, "mc.vlssu.cn:45114", 45114, "warma.fans");
    assert_eq!(addr, "1.2.3.4:24454");

    // 5. 用户指定了 override_host 带端口
    let addr = resolve_voice_address("proxy.node.com:12345", 0, "mc.vlssu.cn:45114", 45114, "warma.fans");
    assert_eq!(addr, "proxy.node.com:12345");

    // 6. IP 地址格式
    let addr = resolve_voice_address("", 0, "192.168.1.100:24454", 24454, "warma.fans");
    assert_eq!(addr, "192.168.1.100:24454");
}

#[test]
fn test_server_to_client_packet_unpacking() {
    let secret_key = [0x55u8; 16];
    let crypto = VoiceCrypto::new(&secret_key);

    // 服务端发送 AuthenticateAckPacket (0x06，无负载)
    let encrypted = crypto.encrypt(PKT_AUTHENTICATE_ACK, &[]).expect("Encrypt failed");

    // 官方服务端数据包格式：[0xFF (MAGIC)] + [VarInt(len)] + [Encrypted]
    let mut server_packet = Vec::new();
    server_packet.push(MAGIC_BYTE);
    write_varint(encrypted.len() as i32, &mut server_packet).expect("VarInt failed");
    server_packet.extend_from_slice(&encrypted);

    // 验证客户端解包逻辑（偏移 1 开始读取 VarInt 长度）
    assert_eq!(server_packet[0], MAGIC_BYTE);
    let mut cursor = Cursor::new(&server_packet[1..]);
    let payload_len = read_varint(&mut cursor).expect("Read VarInt failed") as usize;
    let varint_size = cursor.position() as usize;

    let payload_start = 1 + varint_size;
    let payload_end = payload_start + payload_len;
    assert_eq!(payload_end, server_packet.len());

    let (pkt_type, body) = crypto.decrypt(&server_packet[payload_start..payload_end]).expect("Decrypt failed");
    assert_eq!(pkt_type, PKT_AUTHENTICATE_ACK);
    assert!(body.is_empty());
}

#[test]
fn test_cmi_tpa_detection() {
    let dummy_component = FormattedText::from("test");

    // 1. CMI 中文经典提示
    let raw = "§6[CMI] §e玩家 §aAlice §e请求传送到你这里。";
    let cmd = ChatHandler::detect_tpa_request(&dummy_component, raw, "doveoverthere");
    assert_eq!(cmd, Some("/cmi tpaccept".to_string()));

    // 2. 带有接受指令提示
    let raw2 = "§e输入 §b/tpaccept §e接受传送，或者输入 §c/tpdeny §e拒绝。";
    let cmd2 = ChatHandler::detect_tpa_request(&dummy_component, raw2, "doveoverthere");
    assert_eq!(cmd2, Some("/cmi tpaccept".to_string()));

    // 3. 英文 CMI 提示
    let raw3 = "Bob has requested to teleport to you.";
    let cmd3 = ChatHandler::detect_tpa_request(&dummy_component, raw3, "doveoverthere");
    assert_eq!(cmd3, Some("/cmi tpaccept".to_string()));

    // 4. 机器人自身消息被忽略，防止自循环
    let raw_bot = "[Bot] 点歌失败: yt-dlp 解析链接失败";
    let cmd_bot = ChatHandler::detect_tpa_request(&dummy_component, raw_bot, "doveoverthere");
    assert_eq!(cmd_bot, None);

    // 5. 交互式 ClickEvent 组件测试
    let mut comp = FormattedText::Text(azalea_chat::text_component::TextComponent::new("玩家 Alice 请求传送 [接受]"));
    comp.get_base_mut().style.click_event = Some(azalea_chat::click_event::ClickEvent::RunCommand {
        command: "/cmi tpaccept Alice".to_string(),
    });
    let cmd_click = ChatHandler::detect_tpa_request(&comp, "玩家 Alice 请求传送 [接受]", "doveoverthere");
    assert_eq!(cmd_click, Some("/cmi tpaccept Alice".to_string()));
}

#[test]
fn test_chat_link_placeholder_and_extraction() {
    use svc_music_player::mc::chat_handler::{extract_chat_text_with_links, is_link_placeholder};

    // 1. 测试 is_link_placeholder
    assert!(is_link_placeholder("[链接]"));
    assert!(is_link_placeholder("【链接】"));
    assert!(is_link_placeholder("[link]"));
    assert!(is_link_placeholder("[Link]"));
    assert!(is_link_placeholder("[url]"));
    assert!(is_link_placeholder("[URL]"));
    assert!(is_link_placeholder("链接"));
    assert!(is_link_placeholder("link"));
    assert!(is_link_placeholder("§a[链接]§r"));
    assert!(!is_link_placeholder("https://www.youtube.com"));
    assert!(!is_link_placeholder("Alice"));

    // 2. 测试简单组件中的链接还原
    let mut root = FormattedText::Text(azalea_chat::text_component::TextComponent::new("Alice: !play "));
    let mut link_comp = FormattedText::Text(azalea_chat::text_component::TextComponent::new("[链接]"));
    link_comp.get_base_mut().style.click_event = Some(azalea_chat::click_event::ClickEvent::OpenUrl {
        url: "https://www.youtube.com/watch?v=MlTQUSFr-gw".to_string(),
    });
    root.get_base_mut().siblings.push(link_comp);

    let extracted = extract_chat_text_with_links(&root);
    assert_eq!(extracted, "Alice: !play https://www.youtube.com/watch?v=MlTQUSFr-gw");

    // 3. 测试后随文本的兄弟节点还原
    let mut root2 = FormattedText::Text(azalea_chat::text_component::TextComponent::new("<Bob> !play "));
    let mut link_comp2 = FormattedText::Text(azalea_chat::text_component::TextComponent::new("【链接】"));
    link_comp2.get_base_mut().style.click_event = Some(azalea_chat::click_event::ClickEvent::OpenUrl {
        url: "https://www.bilibili.com/video/BV1xx411c79H".to_string(),
    });
    let tail_comp = FormattedText::Text(azalea_chat::text_component::TextComponent::new(" 很好听"));
    root2.get_base_mut().siblings.push(link_comp2);
    root2.get_base_mut().siblings.push(tail_comp);

    let extracted2 = extract_chat_text_with_links(&root2);
    assert_eq!(extracted2, "<Bob> !play https://www.bilibili.com/video/BV1xx411c79H 很好听");
}

#[tokio::test]
async fn test_symphonia_decode_cached_media() {
    use svc_music_player::audio::decoder::AudioDecoder;
    use std::path::Path;

    // 测试 YouTube 缓存 (.m4a - AAC)
    let m4a_path = "cache/MlTQUSFr-gw.m4a";
    if Path::new(m4a_path).is_file() {
        let mut dec = AudioDecoder::open(m4a_path, true, None, None)
            .await
            .expect("Failed to open cached m4a");
        let samples = dec.read_samples(960).await.expect("Failed to read samples");
        assert!(samples.is_some(), "Should read 960 samples from m4a");
        assert_eq!(samples.unwrap().len(), 960);
    }

    // 测试 Niconico 缓存 (.mp4 - AAC)
    for mp4_path in &["cache/sm9.mp4", "cache/sm46865356.mp4"] {
        if Path::new(mp4_path).is_file() {
            let mut dec = AudioDecoder::open(mp4_path, true, None, None)
                .await
                .expect("Failed to open cached mp4");
            let samples = dec.read_samples(960).await.expect("Failed to read samples");
            assert!(samples.is_some(), "Should read 960 samples from mp4");
            assert_eq!(samples.unwrap().len(), 960);
        }
    }
}

#[tokio::test]
async fn test_ffmpeg_decode_pipeline() {
    use svc_music_player::audio::decoder::AudioDecoder;
    use std::path::Path;

    let ffmpeg_candidates = ["bin/ffmpeg.exe", "dist/windows-x86_64/bin/ffmpeg.exe"];
    let ffmpeg_found = ffmpeg_candidates.iter().find(|p| Path::new(p).is_file());

    if let Some(&ffmpeg_path_str) = ffmpeg_found {
        let ffmpeg_path = Path::new(ffmpeg_path_str);
        let m4a_path = "cache/MlTQUSFr-gw.m4a";
        if Path::new(m4a_path).is_file() {
            let mut dec = AudioDecoder::open(m4a_path, true, None, Some(ffmpeg_path))
                .await
                .expect("Failed to open m4a with FFmpeg");
            let samples = dec.read_samples(960).await.expect("Failed to read samples from FFmpeg");
            assert!(samples.is_some(), "Should read 960 samples from FFmpeg");
            assert_eq!(samples.unwrap().len(), 960);
        }
    }
}

#[test]
fn test_svc_group_packets() {
    // 1. 测试 ClientGroup 解包 (voicechat:add_group)
    let group_id = Uuid::new_v4();
    let mut add_group_raw = Vec::new();
    add_group_raw.extend_from_slice(group_id.as_bytes());
    // write name "Music"
    let _ = write_mc_string("Music", &mut add_group_raw);
    add_group_raw.push(0); // hasPassword = false
    add_group_raw.push(1); // persistent = true
    add_group_raw.push(0); // hidden = false
    add_group_raw.extend_from_slice(&1i16.to_be_bytes()); // group_type = 1 (Open)

    let parsed_group = SvcGroup::from_bytes(&add_group_raw).expect("Parse SvcGroup failed");
    assert_eq!(parsed_group.id, group_id);
    assert_eq!(parsed_group.name, "Music");
    assert_eq!(parsed_group.has_password, false);
    assert_eq!(parsed_group.persistent, true);
    assert_eq!(parsed_group.hidden, false);
    assert_eq!(parsed_group.group_type, 1);

    // 2. 测试 create_group_payload 构造
    let create_payload = create_group_payload("Music", None, 1).expect("Create payload failed");
    assert!(!create_payload.is_empty());

    // 3. 测试 join_group_payload 构造
    let join_payload = join_group_payload(group_id, None).expect("Join payload failed");
    assert_eq!(&join_payload[0..16], group_id.as_bytes());
    assert_eq!(join_payload[16], 0); // has_password = false

    // 4. 测试 joined_group 解包
    let mut joined_raw = Vec::new();
    joined_raw.push(1); // has_group = true
    joined_raw.extend_from_slice(group_id.as_bytes());
    joined_raw.push(0); // wrong_password = false

    let (joined_id, wrong_pwd) = parse_joined_group_packet(&joined_raw).expect("Parse joined failed");
    assert_eq!(joined_id, Some(group_id));
    assert_eq!(wrong_pwd, false);

    // 5. 测试 remove_group 解包
    let remove_raw = group_id.as_bytes().to_vec();
    let removed_id = parse_remove_group_packet(&remove_raw).expect("Parse remove failed");
    assert_eq!(removed_id, group_id);
}

#[test]
fn test_decode_output_string_gbk_fallback() {
    use svc_music_player::audio::resolver::decode_output_string;

    // 1. 标准 UTF-8 字符串
    let utf8_text = "【东方】Bad Apple!! ＰＶ【影絵】";
    assert_eq!(decode_output_string(utf8_text.as_bytes()), utf8_text);

    // 2. Windows CP936 / GBK 编码的中文字符串，必须能自动回退并正确解码为 UTF-8
    let (gbk_bytes, _, _) = encoding_rs::GBK.encode(utf8_text);
    let decoded = decode_output_string(&gbk_bytes);
    assert_eq!(decoded, utf8_text);

    // 3. 英文纯 ASCII
    let ascii_text = "Never Gonna Give You Up - Rick Astley";
    assert_eq!(decode_output_string(ascii_text.as_bytes()), ascii_text);
}

#[test]
fn test_audio_headers_builder() {
    use svc_music_player::audio::decoder::AudioDecoder;

    // 1. Bilibili CDN 直链自动补全 Referer 与 User-Agent
    let bili_stream = "https://cn-sccd-ct-02-16.bilivideo.com/upgcxcode/23/47/3724723/3724723.m4s?e=xxx";
    let (ffmpeg_headers, header_map) = AudioDecoder::build_headers(bili_stream, None);
    assert!(ffmpeg_headers.contains("Referer: https://www.bilibili.com/"));
    assert!(ffmpeg_headers.contains("User-Agent:"));
    assert_eq!(header_map.get("Referer"), Some(&"https://www.bilibili.com/".to_string()));

    // 2. 网易云音乐直链自动补全 Referer
    let netease_stream = "https://music.163.com/song/media/outer/url?id=186016.mp3";
    let (ffmpeg_headers, header_map) = AudioDecoder::build_headers(netease_stream, None);
    assert!(ffmpeg_headers.contains("Referer: http://music.163.com/"));
    assert_eq!(header_map.get("Referer"), Some(&"http://music.163.com/".to_string()));

    // 3. 用户传入自定义 Header
    let mut custom = std::collections::HashMap::new();
    custom.insert("Authorization".to_string(), "Bearer token123".to_string());
    let (ffmpeg_headers, header_map) = AudioDecoder::build_headers(bili_stream, Some(&custom));
    assert!(ffmpeg_headers.contains("Authorization: Bearer token123"));
    assert_eq!(header_map.get("Authorization"), Some(&"Bearer token123".to_string()));
}

#[test]
fn test_find_local_file() {
    use svc_music_player::audio::resolver::MediaResolver;

    // 创建测试文件
    let test_dir = std::path::Path::new("music");
    let _ = std::fs::create_dir_all(test_dir);
    let dummy_file = test_dir.join("test_song.mp3");
    let _ = std::fs::write(&dummy_file, b"dummy audio content");

    // 1. 直接通过 music/test_song.mp3 检索
    let res = MediaResolver::find_local_file("music/test_song.mp3");
    assert!(res.is_some());
    let (_path, title) = res.unwrap();
    assert_eq!(title, "test_song");

    // 2. 在 music/ 目录下自动根据文件名检索
    let res2 = MediaResolver::find_local_file("test_song.mp3");
    assert!(res2.is_some());
    let (_, title2) = res2.unwrap();
    assert_eq!(title2, "test_song");

    // 3. 省略后缀检索
    let res3 = MediaResolver::find_local_file("test_song");
    assert!(res3.is_some());
    let (_, title3) = res3.unwrap();
    assert_eq!(title3, "test_song");

    // 4. 带引号检索
    let res4 = MediaResolver::find_local_file("\"test_song\"");
    assert!(res4.is_some());

    // 5. 不存在的文件应返回 None
    assert!(MediaResolver::find_local_file("non_existent_file_xyz").is_none());

    // 清理测试文件
    let _ = std::fs::remove_file(dummy_file);
}

#[test]
fn test_bitrate_calculation_logic() {
    // 20ms = 0.02s，50 帧对应 1 秒
    // 假设每包平均 240 字节 (对应 96 kbps 目标)
    let window_bytes = 240 * 50;
    let window_frames = 50;
    let kbps = (window_bytes as f64 * 8.0) / (window_frames as f64 * 0.02 * 1000.0);
    assert!((kbps - 96.0).abs() < 1e-6);

    // 测试 5 秒统计窗口 (250 帧)
    let window_bytes_5s = 200 * 250;
    let window_frames_5s = 250;
    let kbps_5s = (window_bytes_5s as f64 * 8.0) / (window_frames_5s as f64 * 0.02 * 1000.0);
    assert!((kbps_5s - 80.0).abs() < 1e-6);
}

#[tokio::test]
async fn test_niconico_resolve_with_new_subdomain() {
    use svc_music_player::audio::resolver::MediaResolver;
    use std::path::Path;

    let ytdlp_candidates = ["bin/yt-dlp.exe", "dist/windows-x86_64/bin/yt-dlp.exe"];
    let ytdlp_found = ytdlp_candidates.iter().find(|p| Path::new(p).is_file());
    let ffmpeg_candidates = ["bin/ffmpeg.exe", "dist/windows-x86_64/bin/ffmpeg.exe"];
    let ffmpeg_found = ffmpeg_candidates.iter().find(|p| Path::new(p).is_file());

    if let (Some(&ytdlp_str), Some(&ffmpeg_str)) = (ytdlp_found, ffmpeg_found) {
        match MediaResolver::resolve(
            "https://www.video.nicovideo.jp/watch/sm46865356",
            Some(Path::new(ytdlp_str)),
            Some(Path::new(ffmpeg_str)),
        )
        .await {
            Ok(res) => {
                assert!(res.is_local_file);
                assert!(res.source_url_or_path.contains("sm46865356"));
                assert!(res.title.contains("初音ミク") || res.title.contains("えねみぃ"));
            }
            Err(e) => {
                eprintln!("[Test] Niconico 外部网络请求受限或发生网络错误 (跳过): {:?}", e);
            }
        }
    }
}

#[tokio::test]
async fn test_playlist_modes_and_operations() {
    use svc_music_player::audio::playlist::{PlayMode, PlaylistManager, RequestOrigin, TrackInfo};
    use std::path::PathBuf;
    use uuid::Uuid;

    let test_file = PathBuf::from("target/test_proto_playlist.json");
    let _ = std::fs::remove_file(&test_file);

    let mut pl = PlaylistManager::new(test_file.clone());

    let mk_track = |title: &str| TrackInfo {
        id: Uuid::new_v4(),
        title: title.to_string(),
        raw_query: title.to_string(),
        source: format!("http://test/{}", title),
        is_local_file: false,
        http_headers: None,
        source_bitrate_kbps: Some(320.0),
        duration_secs: Some(200),
        replay_gain_db: None,
        origin: RequestOrigin::WebConsole,
    };

    // Add tracks
    let idx0 = pl.add_track(mk_track("Track 1"));
    let idx1 = pl.add_track(mk_track("Track 2"));
    let idx2 = pl.add_track(mk_track("Track 3"));
    assert_eq!(idx0, 0);
    assert_eq!(idx1, 1);
    assert_eq!(idx2, 2);
    assert_eq!(pl.current_index, Some(0));

    // Test ListLoop
    pl.set_mode(PlayMode::ListLoop);
    assert_eq!(pl.advance_next(false).unwrap().title, "Track 2");
    assert_eq!(pl.advance_next(false).unwrap().title, "Track 3");
    assert_eq!(pl.advance_next(false).unwrap().title, "Track 1"); // wrapped around

    // Test SingleLoop
    pl.set_mode(PlayMode::SingleLoop);
    assert_eq!(pl.advance_next(false).unwrap().title, "Track 1"); // auto repeat
    assert_eq!(pl.advance_next(true).unwrap().title, "Track 2"); // manual skip to next

    // Test Sequential
    pl.set_mode(PlayMode::Sequential);
    assert_eq!(pl.advance_next(false).unwrap().title, "Track 3");
    assert!(pl.advance_next(false).is_none()); // reached end

    // Test Insert Next
    pl.jump_to(0);
    pl.insert_next(mk_track("Track 1.5"));
    assert_eq!(pl.tracks[1].title, "Track 1.5");

    // Test Move to Next (Pin/Top)
    let moved_pos = pl.move_to_next(3).unwrap(); // Move Track 3 to next
    assert_eq!(moved_pos, 1);
    assert_eq!(pl.tracks[1].title, "Track 3");

    // Test Remove Track with index fix
    let (removed, was_cur) = pl.remove_track(0).unwrap();
    assert_eq!(removed.title, "Track 1");
    assert!(was_cur);
    assert_eq!(pl.current_index, Some(0));

    let _ = std::fs::remove_file(&test_file);
}

#[test]
fn test_loudness_replaygain_and_limiter() {
    use svc_music_player::audio::loudness::LoudnessProcessor;

    // 1. Tag parsing
    assert_eq!(LoudnessProcessor::parse_replaygain_tag("-3.21 dB"), Some(-3.21));
    assert_eq!(LoudnessProcessor::parse_replaygain_tag("+1.5 db"), Some(1.5));
    assert_eq!(LoudnessProcessor::parse_replaygain_tag(" -14.2 LUFS "), Some(-14.2));

    // 2. Soft clip limiter boundary: guarantees values never exceed ±0.98
    for &val in &[-100.0, -10.0, -2.0, -0.99, -0.85, 0.0, 0.5, 0.85, 0.99, 2.0, 10.0, 100.0] {
        let clipped = LoudnessProcessor::soft_clip_sample(val);
        assert!(clipped >= -0.98 && clipped <= 0.98);
        if val.abs() <= 0.85 {
            assert!((clipped - val).abs() < 1e-6);
        }
    }

    // 3. Static gain analysis
    let silence = vec![0i16; 4800];
    assert!(LoudnessProcessor::analyze_static_gain_db(&silence, -16.0).is_none());

    let mut signal = vec![0i16; 9600];
    for i in 0..signal.len() {
        signal[i] = (20000.0 * (2.0 * std::f32::consts::PI * 1000.0 * (i as f32 / 48000.0)).sin()) as i16;
    }
    let gain = LoudnessProcessor::analyze_static_gain_db(&signal, -16.0);
    assert!(gain.is_some());
    let g = gain.unwrap();
    assert!(g >= -12.0 && g <= 12.0);

    // 4. Frame processing
    let mut proc = LoudnessProcessor::new(-16.0, Some(-2.5));
    let mut out = vec![0.0f32; 960];
    let applied = proc.process_frame(&signal[..960], &mut out, true, 0.8);
    assert_eq!(applied, -2.5);
    for &sample in &out {
        assert!(sample >= -0.98 && sample <= 0.98);
    }
}

#[test]
fn test_extended_chat_commands() {
    use svc_music_player::audio::playlist::PlayMode;
    use svc_music_player::mc::chat_handler::extract_command;

    // Commands extraction with prefix "!"
    let (_, cmd) = extract_command("!play song1", "!").unwrap();
    assert_eq!(cmd, "play song1");

    let (_, cmd) = extract_command("<Alice> !playnow https://bilibili.com/video/BV1xx", "!").unwrap();
    assert_eq!(cmd, "playnow https://bilibili.com/video/BV1xx");

    let (_, cmd) = extract_command("Bob: !pn https://music.163.com/song?id=123", "!").unwrap();
    assert_eq!(cmd, "pn https://music.163.com/song?id=123");

    let (_, cmd) = extract_command("!mode shuffle", "!").unwrap();
    assert_eq!(cmd, "mode shuffle");

    let (_, cmd) = extract_command("!loop", "!").unwrap();
    assert_eq!(cmd, "loop");

    let (_, cmd) = extract_command("!single", "!").unwrap();
    assert_eq!(cmd, "single");

    let (_, cmd) = extract_command("!shuffle", "!").unwrap();
    assert_eq!(cmd, "shuffle");

    let (_, cmd) = extract_command("!seq", "!").unwrap();
    assert_eq!(cmd, "seq");

    let (_, cmd) = extract_command("!list 2", "!").unwrap();
    assert_eq!(cmd, "list 2");

    let (_, cmd) = extract_command("!queue", "!").unwrap();
    assert_eq!(cmd, "queue");

    let (_, cmd) = extract_command("!pl", "!").unwrap();
    assert_eq!(cmd, "pl");

    let (_, cmd) = extract_command("!jump 3", "!").unwrap();
    assert_eq!(cmd, "jump 3");

    let (_, cmd) = extract_command("!top 5", "!").unwrap();
    assert_eq!(cmd, "top 5");

    let (_, cmd) = extract_command("!rm 2", "!").unwrap();
    assert_eq!(cmd, "rm 2");

    let (_, cmd) = extract_command("!clear", "!").unwrap();
    assert_eq!(cmd, "clear");

    let (_, cmd) = extract_command("!prev", "!").unwrap();
    assert_eq!(cmd, "prev");

    let (_, cmd) = extract_command("!skip", "!").unwrap();
    assert_eq!(cmd, "skip");

    let (_, cmd) = extract_command("!rg on", "!").unwrap();
    assert_eq!(cmd, "rg on");

    let (_, cmd) = extract_command("!rg off", "!").unwrap();
    assert_eq!(cmd, "rg off");

    // PlayMode::from_input parsing
    assert_eq!(PlayMode::from_input("loop"), Some(PlayMode::ListLoop));
    assert_eq!(PlayMode::from_input("列表循环"), Some(PlayMode::ListLoop));
    assert_eq!(PlayMode::from_input("single"), Some(PlayMode::SingleLoop));
    assert_eq!(PlayMode::from_input("单曲循环"), Some(PlayMode::SingleLoop));
    assert_eq!(PlayMode::from_input("shuffle"), Some(PlayMode::Shuffle));
    assert_eq!(PlayMode::from_input("random"), Some(PlayMode::Shuffle));
    assert_eq!(PlayMode::from_input("随机"), Some(PlayMode::Shuffle));
    assert_eq!(PlayMode::from_input("seq"), Some(PlayMode::Sequential));
    assert_eq!(PlayMode::from_input("顺序"), Some(PlayMode::Sequential));
    assert_eq!(PlayMode::from_input("invalid"), None);
}






