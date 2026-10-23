//! 假 IMAP 服务器的配置。
//!
//! 配置项刻意做成**行为开关**，而不是「录制回放」：
//! 真实服务商的怪癖（163 拒绝 EXAMINE、要求先发 `ID`……）都能用开关复现，
//! 而录制回放只能复现一种情况。

/// 一封假邮件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageFixture {
    /// UID。**不保证**从 1 开始——163 的 UID 空间就很大。
    pub uid: u32,
    /// 标记，如 `\Seen`。
    pub flags: Vec<String>,
    /// 主题（仅用于调试输出；不进协议响应）。
    pub subject: String,
}

impl MessageFixture {
    /// 新建一封未读邮件。
    pub fn new(uid: u32) -> Self {
        Self {
            uid,
            flags: Vec::new(),
            subject: format!("测试邮件 {uid}"),
        }
    }

    /// 加上一个标记。
    pub fn with_flag(mut self, flag: &str) -> Self {
        self.flags.push(flag.to_string());
        self
    }
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
}

/// 假服务器的配置。
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// 宣告的能力集。
    ///
    /// 模拟不同服务商的能力差异就改这里。
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
    /// 文件夹。
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

    /// 是否触发某个故障。
    pub fn has_fault(&self, fault: &Fault) -> bool {
        self.faults.contains(fault)
    }

    /// 找出发送给某条命令的故障。
    pub fn fault_for(&self, command: &str) -> Option<&Fault> {
        let upper = command.to_ascii_uppercase();
        self.faults.iter().find(|fault| match fault {
            Fault::HangOn(cmd) | Fault::Reject(cmd) => cmd.eq_ignore_ascii_case(&upper),
            _ => false,
        })
    }
}
