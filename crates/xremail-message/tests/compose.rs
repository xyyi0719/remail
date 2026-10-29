//! `xremail-message` 的**发信构建**测试。
//!
//! 测试预期来自 **RFC 5322 / 2045 / 2046 / 2047 的语义与安全要求**，
//! 不来自实现本身（见 [AI 开发工作规范 §1.3](../../../docs/AI_DEVELOPMENT_GUIDE.md)）。
//!
//! 覆盖的要点：
//! * **头部注入防护**：任何会进邮件头的字段里带 CR/LF 都必须被拒绝，
//!   且**不产出任何字节**——这是发信路径上最致命的漏洞。
//! * **密送不泄漏**：Bcc 地址只能出现在 envelope 里，绝不能出现在邮件字节里。
//! * **信封收件人**：to + cc + bcc 去重去空。
//! * **RFC 2047 往返**：非 ASCII 的标题与显示名编码后能用
//!   `decode_encoded_words` 解回原文。
//! * **正文形态**：纯文本 / HTML / alternative / mixed 四种结构。
//! * **传输编码**：7bit 走 quoted-printable 且**整封邮件是纯 ASCII**，
//!   8bit 原样透传，附件一律 base64。
//! * **必需头字段**与 **CRLF 行尾**。
//! * **Date 格式**与 **Message-ID 唯一性**。

use xremail_message::compose::{
    BuildOptions, BuiltMessage, ComposeError, OutgoingAttachment, OutgoingMessage, build_message,
};
use xremail_message::encoded_word::decode_encoded_words;
use xremail_message::transfer::{decode_quoted_printable, encode_quoted_printable};

// ===========================================================================
// 辅助
// ===========================================================================

/// 一个头部注入用例：字段名 + 往邮件里塞 CR/LF 的改写函数。
type InjectionCase = (&'static str, Box<dyn Fn(&mut OutgoingMessage)>);

/// 一封最小的可发送邮件。
fn sample_message() -> OutgoingMessage {
    OutgoingMessage {
        from: "sender@example.com".to_string(),
        to: vec!["alice@example.com".to_string()],
        cc: Vec::new(),
        bcc: Vec::new(),
        subject: "Hello".to_string(),
        body_text: Some("plain body".to_string()),
        body_html: None,
        attachments: Vec::new(),
        in_reply_to: None,
        references: None,
    }
}

/// 默认构建选项（7bit、2026-10-28 02:30:19 UTC）。
fn sample_options() -> BuildOptions {
    BuildOptions {
        eight_bit: false,
        now_epoch_secs: 1_793_154_619,
        message_id_domain: "xremail.local".to_string(),
        charset: "utf-8".to_string(),
    }
}

/// 把邮件字节按 UTF-8 读成字符串（便于断言）。
fn as_text(built: &BuiltMessage) -> String {
    String::from_utf8_lossy(&built.bytes).into_owned()
}

/// 取出一个头字段的值（只看第一行——本模块只在超长时才折行）。
fn header_value(built: &BuiltMessage, name: &str) -> Option<String> {
    let prefix = format!("{name}: ");
    as_text(built)
        .split("\r\n")
        .find_map(|line| line.strip_prefix(&prefix).map(str::to_string))
}

/// 断言整封邮件里没有裸 LF（每行都以 CRLF 结束）。
fn assert_crlf_line_endings(bytes: &[u8]) {
    for (index, byte) in bytes.iter().enumerate() {
        if *byte == b'\n' {
            assert!(
                index > 0 && bytes[index - 1] == b'\r',
                "第 {index} 个字节是裸 LF，行尾必须是 CRLF"
            );
        }
    }
}

// ===========================================================================
// 1. 头部注入防护
// ===========================================================================

#[test]
fn subject_with_crlf_is_rejected() {
    let mut message = sample_message();
    // 经典注入：标题里塞一个换行 + 一个伪造的 Bcc 头。
    message.subject = "Hello\r\nBcc: evil@example.com".to_string();

    let error = build_message(&message, &sample_options()).expect_err("必须拒绝头部注入");

    assert!(
        matches!(error, ComposeError::HeaderInjection { field: "subject" }),
        "实际错误：{error}"
    );
}

#[test]
fn injection_is_rejected_in_every_header_field() {
    let cases: Vec<InjectionCase> = vec![
        (
            "from",
            Box::new(|m: &mut OutgoingMessage| {
                m.from = "a@example.com\r\nBcc: evil@example.com".to_string();
            }),
        ),
        (
            "to",
            Box::new(|m: &mut OutgoingMessage| {
                m.to = vec!["a@example.com\nBcc: evil@example.com".to_string()];
            }),
        ),
        (
            "cc",
            Box::new(|m: &mut OutgoingMessage| {
                m.cc = vec!["a@example.com\r\nX-Evil: 1".to_string()];
            }),
        ),
        (
            "bcc",
            Box::new(|m: &mut OutgoingMessage| {
                m.bcc = vec!["a@example.com\r\nX-Evil: 1".to_string()];
            }),
        ),
        (
            "in_reply_to",
            Box::new(|m: &mut OutgoingMessage| {
                m.in_reply_to = Some("<a@b>\r\nX-Evil: 1".to_string());
            }),
        ),
        (
            "references",
            Box::new(|m: &mut OutgoingMessage| {
                m.references = Some("<a@b>\nX-Evil: 1".to_string());
            }),
        ),
        (
            "filename",
            Box::new(|m: &mut OutgoingMessage| {
                m.attachments.push(OutgoingAttachment {
                    filename: "a.txt\r\nX-Evil: 1".to_string(),
                    content_type: "text/plain".to_string(),
                    bytes: vec![1, 2, 3],
                });
            }),
        ),
    ];

    for (field, mutate) in cases {
        let mut message = sample_message();
        mutate(&mut message);

        let error = build_message(&message, &sample_options()).expect_err("必须拒绝头部注入");

        match error {
            ComposeError::HeaderInjection { field: actual } => assert_eq!(actual, field),
            other => panic!("字段 {field} 的错误类型不对：{other}"),
        }
    }
}

#[test]
fn injection_check_runs_before_other_validation() {
    // 注入检查必须最先做：否则「标题带换行 + 正文为空」会先返回 EmptyBody，
    // 调用方可能误以为注入不是问题。
    let mut message = sample_message();
    message.subject = "x\r\nBcc: evil@example.com".to_string();
    message.body_text = None;
    message.body_html = None;

    let error = build_message(&message, &sample_options()).expect_err("必须拒绝头部注入");

    assert!(matches!(error, ComposeError::HeaderInjection { .. }));
}

// ===========================================================================
// 2. 密送不泄漏
// ===========================================================================

#[test]
fn bcc_never_appears_in_the_message_bytes() {
    let mut message = sample_message();
    message.bcc = vec!["secret@example.com".to_string()];

    let built = build_message(&message, &sample_options()).expect("应当构建成功");
    let text = as_text(&built);

    assert!(
        !text.contains("secret@example.com"),
        "密送地址泄漏进了邮件字节"
    );
    assert!(!text.contains("Bcc:"), "邮件字节里不该有 Bcc 头");
    assert!(
        built
            .envelope_recipients
            .contains(&"secret@example.com".to_string()),
        "密送地址必须进 envelope"
    );
}

// ===========================================================================
// 3. 信封收件人
// ===========================================================================

#[test]
fn envelope_is_to_plus_cc_plus_bcc_deduped() {
    let mut message = sample_message();
    message.to = vec![
        "alice@example.com".to_string(),
        "alice@example.com".to_string(),
        String::new(),
    ];
    message.cc = vec!["bob@example.com".to_string()];
    message.bcc = vec![
        "alice@example.com".to_string(),
        "carol@example.com".to_string(),
    ];

    let built = build_message(&message, &sample_options()).expect("应当构建成功");

    assert_eq!(
        built.envelope_recipients,
        vec![
            "alice@example.com".to_string(),
            "bob@example.com".to_string(),
            "carol@example.com".to_string(),
        ]
    );
    assert!(
        !built
            .envelope_recipients
            .contains(&"sender@example.com".to_string()),
        "envelope 里不该有发件人"
    );
}

#[test]
fn message_without_recipients_is_rejected() {
    let mut message = sample_message();
    message.to.clear();

    let error = build_message(&message, &sample_options()).expect_err("必须拒绝");

    assert!(
        matches!(error, ComposeError::EmptyRecipient),
        "实际错误：{error}"
    );
}

// ===========================================================================
// 4. 非 ASCII 标题（RFC 2047）
// ===========================================================================

#[test]
fn non_ascii_subject_is_rfc2047_encoded_and_round_trips() {
    let mut message = sample_message();
    message.subject = "中文标题".to_string();

    let built = build_message(&message, &sample_options()).expect("应当构建成功");
    let value = header_value(&built, "Subject").expect("应当有 Subject 头");

    assert!(
        value.starts_with("=?utf-8?B?"),
        "非 ASCII 标题必须编码成 base64 编码字，实际：{value}"
    );
    assert!(value.ends_with("?="), "编码字必须以 ?= 结束：{value}");
    assert_eq!(
        decode_encoded_words(&value),
        "中文标题",
        "编码后的标题必须能解回原文"
    );
}

#[test]
fn mixed_subject_keeps_ascii_part_readable() {
    let mut message = sample_message();
    message.subject = "Re: 中文标题".to_string();

    let built = build_message(&message, &sample_options()).expect("应当构建成功");
    let value = header_value(&built, "Subject").expect("应当有 Subject 头");

    // ASCII 前缀原样保留（RFC 2047 只要求编码非 ASCII 部分）。
    assert!(value.starts_with("Re: "), "实际：{value}");
    assert_eq!(decode_encoded_words(&value), "Re: 中文标题");
}

#[test]
fn ascii_subject_is_not_encoded() {
    let built = build_message(&sample_message(), &sample_options()).expect("应当构建成功");
    let value = header_value(&built, "Subject").expect("应当有 Subject 头");

    assert_eq!(value, "Hello", "纯 ASCII 标题不该被编码");
}

// ===========================================================================
// 5. 地址里的显示名
// ===========================================================================

#[test]
fn display_name_is_rfc2047_encoded() {
    let mut message = sample_message();
    message.from = "张三 <zhang@example.com>".to_string();

    let built = build_message(&message, &sample_options()).expect("应当构建成功");
    let value = header_value(&built, "From").expect("应当有 From 头");

    assert!(
        value.starts_with("=?utf-8?B?"),
        "非 ASCII 显示名必须编码，实际：{value}"
    );
    assert!(
        value.ends_with("<zhang@example.com>"),
        "显示名编码后必须保留裸地址，实际：{value}"
    );
    assert_eq!(decode_encoded_words(&value), "张三 <zhang@example.com>");
}

#[test]
fn ascii_display_name_is_kept_as_is() {
    let mut message = sample_message();
    message.from = "Zhang San <zhang@example.com>".to_string();

    let built = build_message(&message, &sample_options()).expect("应当构建成功");
    let value = header_value(&built, "From").expect("应当有 From 头");

    assert_eq!(value, "Zhang San <zhang@example.com>");
}

// ===========================================================================
// 6. 正文形态
// ===========================================================================

#[test]
fn text_only_body_is_text_plain() {
    let built = build_message(&sample_message(), &sample_options()).expect("应当构建成功");
    let value = header_value(&built, "Content-Type").expect("应当有 Content-Type 头");

    assert_eq!(value, "text/plain; charset=utf-8");
    assert!(!as_text(&built).contains("multipart/"));
}

#[test]
fn html_only_body_is_text_html() {
    let mut message = sample_message();
    message.body_text = None;
    message.body_html = Some("<p>hello</p>".to_string());

    let built = build_message(&message, &sample_options()).expect("应当构建成功");
    let value = header_value(&built, "Content-Type").expect("应当有 Content-Type 头");

    assert_eq!(value, "text/html; charset=utf-8");
    assert!(!as_text(&built).contains("multipart/"));
}

#[test]
fn both_bodies_become_multipart_alternative_with_text_first() {
    let mut message = sample_message();
    message.body_html = Some("<p>hello</p>".to_string());

    let built = build_message(&message, &sample_options()).expect("应当构建成功");
    let text = as_text(&built);

    assert!(
        text.contains("Content-Type: multipart/alternative"),
        "两种正文必须用 multipart/alternative，实际：{text}"
    );

    let text_at = text.find("text/plain").expect("应当有 text/plain 部分");
    let html_at = text.find("text/html").expect("应当有 text/html 部分");
    assert!(
        text_at < html_at,
        "纯文本部分必须在 HTML 部分之前（RFC 2046 §5.1.4）"
    );
}

#[test]
fn attachments_are_wrapped_in_multipart_mixed() {
    let mut message = sample_message();
    message.body_html = Some("<p>hello</p>".to_string());
    message.attachments.push(OutgoingAttachment {
        filename: "report.txt".to_string(),
        content_type: "text/plain".to_string(),
        bytes: b"hello attachment".to_vec(),
    });

    let built = build_message(&message, &sample_options()).expect("应当构建成功");
    let text = as_text(&built);

    assert!(
        text.contains("Content-Type: multipart/mixed"),
        "有附件时最外层必须是 multipart/mixed，实际：{text}"
    );

    let mixed_at = text.find("multipart/mixed").expect("应当有 mixed");
    let alternative_at = text
        .find("multipart/alternative")
        .expect("alternative 应当嵌在 mixed 里");
    let attachment_at = text.find("text/plain").expect("应当有附件部分");
    assert!(mixed_at < alternative_at, "alternative 必须嵌在 mixed 内部");
    assert!(alternative_at < attachment_at, "附件部分必须在正文部分之后");
}

// ===========================================================================
// 7. 传输编码
// ===========================================================================

#[test]
fn seven_bit_body_is_quoted_printable_and_pure_ascii() {
    let mut message = sample_message();
    message.subject = "中文标题".to_string();
    message.body_text = Some("正文内容 with 中文".to_string());

    let built = build_message(&message, &sample_options()).expect("应当构建成功");
    let text = as_text(&built);

    assert!(
        text.contains("Content-Transfer-Encoding: quoted-printable"),
        "7bit 模式必须用 quoted-printable，实际：{text}"
    );
    assert!(
        built.bytes.iter().all(|byte| *byte < 128),
        "7bit 模式下整封邮件必须是纯 ASCII"
    );
}

#[test]
fn eight_bit_body_is_raw_utf8() {
    let mut options = sample_options();
    options.eight_bit = true;

    let mut message = sample_message();
    message.body_text = Some("正文内容 with 中文".to_string());

    let built = build_message(&message, &options).expect("应当构建成功");
    let text = as_text(&built);

    assert!(
        text.contains("Content-Transfer-Encoding: 8bit"),
        "8bit 模式必须声明 8bit，实际：{text}"
    );
    assert!(text.contains("正文内容 with 中文"), "8bit 正文必须原样透传");
}

#[test]
fn attachments_are_base64_encoded_with_disposition() {
    let mut message = sample_message();
    message.attachments.push(OutgoingAttachment {
        filename: "report.txt".to_string(),
        content_type: "text/plain".to_string(),
        bytes: b"hello attachment".to_vec(),
    });

    let built = build_message(&message, &sample_options()).expect("应当构建成功");
    let text = as_text(&built);

    assert!(
        text.contains("Content-Transfer-Encoding: base64"),
        "附件必须用 base64，实际：{text}"
    );
    assert!(
        text.contains("Content-Disposition: attachment; filename=\"report.txt\""),
        "附件必须带 Content-Disposition，实际：{text}"
    );
    assert!(
        text.contains("aGVsbG8gYXR0YWNobWVudA=="),
        "附件内容必须是 base64，实际：{text}"
    );
}

#[test]
fn non_ascii_filename_keeps_an_ascii_fallback() {
    let mut message = sample_message();
    message.attachments.push(OutgoingAttachment {
        filename: "报告.txt".to_string(),
        content_type: "text/plain".to_string(),
        bytes: vec![0u8; 4],
    });

    let built = build_message(&message, &sample_options()).expect("应当构建成功");
    let text = as_text(&built);

    assert!(
        text.contains("Content-Disposition: attachment; filename=\""),
        "附件必须带 Content-Disposition，实际：{text}"
    );
    assert!(
        text.contains("filename*=utf-8''"),
        "非 ASCII 文件名必须给出 RFC 2231 形式，实际：{text}"
    );
    assert!(
        built.bytes.iter().all(|byte| *byte < 128),
        "非 ASCII 文件名不能让邮件变成非 ASCII"
    );
}

// ===========================================================================
// 8. 必需头字段
// ===========================================================================

#[test]
fn required_headers_are_present() {
    let built = build_message(&sample_message(), &sample_options()).expect("应当构建成功");

    for name in [
        "From",
        "To",
        "Subject",
        "Date",
        "Message-ID",
        "MIME-Version",
        "Content-Type",
    ] {
        assert!(
            header_value(&built, name).is_some(),
            "缺少必需头字段：{name}"
        );
    }

    assert_eq!(
        header_value(&built, "MIME-Version").as_deref(),
        Some("1.0"),
        "MIME-Version 必须是 1.0"
    );
}

#[test]
fn cc_header_is_omitted_when_there_is_no_cc() {
    let built = build_message(&sample_message(), &sample_options()).expect("应当构建成功");

    assert!(
        header_value(&built, "Cc").is_none(),
        "没有 Cc 时不能写 Cc 头"
    );
}

#[test]
fn cc_header_is_written_when_present() {
    let mut message = sample_message();
    message.cc = vec!["bob@example.com".to_string()];

    let built = build_message(&message, &sample_options()).expect("应当构建成功");

    assert_eq!(
        header_value(&built, "Cc").as_deref(),
        Some("bob@example.com")
    );
}

#[test]
fn missing_from_is_rejected() {
    let mut message = sample_message();
    message.from = String::new();

    let error = build_message(&message, &sample_options()).expect_err("必须拒绝");

    assert!(
        matches!(error, ComposeError::MissingFrom),
        "实际错误：{error}"
    );
}

#[test]
fn empty_body_is_rejected() {
    let mut message = sample_message();
    message.body_text = None;
    message.body_html = None;

    let error = build_message(&message, &sample_options()).expect_err("必须拒绝");

    assert!(
        matches!(error, ComposeError::EmptyBody),
        "实际错误：{error}"
    );
}

#[test]
fn malformed_address_is_rejected() {
    let mut message = sample_message();
    message.to = vec!["not-an-address".to_string()];

    let error = build_message(&message, &sample_options()).expect_err("必须拒绝");

    assert!(
        matches!(error, ComposeError::InvalidAddress(_)),
        "实际错误：{error}"
    );
}

// ===========================================================================
// 9. Date 格式
// ===========================================================================

#[test]
fn date_uses_rfc5322_format() {
    let mut options = sample_options();
    options.now_epoch_secs = 0;

    let built = build_message(&sample_message(), &options).expect("应当构建成功");

    assert_eq!(
        header_value(&built, "Date").as_deref(),
        Some("Thu, 01 Jan 1970 00:00:00 +0000")
    );
}

#[test]
fn date_handles_leap_day_and_century() {
    let cases = [
        (946_684_800_i64, "Sat, 01 Jan 2000 00:00:00 +0000"),
        (1_709_210_096, "Thu, 29 Feb 2024 12:34:56 +0000"),
        (946_684_799, "Fri, 31 Dec 1999 23:59:59 +0000"),
        (4_107_542_400, "Mon, 01 Mar 2100 00:00:00 +0000"),
    ];

    for (epoch, expected) in cases {
        let mut options = sample_options();
        options.now_epoch_secs = epoch;

        let built = build_message(&sample_message(), &options).expect("应当构建成功");

        assert_eq!(
            header_value(&built, "Date").as_deref(),
            Some(expected),
            "epoch {epoch} 的日期不对"
        );
    }
}

// ===========================================================================
// 10. Message-ID
// ===========================================================================

#[test]
fn message_id_has_the_expected_shape() {
    let built = build_message(&sample_message(), &sample_options()).expect("应当构建成功");

    assert!(
        built.message_id.starts_with('<') && built.message_id.ends_with('>'),
        "Message-ID 必须带尖括号：{}",
        built.message_id
    );
    assert!(
        built.message_id.ends_with("@xremail.local>"),
        "Message-ID 必须用配置的域：{}",
        built.message_id
    );
    assert_eq!(
        header_value(&built, "Message-ID").as_deref(),
        Some(built.message_id.as_str())
    );
}

#[test]
fn message_id_is_unique_between_two_builds() {
    let first = build_message(&sample_message(), &sample_options()).expect("应当构建成功");
    let second = build_message(&sample_message(), &sample_options()).expect("应当构建成功");

    assert_ne!(
        first.message_id, second.message_id,
        "同一进程内连续构建两次不能得到相同的 Message-ID"
    );
}

// ===========================================================================
// 11. CRLF
// ===========================================================================

#[test]
fn message_uses_crlf_line_endings() {
    let mut message = sample_message();
    message.body_text = Some("第一行\n第二行".to_string());
    message.attachments.push(OutgoingAttachment {
        filename: "a.txt".to_string(),
        content_type: "text/plain".to_string(),
        bytes: vec![1, 2, 3],
    });

    let built = build_message(&message, &sample_options()).expect("应当构建成功");

    assert_crlf_line_endings(&built.bytes);
    assert!(built.bytes.ends_with(b"\r\n"), "邮件必须以 CRLF 结束");
}

// ===========================================================================
// 12. multipart 边界
// ===========================================================================

#[test]
fn multipart_boundaries_differ_between_two_messages() {
    let mut message = sample_message();
    message.attachments.push(OutgoingAttachment {
        filename: "a.txt".to_string(),
        content_type: "text/plain".to_string(),
        bytes: vec![1, 2, 3],
    });

    let first = build_message(&message, &sample_options()).expect("应当构建成功");
    let second = build_message(&message, &sample_options()).expect("应当构建成功");

    let boundary_of = |built: &BuiltMessage| {
        let text = as_text(built);
        let start = text.find("boundary=\"").expect("应当有 boundary 参数");
        let rest = &text[start + "boundary=\"".len()..];
        let end = rest.find('"').expect("boundary 参数必须闭合");
        rest[..end].to_string()
    };

    assert_ne!(
        boundary_of(&first),
        boundary_of(&second),
        "不同邮件的边界不能重复"
    );
    assert!(
        as_text(&first).contains(&format!("--{}", boundary_of(&first))),
        "邮件里必须出现 --boundary 分隔行"
    );
}

// ===========================================================================
// 字符集
// ===========================================================================

#[test]
fn charset_option_is_used_for_body_and_headers() {
    let mut options = sample_options();
    options.charset = "gbk".to_string();

    let mut message = sample_message();
    message.subject = "中文标题".to_string();
    message.body_text = Some("中文正文".to_string());

    let built = build_message(&message, &options).expect("应当构建成功");
    let text = as_text(&built);

    assert!(
        text.contains("Content-Type: text/plain; charset=gbk"),
        "Content-Type 必须用配置的字符集，实际：{text}"
    );
    assert!(
        text.contains("=?gbk?B?"),
        "编码字必须用配置的字符集，实际：{text}"
    );
}

#[test]
fn unknown_charset_falls_back_to_utf8() {
    let mut options = sample_options();
    options.charset = "x-not-a-charset".to_string();

    let built = build_message(&sample_message(), &options).expect("应当构建成功");

    assert_eq!(
        header_value(&built, "Content-Type").as_deref(),
        Some("text/plain; charset=utf-8")
    );
}

// ===========================================================================
// 传输编码与编码字（RFC 2045 / 2047 的细节）
// ===========================================================================

#[test]
fn quoted_printable_round_trips_through_the_decoder() {
    let original = "第一行\r\n第二行 with = and space \r\n".as_bytes();

    let encoded = encode_quoted_printable(original);
    let decoded = decode_quoted_printable(&encoded);

    assert_eq!(decoded, original.to_vec());
}

#[test]
fn quoted_printable_escapes_special_bytes() {
    let encoded = String::from_utf8(encode_quoted_printable(b"a=b c")).expect("QP 输出是 ASCII");

    assert_eq!(encoded, "a=3Db c");
}

#[test]
fn quoted_printable_encodes_trailing_space() {
    let encoded =
        String::from_utf8(encode_quoted_printable(b"trailing ")).expect("QP 输出是 ASCII");

    // 行尾的空白必须编码，否则中间的 MTA 会把它吃掉。
    assert_eq!(encoded, "trailing=20");
}

#[test]
fn quoted_printable_lines_are_short_enough() {
    let body = "中".repeat(200);

    let encoded = encode_quoted_printable(body.as_bytes());
    for line in String::from_utf8(encoded.clone())
        .expect("QP 输出是 ASCII")
        .split("\r\n")
    {
        assert!(line.len() <= 76, "QP 单行过长：{} 字符", line.len());
    }
    assert_eq!(
        decode_quoted_printable(&encoded),
        body.as_bytes().to_vec(),
        "QP 折行后必须还能解回原文"
    );
}

#[test]
fn long_subject_is_split_into_several_encoded_words() {
    let subject =
        "这是一段很长的中文标题用来测试RFC2047编码字的长度限制是否会被正确拆开".to_string();

    let mut message = sample_message();
    message.subject = subject.clone();

    let built = build_message(&message, &sample_options()).expect("应当构建成功");
    let value = header_value(&built, "Subject").expect("应当有 Subject 头");

    let words: Vec<&str> = value.split(' ').collect();
    assert!(words.len() > 1, "过长的标题必须拆成多个编码字：{value}");
    for word in &words {
        assert!(
            word.len() <= 75,
            "RFC 2047 §2 规定单个编码字不超过 75 字符，实际 {}：{word}",
            word.len()
        );
        assert!(word.starts_with("=?utf-8?B?"), "编码字格式不对：{word}");
    }
    assert_eq!(
        decode_encoded_words(&value),
        subject,
        "拆开后必须能解回原文"
    );
}

#[test]
fn spaces_inside_a_non_ascii_subject_survive_the_round_trip() {
    let subject = "中 文 标 题".to_string();

    let mut message = sample_message();
    message.subject = subject.clone();

    let built = build_message(&message, &sample_options()).expect("应当构建成功");
    let value = header_value(&built, "Subject").expect("应当有 Subject 头");

    assert_eq!(decode_encoded_words(&value), subject);
}
