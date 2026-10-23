//! FETCH 项与 STORE 参数的解析。
//!
//! 都在**服务端**这一侧：客户端把 sequence set 和取件项当**原始字符串**发过来，
//! 由服务器解释。

use crate::config::MessageFixture;

/// FETCH / STORE 参数解析错误。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FetchError {
    /// 语法错误。
    #[error("参数语法错误: {0}")]
    Invalid(String),
}

/// `BODY[...]` 的取哪部分。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BodySection {
    /// 整封邮件（`BODY[]`）。
    Full,
    /// 头（`BODY[HEADER]`）。
    Header,
    /// 正文（`BODY[TEXT]`）。
    Text,
    /// 指定的头字段（`BODY[HEADER.FIELDS (FROM TO)]`）。
    HeaderFields(Vec<String>),
}

/// 一个取件项。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchItem {
    /// `UID`。
    Uid,
    /// `FLAGS`。
    Flags,
    /// `RFC822.SIZE`。
    Rfc822Size,
    /// `INTERNALDATE`。
    InternalDate,
    /// `ENVELOPE`。
    Envelope,
    /// `MODSEQ`（CONDSTORE）。
    ModSeq,
    /// `BODY[...]` / `BODY.PEEK[...]`。
    Body {
        /// 是不是 `PEEK` 变体。
        ///
        /// `BODY[...]`（不带 PEEK）会**置上 `\Seen`**，`BODY.PEEK[...]` 不会。
        peek: bool,
        /// 取哪部分。
        section: BodySection,
    },
    /// `RFC822` / `RFC822.HEADER` / `RFC822.TEXT`。
    Rfc822 {
        /// 取哪部分（`None` = 整封）。
        part: Option<BodySection>,
    },
    /// 不认识的项（忽略即可）。
    Other(String),
}

/// 解析 `FETCH` / `UID FETCH` 的参数。
///
/// 返回 `(sequence set, 取件项列表)`。
///
/// ```
/// use xremail_testkit::fetch::parse_fetch;
/// let (set, items) = parse_fetch("1:* (UID FLAGS RFC822.SIZE)").unwrap();
/// assert_eq!(set, "1:*");
/// assert_eq!(items.len(), 3);
/// ```
pub fn parse_fetch(args: &str) -> Result<(String, Vec<FetchItem>), FetchError> {
    let args = args.trim();
    let (set, rest) =
        split_first_token(args).ok_or_else(|| FetchError::Invalid("缺少 sequence set".into()))?;
    if set.is_empty() {
        return Err(FetchError::Invalid("sequence set 为空".into()));
    }

    let items = parse_items(rest)?;
    Ok((set.to_string(), items))
}

/// 解析取件项列表（带外层括号）。
pub fn parse_items(input: &str) -> Result<Vec<FetchItem>, FetchError> {
    let trimmed = input.trim();
    let inner = trimmed
        .strip_prefix('(')
        .and_then(|rest| rest.strip_suffix(')'))
        .ok_or_else(|| FetchError::Invalid(format!("取件项缺括号: {input}")))?;

    Ok(split_top_level(inner)
        .into_iter()
        .filter(|item| !item.is_empty())
        .map(|item| classify_item(&item))
        .collect())
}

/// 切出第一个词（其余原样返回）。
fn split_first_token(input: &str) -> Option<(&str, &str)> {
    let input = input.trim_start();
    match input.find(char::is_whitespace) {
        Some(index) => Some((&input[..index], input[index..].trim_start())),
        None => Some((input, "")),
    }
}

/// 按顶层空白切分（括号/方括号内部的不算）。
fn split_top_level(input: &str) -> Vec<String> {
    let mut items = Vec::new();
    let mut current = String::new();
    let mut depth: i32 = 0;

    for ch in input.chars() {
        match ch {
            '(' | '[' => {
                depth += 1;
                current.push(ch);
            }
            ')' | ']' => {
                depth -= 1;
                current.push(ch);
            }
            c if c.is_whitespace() && depth == 0 => {
                if !current.is_empty() {
                    items.push(std::mem::take(&mut current));
                }
            }
            c => current.push(c),
        }
    }
    if !current.is_empty() {
        items.push(current);
    }
    items
}

/// 归类一个取件项。
fn classify_item(item: &str) -> FetchItem {
    let upper = item.to_ascii_uppercase();

    if upper == "UID" {
        return FetchItem::Uid;
    }
    if upper == "FLAGS" {
        return FetchItem::Flags;
    }
    if upper == "RFC822.SIZE" {
        return FetchItem::Rfc822Size;
    }
    if upper == "INTERNALDATE" {
        return FetchItem::InternalDate;
    }
    if upper == "ENVELOPE" {
        return FetchItem::Envelope;
    }
    if upper == "MODSEQ" {
        return FetchItem::ModSeq;
    }

    if let Some(rest) = item.strip_prefix("BODY") {
        let peek = rest.to_ascii_uppercase().starts_with(".PEEK");
        let section = rest
            .find('[')
            .and_then(|start| {
                let inner = &rest[start + 1..];
                inner.rfind(']').map(|end| &inner[..end])
            })
            .unwrap_or("");
        return FetchItem::Body {
            peek,
            section: parse_body_section(section),
        };
    }

    if let Some(rest) = item.strip_prefix("RFC822") {
        let part = match rest.to_ascii_uppercase().as_str() {
            "" => None,
            ".HEADER" => Some(BodySection::Header),
            ".TEXT" => Some(BodySection::Text),
            _ => return FetchItem::Other(item.to_string()),
        };
        return FetchItem::Rfc822 { part };
    }

    FetchItem::Other(item.to_string())
}

/// 解析 `BODY[...]` 方括号里的内容。
fn parse_body_section(content: &str) -> BodySection {
    let content = content.trim();
    let upper = content.to_ascii_uppercase();

    if content.is_empty() {
        return BodySection::Full;
    }
    if upper == "HEADER" {
        return BodySection::Header;
    }
    if upper == "TEXT" {
        return BodySection::Text;
    }
    if let Some(rest) = upper.strip_prefix("HEADER.FIELDS") {
        // `HEADER.FIELDS (FROM TO)` 或 `HEADER.FIELDS (FROM TO)`。
        let fields = rest
            .trim()
            .trim_start_matches('(')
            .trim_end_matches(')')
            .split_whitespace()
            .map(|field| {
                // 保持原始大小写：用原文里的对应片段。
                content
                    .split_whitespace()
                    .find(|token| token.eq_ignore_ascii_case(field))
                    .unwrap_or(field)
                    .to_string()
            })
            .collect();
        return BodySection::HeaderFields(fields);
    }

    // 不认识的段落 → 当作整封。
    BodySection::Full
}

/// STORE 的操作类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreOperation {
    /// `+FLAGS`：加上。
    Add,
    /// `-FLAGS`：去掉。
    Remove,
    /// `FLAGS`：整组替换。
    Set,
}

/// 一条 STORE 请求。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreRequest {
    /// 操作类型。
    pub operation: StoreOperation,
    /// 涉及的标记。
    pub flags: Vec<String>,
    /// 是不是 `.SILENT` 变体（不回 untagged FETCH 响应）。
    pub silent: bool,
    /// `UNCHANGEDSINCE (<modseq>)`：条件写。
    ///
    /// 当前 modseq 与之不符时**不改**，并回 `NO [MODIFIED]`。
    pub unchangedsince: Option<u64>,
}

/// 解析 `STORE` / `UID STORE` 的参数（sequence set 之后的部分）。
///
/// ```
/// use xremail_testkit::fetch::{parse_store, StoreOperation};
/// let request = parse_store("+FLAGS (\\Seen)").unwrap();
/// assert_eq!(request.operation, StoreOperation::Add);
/// assert_eq!(request.flags, vec!["\\Seen"]);
/// ```
pub fn parse_store(args: &str) -> Result<StoreRequest, FetchError> {
    let args = args.trim();
    // 第一个词是 `FLAGS` / `FLAGS.SILENT`（可带 `+` / `-` 前缀）。
    let (head, rest) =
        split_first_token(args).ok_or_else(|| FetchError::Invalid("STORE 缺少参数".into()))?;

    let (operation, head) = if let Some(rest) = head.strip_prefix('+') {
        (StoreOperation::Add, rest.trim())
    } else if let Some(rest) = head.strip_prefix('-') {
        (StoreOperation::Remove, rest.trim())
    } else {
        (StoreOperation::Set, head)
    };

    // `FLAGS` 或 `FLAGS.SILENT`。
    let silent = head.to_ascii_uppercase().ends_with(".SILENT");
    let keyword = if silent {
        &head[..head.len() - ".SILENT".len()]
    } else {
        head
    };
    if !keyword.eq_ignore_ascii_case("FLAGS") {
        return Err(FetchError::Invalid(format!(
            "STORE 只支持 FLAGS，收到: {keyword}"
        )));
    }

    // 标记列表在 `rest` 里；`UNCHANGEDSINCE (<n>)` 跟在它**后面**。
    let (flag_text, unchangedsince) = split_unchangedsince(rest);

    let flags = flag_text
        .trim()
        .trim_start_matches('(')
        .trim_end_matches(')')
        .split_whitespace()
        .map(str::to_string)
        .collect();

    Ok(StoreRequest {
        operation,
        flags,
        silent,
        unchangedsince,
    })
}

/// 拆出 `UNCHANGEDSINCE (<n>)`。
fn split_unchangedsince(input: &str) -> (&str, Option<u64>) {
    let upper = input.to_ascii_uppercase();
    let Some(index) = upper.find("UNCHANGEDSINCE") else {
        return (input, None);
    };

    let before = &input[..index];
    let after = &input[index + "UNCHANGEDSINCE".len()..];
    let after = after.trim_start();
    let after = after.strip_prefix('(').unwrap_or(after);
    let Some(end) = after.find(')') else {
        return (before, None);
    };
    let number = after[..end].trim().parse::<u64>().ok();
    (before, number)
}

/// 一条 FETCH 响应。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchResponse {
    /// 序号（邮件在文件夹里的第几封，**不是** UID）。
    pub seq: u32,
    /// UID。
    pub uid: u32,
    /// 简单部分（如 `UID 1 FLAGS (\Seen) RFC822.SIZE 42`）。
    pub simple: String,
    /// 字面量部分（`(section 名, 字节)`）。
    ///
    /// 有字面量时它排在最后——服务器也习惯这么排。
    pub literal: Option<(String, Vec<u8>)>,
}

/// 生成一条 FETCH 响应。
pub fn build_fetch_response(
    seq: u32,
    message: &MessageFixture,
    items: &[FetchItem],
    with_modseq: bool,
) -> FetchResponse {
    let mut parts: Vec<String> = Vec::new();
    let mut literal: Option<(String, Vec<u8>)> = None;

    for item in items {
        match item {
            FetchItem::Uid => parts.push(format!("UID {}", message.uid)),
            FetchItem::Flags => {
                parts.push(format!("FLAGS ({})", message.flags.join(" ")));
            }
            FetchItem::Rfc822Size => {
                parts.push(format!("RFC822.SIZE {}", message.size()));
            }
            FetchItem::InternalDate => {
                parts.push(format!(
                    "INTERNALDATE \"{}\"",
                    message.header_values("Date").first().copied().unwrap_or("")
                ));
            }
            FetchItem::Envelope => {
                parts.push(format!("ENVELOPE {}", build_envelope(message)));
            }
            FetchItem::ModSeq => {
                if with_modseq {
                    parts.push(format!("MODSEQ ({})", message.modseq));
                }
            }
            FetchItem::Body { section, .. } => {
                let (name, bytes) = body_section_content(message, section);
                literal = Some((name, bytes));
            }
            FetchItem::Rfc822 { part } => {
                let section = part.clone().unwrap_or(BodySection::Full);
                let (name, bytes) = body_section_content(message, &section);
                literal = Some((name, bytes));
            }
            FetchItem::Other(_) => {}
        }
    }

    FetchResponse {
        seq,
        uid: message.uid,
        simple: parts.join(" "),
        literal,
    }
}

/// 取 `BODY[...]` 的内容。
///
/// 返回 `(响应里的段落名, 字节)`。
fn body_section_content(message: &MessageFixture, section: &BodySection) -> (String, Vec<u8>) {
    match section {
        BodySection::Full => ("BODY[]".to_string(), message.content().as_bytes().to_vec()),
        BodySection::Header => (
            "BODY[HEADER]".to_string(),
            message.header().as_bytes().to_vec(),
        ),
        BodySection::Text => ("BODY[TEXT]".to_string(), message.body().as_bytes().to_vec()),
        BodySection::HeaderFields(fields) => {
            let mut text = String::new();
            for field in fields {
                for value in message.header_values(field) {
                    text.push_str(field);
                    text.push_str(": ");
                    text.push_str(value);
                    text.push_str("\r\n");
                }
            }
            (
                format!("BODY[HEADER.FIELDS ({})]", fields.join(" ")),
                text.into_bytes(),
            )
        }
    }
}

/// 生成 `ENVELOPE`。
///
/// 结构（RFC 3501 §7.4.2）：
/// `(date subject from sender reply-to to cc bcc in-reply-to message-id)`
fn build_envelope(message: &MessageFixture) -> String {
    let date = message
        .header_values("Date")
        .first()
        .map(|value| quote(value))
        .unwrap_or_else(|| "NIL".to_string());
    let subject = message
        .header_values("Subject")
        .first()
        .map(|value| quote(value))
        .unwrap_or_else(|| "NIL".to_string());
    let from = address_list(message.header_values("From").first().copied());
    let sender = address_list(message.header_values("Sender").first().copied());
    let reply_to = address_list(message.header_values("Reply-To").first().copied());
    let to = address_list(message.header_values("To").first().copied());
    let message_id = message
        .header_values("Message-ID")
        .first()
        .map(|value| quote(value.trim_matches(['<', '>'])))
        .unwrap_or_else(|| "NIL".to_string());

    format!("({date} {subject} {from} {sender} {reply_to} {to} NIL NIL NIL {message_id})")
}

/// 生成地址列表 `((name adl mailbox host) …)`。
fn address_list(value: Option<&str>) -> String {
    let Some(value) = value else {
        return "NIL".to_string();
    };

    let addresses: Vec<String> = value
        .split(',')
        .filter_map(|part| {
            let part = part.trim();
            if part.is_empty() {
                return None;
            }
            let (name, address) = match (part.find('<'), part.rfind('>')) {
                (Some(start), Some(end)) if end > start => (
                    Some(part[..start].trim().trim_matches('"')),
                    &part[start + 1..end],
                ),
                _ => (None, part),
            };
            let (mailbox, host) = address.rsplit_once('@').unwrap_or((address, ""));

            let name = name
                .filter(|name| !name.is_empty())
                .map(quote)
                .unwrap_or_else(|| "NIL".to_string());

            Some(format!("({name} NIL {} {})", quote(mailbox), quote(host)))
        })
        .collect();

    if addresses.is_empty() {
        "NIL".to_string()
    } else {
        format!("({})", addresses.join(""))
    }
}

/// 加引号并转义。
fn quote(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        if ch == '"' || ch == '\\' {
            out.push('\\');
        }
        out.push(ch);
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sequence_set_and_items() {
        let (set, items) = parse_fetch("1:* (UID FLAGS RFC822.SIZE)").unwrap();

        assert_eq!(set, "1:*");
        assert_eq!(
            items,
            vec![FetchItem::Uid, FetchItem::Flags, FetchItem::Rfc822Size]
        );
    }

    #[test]
    fn parses_body_peek_header() {
        let (_, items) = parse_fetch("1 (BODY.PEEK[HEADER])").unwrap();

        assert_eq!(
            items,
            vec![FetchItem::Body {
                peek: true,
                section: BodySection::Header
            }]
        );
    }

    #[test]
    fn parses_body_without_peek() {
        let (_, items) = parse_fetch("1 (BODY[])").unwrap();

        assert_eq!(
            items,
            vec![FetchItem::Body {
                peek: false,
                section: BodySection::Full
            }]
        );
    }

    #[test]
    fn parses_header_fields_with_nested_parens() {
        let (_, items) = parse_fetch("1 (BODY.PEEK[HEADER.FIELDS (FROM TO SUBJECT)])").unwrap();

        match &items[0] {
            FetchItem::Body { section, .. } => match section {
                BodySection::HeaderFields(fields) => {
                    assert_eq!(fields, &vec!["FROM", "TO", "SUBJECT"]);
                }
                other => panic!("期望 HeaderFields，实际是 {other:?}"),
            },
            other => panic!("期望 Body，实际是 {other:?}"),
        }
    }

    #[test]
    fn parses_rfc822_variants() {
        let (_, items) = parse_fetch("1 (RFC822.HEADER)").unwrap();
        assert_eq!(
            items,
            vec![FetchItem::Rfc822 {
                part: Some(BodySection::Header)
            }]
        );

        let (_, items) = parse_fetch("1 (RFC822)").unwrap();
        assert_eq!(items, vec![FetchItem::Rfc822 { part: None }]);
    }

    #[test]
    fn unknown_item_is_kept_but_ignored() {
        let (_, items) = parse_fetch("1 (X-GM-LABELS)").unwrap();

        assert_eq!(items, vec![FetchItem::Other("X-GM-LABELS".into())]);
    }

    #[test]
    fn missing_parentheses_is_an_error() {
        assert!(parse_fetch("1 UID FLAGS").is_err());
    }

    #[test]
    fn parses_store_operations() {
        assert_eq!(
            parse_store("+FLAGS (\\Seen)").unwrap().operation,
            StoreOperation::Add
        );
        assert_eq!(
            parse_store("-FLAGS (\\Seen)").unwrap().operation,
            StoreOperation::Remove
        );
        assert_eq!(
            parse_store("FLAGS (\\Seen \\Draft)").unwrap().operation,
            StoreOperation::Set
        );
    }

    #[test]
    fn parses_silent_flag() {
        let request = parse_store("+FLAGS.SILENT (\\Seen)").unwrap();

        assert!(request.silent);
        assert_eq!(request.flags, vec!["\\Seen"]);
    }

    #[test]
    fn parses_unchangedsince() {
        let request = parse_store("+FLAGS (\\Seen) UNCHANGEDSINCE (12345)").unwrap();

        assert_eq!(request.unchangedsince, Some(12345));
        assert_eq!(request.flags, vec!["\\Seen"]);
    }

    #[test]
    fn store_requires_flags_keyword() {
        assert!(parse_store("+BOGUS (\\Seen)").is_err());
    }

    #[test]
    fn builds_fetch_response() {
        let message = MessageFixture::new(42).with_flag("\\Seen");
        let response =
            build_fetch_response(3, &message, &[FetchItem::Uid, FetchItem::Flags], false);

        assert_eq!(response.seq, 3);
        assert_eq!(response.uid, 42);
        assert_eq!(response.simple, "UID 42 FLAGS (\\Seen)");
        assert!(response.literal.is_none());
    }

    #[test]
    fn builds_body_literal() {
        let message = MessageFixture::new(1).with_body("内容");
        let response = build_fetch_response(
            1,
            &message,
            &[FetchItem::Body {
                peek: true,
                section: BodySection::Header,
            }],
            false,
        );

        let (name, bytes) = response.literal.expect("应当有字面量");
        assert_eq!(name, "BODY[HEADER]");
        assert!(String::from_utf8_lossy(&bytes).contains("Message-ID"));
    }

    #[test]
    fn builds_envelope() {
        let message = MessageFixture::new(1)
            .with_from("sender@example.com")
            .with_to("tester@example.com")
            .with_subject("主题");
        let response = build_fetch_response(1, &message, &[FetchItem::Envelope], false);

        assert!(response.simple.contains("ENVELOPE ("));
        assert!(response.simple.contains("\"主题\""));
        assert!(response.simple.contains("\"sender\" \"example.com\""));
    }
}
