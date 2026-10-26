//! 假 IMAP 服务器的配置。
//!
//! 配置项刻意做成**行为开关**，而不是「录制回放」：
//! 真实服务商的怪癖（163 拒绝 EXAMINE、要求先发 `ID`……）都能用开关复现，
//! 而录制回放只能复现一种情况。

/// 一封假邮件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageFixture {
    /// UID。**不保证**从 1 开始——163 的 UID 空间就很大。
    ///
    /// 这点很重要：客户端如果把 FETCH 响应里的**序号**（第几封）
    /// 当成 UID 用，在这种数据上就会立刻出错。
    pub uid: u32,
    /// 标记，如 `\Seen`。
    pub flags: Vec<String>,
    /// 完整邮件（RFC 822 文本：头 + 空行 + 正文）。
    pub content: String,
    /// 修改序号（CONDSTORE）。
    ///
    /// 每次标记变更都会 +1；`UNCHANGEDSINCE` 条件写就是比对这个值。
    pub modseq: u64,
}

impl MessageFixture {
    /// 新建一封未读邮件（带默认的头，正文为空）。
    pub fn new(uid: u32) -> Self {
        Self {
            uid,
            flags: Vec::new(),
            content: default_content(uid, "测试邮件", ""),
            modseq: 1,
        }
    }

    /// 加上一个标记。
    pub fn with_flag(mut self, flag: &str) -> Self {
        self.flags.push(flag.to_string());
        self
    }

    /// 设置修改序号。
    pub fn with_modseq(mut self, modseq: u64) -> Self {
        self.modseq = modseq;
        self
    }

    /// 替换整封邮件内容。
    pub fn with_content(mut self, content: &str) -> Self {
        self.content = content.to_string();
        self
    }

    /// 设置主题。
    pub fn with_subject(self, subject: &str) -> Self {
        self.with_header("Subject", subject)
    }

    /// 设置发件人。
    pub fn with_from(self, from: &str) -> Self {
        self.with_header("From", from)
    }

    /// 设置收件人。
    pub fn with_to(self, to: &str) -> Self {
        self.with_header("To", to)
    }

    /// 设置日期（`DD Mon YYYY HH:MM:SS +0800` 形式）。
    pub fn with_date(self, date: &str) -> Self {
        self.with_header("Date", date)
    }

    /// 设置正文。
    pub fn with_body(self, body: &str) -> Self {
        let separator = head_body_separator(&self.content);
        let (head, _) = split_head_body(&self.content);
        let mut content = head.to_string();
        content.push_str(separator);
        content.push_str(body);
        self.with_content(&content)
    }

    /// 替换一个头字段。
    fn with_header(self, field: &str, value: &str) -> Self {
        let ending = line_ending(&self.content);
        let separator = head_body_separator(&self.content);
        let (head, body) = split_head_body(&self.content);
        let mut lines: Vec<String> = head
            .lines()
            .filter(|line| {
                !line
                    .to_ascii_lowercase()
                    .starts_with(&field.to_ascii_lowercase())
            })
            .map(str::to_string)
            .collect();
        lines.push(format!("{field}: {value}"));
        // 头字段之间用单换行，末尾补一个空行把头与正文隔开。
        let mut content = lines.join(ending);
        content.push_str(separator);
        content.push_str(body);
        self.with_content(&content)
    }

    /// 整封邮件文本。
    pub fn content(&self) -> &str {
        &self.content
    }

    /// 头部分（到第一个空行为止）。
    pub fn header(&self) -> &str {
        split_head_body(&self.content).0
    }

    /// 正文部分。
    pub fn body(&self) -> &str {
        split_head_body(&self.content).1
    }

    /// 邮件大小（字节）。
    pub fn size(&self) -> u32 {
        self.content.len() as u32
    }

    /// 是否带某个标记（大小写不敏感）。
    pub fn has_flag(&self, flag: &str) -> bool {
        self.flags.iter().any(|f| f.eq_ignore_ascii_case(flag))
    }

    /// 某个头字段的所有值。
    pub fn header_values(&self, field: &str) -> Vec<&str> {
        self.header()
            .lines()
            .filter_map(|line| {
                let (name, value) = line.split_once(':')?;
                if name.trim().eq_ignore_ascii_case(field) {
                    Some(value.trim())
                } else {
                    None
                }
            })
            .collect()
    }

    /// 邮件日期（可比较的整数形式）。
    pub fn date(&self) -> Option<i32> {
        let values = self.header_values("Date");
        let value = values.first()?;
        parse_header_date(value)
    }
}

/// 拆出头与正文（以第一个空行为界）。
fn split_head_body(content: &str) -> (&str, &str) {
    let separator = head_body_separator(content);

    match content.find(separator) {
        Some(index) => {
            let head = &content[..index];
            let body = &content[index + separator.len()..];
            (head, body)
        }
        None => (content, ""),
    }
}

/// 行尾（`\r\n` 或 `\n`）。
fn line_ending(content: &str) -> &str {
    if content.contains("\r\n") {
        "\r\n"
    } else {
        "\n"
    }
}

/// 头与正文之间的空行（`\r\n\r\n` 或 `\n\n`）。
fn head_body_separator(content: &str) -> &str {
    if content.contains("\r\n\r\n") {
        "\r\n\r\n"
    } else if content.contains("\n\n") {
        "\n\n"
    } else {
        "\r\n\r\n"
    }
}

/// 生成一封默认邮件。
fn default_content(uid: u32, subject: &str, body: &str) -> String {
    // 日期由 UID 派生：同一天里的不同邮件落在不同日期上，
    // 这样 `SINCE` / `BEFORE` 这类日期搜索才有意义。
    let day = 1 + (uid % 28);
    let date = format!("{} Jan 2024 00:00:00 +0800", day);
    let weekday = weekday_for(2024, 1, day);

    format!(
        "From: sender@example.com\r\n\
         To: tester@example.com\r\n\
         Subject: {subject}\r\n\
         Date: {weekday}, {date}\r\n\
         Message-ID: <fixture-{uid}@example.com>\r\n\
         \r\n\
         {body}"
    )
}

/// 算星期几（Sakamoto 算法）。
fn weekday_for(year: i64, month: u32, day: u32) -> &'static str {
    const TABLE: [i64; 12] = [0, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    const NAMES: [&str; 7] = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];

    let y = if month < 3 { year - 1 } else { year };
    let index = (y + y / 4 - y / 100 + y / 400 + TABLE[(month - 1) as usize] + day as i64) % 7;
    NAMES[index.rem_euclid(7) as usize]
}

/// 解析 RFC 2822 日期里的日期部分（`Mon, 01 Jan 2024 00:00:00 +0800`）。
fn parse_header_date(value: &str) -> Option<i32> {
    let without_day_name = match value.find(',') {
        Some(index) => value[index + 1..].trim(),
        None => value.trim(),
    };
    let mut parts = without_day_name.split_whitespace();
    let day: u32 = parts.next()?.parse().ok()?;
    let month = month_number(parts.next()?)?;
    let year: i32 = parts.next()?.parse().ok()?;
    Some(year * 10_000 + month * 100 + day as i32)
}

/// 月份名 → 月份号。
fn month_number(name: &str) -> Option<i32> {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    MONTHS
        .iter()
        .position(|month| month.eq_ignore_ascii_case(name))
        .map(|index| index as i32 + 1)
}

/// 一个假文件夹。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MailboxFixture {
    /// 文件夹名（明文）。
    ///
    /// 传输时会按 IMAP 规则编码成 **modified UTF-7**（RFC 3501 §5.1.3），
    /// 中文名在线上是 `&XfJT0ZAB-` 这种形态。
    pub name: String,
    /// 特殊用途属性，如 `\Sent`。
    pub attributes: Vec<String>,
    /// 邮件。
    pub messages: Vec<MessageFixture>,
}

impl MailboxFixture {
    /// 新建一个空文件夹。
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            attributes: Vec::new(),
            messages: Vec::new(),
        }
    }

    /// 加上特殊用途属性。
    pub fn with_attribute(mut self, attribute: &str) -> Self {
        self.attributes.push(attribute.to_string());
        self
    }

    /// 放一封邮件。
    pub fn with_message(mut self, message: MessageFixture) -> Self {
        self.messages.push(message);
        self
    }

    /// 最大的 UID（`UID` 搜索与 sequence set 里的 `*` 用它）。
    pub fn max_uid(&self) -> u32 {
        self.messages.iter().map(|m| m.uid).max().unwrap_or(0)
    }
}

/// 故障注入。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fault {
    /// 登录成功后立刻断开连接。
    DropAfterLogin,
    /// 收到某条命令后**不响应**（客户端应当超时）。
    ///
    /// 命令名大写匹配，如 `"SELECT"`。
    HangOn(String),
    /// 对某条命令回 `NO`。
    Reject(String),
    /// 认证直接失败（模拟授权码错误）。
    RejectLogin,
    /// 认证成功但**不**回 `CAPABILITY`（模拟 163 认证前拒绝一切命令）。
    NoCapabilityResponse,
    /// 收到 `STORE` 后**回 OK 但不真的改**（“听话的假服务器”）。
    ///
    /// 模拟「服务器收了命令却没执行」——客户端只有**回读确认**
    /// 才能发现（见 ARCHITECTURE §6）。
    IgnoreStore,
    /// 收到某条命令后**直接断开连接**（不回任何响应）。
    ///
    /// 模拟「命令发出去了，但没收到服务器的确认」——
    /// 客户端**不知道**服务器到底执行了没（见 ARCHITECTURE §5）。
    DropOn(String),
}

/// 假服务器的配置。
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// 宣告的能力集。
    ///
    /// 模拟不同服务商的能力差异就改这里。
    ///
    /// 含 `CONDSTORE` 时才会启用 modseq / `UNCHANGEDSINCE` 语义。
    pub capabilities: Vec<String>,
    /// 允许登录的用户名。
    pub user: String,
    /// 允许登录的口令。
    pub password: String,
    /// 是否**要求**先发 `ID` 才能访问邮箱。
    ///
    /// 163 就是这样：不发 `ID` 时 `EXAMINE` / `SELECT` 一律被回
    /// `NO … Unsafe Login`。
    pub require_id: bool,
    /// 是否拒绝 `EXAMINE`（只允许 `SELECT`）。
    pub reject_examine: bool,
    /// UIDVALIDITY（epoch）。
    pub uid_validity: u32,
    /// 文件夹的**初始**内容。
    ///
    /// 服务器启动后会被复制成「活的状态」；`STORE` 改的是活的状态，
    /// 不会回写这里。
    pub mailboxes: Vec<MailboxFixture>,
    /// 故障注入。
    pub faults: Vec<Fault>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            capabilities: vec![
                "IMAP4rev1".to_string(),
                "ID".to_string(),
                "UIDPLUS".to_string(),
                "SPECIAL-USE".to_string(),
                "LITERAL+".to_string(),
            ],
            user: "tester@example.com".to_string(),
            password: "test-password".to_string(),
            require_id: false,
            reject_examine: false,
            uid_validity: 1,
            mailboxes: Vec::new(),
            faults: Vec::new(),
        }
    }
}

impl ServerConfig {
    /// 一个带默认文件夹的配置。
    pub fn with_mailboxes() -> Self {
        Self {
            mailboxes: vec![
                MailboxFixture::new("INBOX"),
                MailboxFixture::new("草稿箱").with_attribute("\\Drafts"),
                MailboxFixture::new("已发送").with_attribute("\\Sent"),
                MailboxFixture::new("已删除").with_attribute("\\Trash"),
                MailboxFixture::new("垃圾邮件").with_attribute("\\Junk"),
            ],
            ..Default::default()
        }
    }

    /// 是否宣告了某个能力。
    pub fn has_capability(&self, capability: &str) -> bool {
        self.capabilities
            .iter()
            .any(|c| c.eq_ignore_ascii_case(capability))
    }

    /// 是否触发某个故障。
    pub fn has_fault(&self, fault: &Fault) -> bool {
        self.faults.contains(fault)
    }

    /// 找出发送给某条命令的故障。
    pub fn fault_for(&self, command: &str) -> Option<&Fault> {
        let upper = command.to_ascii_uppercase();
        self.faults.iter().find(|fault| match fault {
            Fault::HangOn(cmd) | Fault::Reject(cmd) | Fault::DropOn(cmd) => {
                cmd.eq_ignore_ascii_case(&upper)
            }
            _ => false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_content_has_header_and_body() {
        let message = MessageFixture::new(42).with_body("正文内容");

        assert!(message.header().contains("Subject: 测试邮件"));
        assert!(
            message
                .header()
                .contains("Message-ID: <fixture-42@example.com>")
        );
        assert_eq!(message.body(), "正文内容");
        assert!(message.size() > 0);
    }

    #[test]
    fn with_subject_replaces_the_header() {
        let message = MessageFixture::new(1).with_subject("新主题");

        assert_eq!(message.header_values("Subject"), vec!["新主题"]);
        assert_eq!(
            message.header_values("Subject").len(),
            1,
            "不该出现重复的头"
        );
    }

    #[test]
    fn header_lookup_is_case_insensitive() {
        let message = MessageFixture::new(1).with_from("someone@example.com");

        assert_eq!(message.header_values("from"), vec!["someone@example.com"]);
        assert_eq!(message.header_values("FROM"), vec!["someone@example.com"]);
    }

    #[test]
    fn flags_are_case_insensitive() {
        let message = MessageFixture::new(1).with_flag("\\Seen");

        assert!(message.has_flag("\\seen"));
        assert!(message.has_flag("\\SEEN"));
        assert!(!message.has_flag("\\Flagged"));
    }

    #[test]
    fn dates_are_derived_from_uid() {
        let first = MessageFixture::new(1);
        let second = MessageFixture::new(2);

        assert_eq!(first.date(), Some(20240102));
        assert_eq!(second.date(), Some(20240103));
    }

    #[test]
    fn weekday_is_correct() {
        // 2024-01-01 是星期一。
        assert_eq!(weekday_for(2024, 1, 1), "Mon");
        assert_eq!(weekday_for(2024, 1, 7), "Sun");
        assert_eq!(weekday_for(2024, 2, 29), "Thu");
    }

    #[test]
    fn mailbox_reports_max_uid() {
        let mailbox = MailboxFixture::new("INBOX")
            .with_message(MessageFixture::new(10))
            .with_message(MessageFixture::new(99));

        assert_eq!(mailbox.max_uid(), 99);
        assert_eq!(MailboxFixture::new("空").max_uid(), 0);
    }

    #[test]
    fn capability_lookup_is_case_insensitive() {
        let config = ServerConfig::default();

        assert!(config.has_capability("imap4rev1"));
        assert!(!config.has_capability("CONDSTORE"));
    }
}
