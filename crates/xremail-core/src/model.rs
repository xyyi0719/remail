//! Domain model for Xremail.
//!
//! This crate is intentionally dependency-light: it must not depend on any
//! storage, network or UI crate, so that every other crate in the workspace
//! can depend on it without creating cycles.

use serde::{Deserialize, Serialize};

/// Well-known account providers.
///
/// The enum is a *preset* marker only — every field of the account is
/// overridable, because real-world deployments differ per region and per
/// mailbox tier (see `docs/PROTOCOLS.md`).
///
/// **每个变体都有显式的 `serde` 名称**：这些字符串会作为 SQLite 的列值落库，
/// 若依赖 `rename_all` 自动推导，变体改名会让旧行读不出来（静默丢数据）。
/// 改名时必须同时写迁移。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ProviderKind {
    /// 163 mailboxes (`@163.com`, personal).
    #[serde(rename = "netease_163")]
    Netease163,
    /// 126 mailboxes (`@126.com`, personal).
    #[serde(rename = "netease_126")]
    Netease126,
    /// 网易企业邮 mailboxes (`@qiye.163.com`): Coremail-like, region-specific hosts.
    #[serde(rename = "netease_enterprise")]
    NeteaseEnterprise,
    /// QQ mailboxes (`@qq.com`).
    #[serde(rename = "tencent")]
    Tencent,
    /// 沃邮箱 mailboxes (`@wo.cn`): Coremail-based.
    #[serde(rename = "wo")]
    Wo,
    /// Gmail / Workspace mailboxes.
    ///
    /// Workspace 账号自 2025-05-01 起禁用密码基础认证；个人账号在开启两步验证后
    /// 仍可用“应用专用密码”走基础认证。Gmail 当前暂缓接入，见 ADR-0003。
    #[serde(rename = "google")]
    Google,
    /// Exchange / Outlook 365 mailboxes.
    #[serde(rename = "exchange")]
    Exchange,
    /// Anything else, configured manually.
    #[serde(rename = "generic")]
    Generic,
}

impl ProviderKind {
    /// Human readable name, used in the UI.
    pub const fn display_name(self) -> &'static str {
        match self {
            ProviderKind::Netease163 => "163 邮箱",
            ProviderKind::Netease126 => "126 邮箱",
            ProviderKind::NeteaseEnterprise => "网易企业邮",
            ProviderKind::Tencent => "QQ 邮箱",
            ProviderKind::Wo => "沃邮箱",
            ProviderKind::Google => "Gmail",
            ProviderKind::Exchange => "Exchange",
            ProviderKind::Generic => "自定义 IMAP",
        }
    }

    /// Whether this provider can only be authenticated through OAuth2.
    pub const fn requires_oauth(self) -> bool {
        matches!(self, ProviderKind::Google | ProviderKind::Exchange)
    }
}

/// How the client authenticates against IMAP/SMTP.
///
/// 显式 `serde` 名称：见 [`ProviderKind`] 的说明（这些值会落库）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuthKind {
    /// Password or provider-issued authorisation code (163/QQ/Wo).
    #[serde(rename = "password")]
    Password,
    /// OAuth 2.0 with the `XOAUTH2` SASL mechanism (Gmail / Exchange).
    #[serde(rename = "oauth2")]
    OAuth2,
}

/// Transport security of a protocol endpoint.
///
/// 显式 `serde` 名称：见 [`ProviderKind`] 的说明（这些值会落库）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TlsMode {
    /// Plaintext (never used for real accounts).
    #[serde(rename = "none")]
    None,
    /// Opportunistic TLS: connect in plaintext, then `STARTTLS`.
    #[serde(rename = "starttls")]
    Starttls,
    /// Implicit TLS: connect directly on a TLS socket (IMAPS/SMTPS).
    #[serde(rename = "implicit")]
    Implicit,
}

/// SMTP authentication mechanism.
///
/// 显式 `serde` 名称：见 [`ProviderKind`] 的说明（这些值会落库）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SmtpAuth {
    /// No authentication (rarely useful).
    #[serde(rename = "none")]
    None,
    /// `AUTH PLAIN`.
    #[serde(rename = "plain")]
    Plain,
    /// `AUTH LOGIN`.
    #[serde(rename = "login")]
    Login,
    /// `AUTH XOAUTH2`.
    #[serde(rename = "xoauth2")]
    XOAuth2,
}

/// How aggressively the client keeps the IMAP connection open.
///
/// 显式 `serde` 名称：见 [`ProviderKind`] 的说明（这些值会落库）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SyncMode {
    /// Long-lived `IDLE` connection; lowest latency, higher battery use.
    #[serde(rename = "idle")]
    Idle,
    /// Periodic polling; gentler on servers and on battery.
    #[serde(rename = "poll")]
    Poll,
}

/// One mailbox / directory endpoint pair.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerConfig {
    pub imap_host: String,
    pub imap_port: u16,
    pub imap_tls: TlsMode,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub smtp_tls: TlsMode,
    pub smtp_auth: SmtpAuth,
}

impl ServerConfig {
    pub fn new(
        imap_host: impl Into<String>,
        imap_port: u16,
        smtp_host: impl Into<String>,
        smtp_port: u16,
    ) -> Self {
        Self {
            imap_host: imap_host.into(),
            imap_port,
            imap_tls: TlsMode::Implicit,
            smtp_host: smtp_host.into(),
            smtp_port,
            smtp_tls: TlsMode::Implicit,
            smtp_auth: SmtpAuth::Login,
        }
    }
}

/// A configured mail account.
///
/// **serde 命名**：对外（前端、事件载荷）用 camelCase。
/// 落库时**不**用这套命名——`xremail-store` 有自己的列映射，
/// 所以这里的 wire 格式与存储格式互不影响。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Account {
    pub id: String,
    pub email: String,
    pub provider: ProviderKind,
    pub display_name: Option<String>,
    /// Optional per-account accent colour (hex).
    pub color: Option<String>,
    pub auth_kind: AuthKind,
    pub server: ServerConfig,
    /// The user name presented to the server (usually the full address).
    pub username: String,
    /// Opaque reference to a secret held in the OS keychain — never the secret
    /// itself. `None` means "not stored on this device yet".
    pub secret_ref: Option<String>,
    pub oauth_client_id: Option<String>,
    pub sync_mode: SyncMode,
    pub poll_interval_secs: i64,
    pub signature_html: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl Account {
    /// Label shown in the UI; falls back to the address.
    pub fn label(&self) -> &str {
        self.display_name.as_deref().unwrap_or(&self.email)
    }
}

/// A mailbox folder (`INBOX`, `Sent`, custom labels, ...).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Folder {
    pub id: String,
    pub account_id: String,
    /// Raw server-side name, including any hierarchy separator.
    pub remote_name: String,
    /// Extended name returned by `LIST`.
    pub remote_id: Option<String>,
    /// User-visible override.
    pub local_name: Option<String>,
    /// Hierarchy delimiter reported by the server (`/` or `.`).
    pub delimiter: Option<String>,
    pub subscribed: bool,
    pub selectable: bool,
    pub last_sync_at: Option<i64>,
    pub last_uid: Option<i64>,
    pub highest_modseq: Option<i64>,
    /// 服务器侧 UIDVALIDITY，即本文件夹的 **epoch**。
    ///
    /// 变更表示服务器已作废旧的 UID 映射，必须在一个事务内切换 epoch、
    /// 失效旧游标并隔离绑定旧 epoch 的待提交操作
    /// （见 [ARCHITECTURE §4.3](../../../docs/ARCHITECTURE.md)）。
    ///
    /// `None` 表示尚未与该文件夹完成过一次同步，因而还不知道 epoch。
    pub uidvalidity: Option<i64>,
}

impl Folder {
    /// Display name, honouring the user's override.
    pub fn label(&self) -> &str {
        self.local_name.as_deref().unwrap_or(&self.remote_name)
    }
}

/// An email address with an optional display name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Address {
    pub name: Option<String>,
    pub email: String,
}

impl Address {
    pub fn new(email: impl Into<String>) -> Self {
        Self {
            name: None,
            email: email.into(),
        }
    }
}

/// IMAP system flags, stored as a bitmask.
///
/// Layout matches the IMAP `FLAGS` special-use attributes so that mapping to
/// and from `imap-proto` types is a direct cast.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
pub struct MessageFlags(pub u32);

impl MessageFlags {
    pub const SEEN: u32 = 1 << 0;
    pub const ANSWERED: u32 = 1 << 1;
    pub const FLAGGED: u32 = 1 << 2;
    pub const DELETED: u32 = 1 << 3;
    pub const DRAFT: u32 = 1 << 4;
    pub const RECENT: u32 = 1 << 5;

    pub const fn empty() -> Self {
        Self(0)
    }

    pub const fn contains(self, flag: u32) -> bool {
        self.0 & flag != 0
    }

    pub const fn with(self, flag: u32) -> Self {
        Self(self.0 | flag)
    }

    pub const fn without(self, flag: u32) -> Self {
        Self(self.0 & !flag)
    }

    /// `true` when no bit is set.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// The raw bit mask.
    pub const fn bits(self) -> u32 {
        self.0
    }

    /// Union: every bit of both sets.
    pub const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }

    /// Intersection: the bits both sets share.
    pub const fn intersection(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }

    /// Difference: `self` without the bits of `other`.
    ///
    /// 用于算「快照 → 最终」之间的**净变化**：只发送真正变化的位，
    /// 而不是整组替换 FLAGS（见 [ARCHITECTURE §6](../../../docs/ARCHITECTURE.md)）。
    pub const fn difference(self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }

    /// `true` when the message has been read.
    pub const fn is_read(self) -> bool {
        self.contains(Self::SEEN)
    }

    /// `true` when the message is starred.
    pub const fn is_starred(self) -> bool {
        self.contains(Self::FLAGGED)
    }

    /// `true` when the message is marked deleted.
    pub const fn is_deleted(self) -> bool {
        self.contains(Self::DELETED)
    }

    /// 位掩码 → IMAP 系统标记名。
    ///
    /// 顺序固定（按位从低到高），便于断言与日志比对。
    /// 自定义关键字（`$Label1` 之类）没有对应位，**不在这里**——
    /// 位掩码表达不了它们。
    pub fn to_imap_names(self) -> Vec<&'static str> {
        let mut names = Vec::new();
        for (bit, name) in [
            (Self::SEEN, "\\Seen"),
            (Self::ANSWERED, "\\Answered"),
            (Self::FLAGGED, "\\Flagged"),
            (Self::DELETED, "\\Deleted"),
            (Self::DRAFT, "\\Draft"),
            (Self::RECENT, "\\Recent"),
        ] {
            if self.contains(bit) {
                names.push(name);
            }
        }
        names
    }

    /// IMAP 系统标记名 → 位掩码。
    ///
    /// 认不出的名字（自定义关键字）**忽略**，不是错误：
    /// 服务器可能给邮件打上应用自���的关键字，客户端不认识就不管。
    pub fn from_imap_names(names: &[impl AsRef<str>]) -> Self {
        let mut flags = Self::empty();
        for name in names {
            let name = name.as_ref();
            let bit = if name.eq_ignore_ascii_case("\\Seen") {
                Self::SEEN
            } else if name.eq_ignore_ascii_case("\\Answered") {
                Self::ANSWERED
            } else if name.eq_ignore_ascii_case("\\Flagged") {
                Self::FLAGGED
            } else if name.eq_ignore_ascii_case("\\Deleted") {
                Self::DELETED
            } else if name.eq_ignore_ascii_case("\\Draft") {
                Self::DRAFT
            } else if name.eq_ignore_ascii_case("\\Recent") {
                Self::RECENT
            } else {
                continue;
            };
            flags = flags.with(bit);
        }
        flags
    }
}

/// A message envelope as stored locally.
///
/// The body lives in a separate table (`message_body`) so that listing a
/// folder never has to touch large text columns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub id: String,
    pub account_id: String,
    pub folder_id: String,
    /// IMAP UID, unique within `(folder, uidvalidity)`.
    pub uid: i64,
    /// Server-side UIDVALIDITY; a change invalidates all cached UIDs.
    pub uidvalidity: i64,
    /// RFC 5322 `Message-ID`。
    ///
    /// **仅作线索**：它不保证唯一、可能缺失，跨文件夹也不唯一。
    /// 去重必须用 `(account_id, folder_id, uidvalidity, uid)`；
    /// 详见 [ARCHITECTURE §4.1](../../../docs/ARCHITECTURE.md)。
    pub message_id: Option<String>,
    pub subject: Option<String>,
    pub snippet: Option<String>,
    pub from: Vec<Address>,
    pub to: Vec<Address>,
    pub cc: Vec<Address>,
    pub bcc: Vec<Address>,
    /// `Date` header, epoch milliseconds.
    pub date_sent: Option<i64>,
    /// `INTERNALDATE`, epoch milliseconds.
    pub internal_date: Option<i64>,
    pub flags: MessageFlags,
    pub size: Option<i64>,
    pub has_attachment: bool,
}

/// 本地待提交操作的状态机。
///
/// **这些取值就是 `mail_operation.state` 列的取值**，两者必须一致：
/// 改枚举名就得配一个数据迁移（见 [ARCHITECTURE §5](../../../docs/ARCHITECTURE.md)）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OperationState {
    /// 已排队，尚未提交。
    #[serde(rename = "queued")]
    Queued,
    /// 正在提交。
    #[serde(rename = "running")]
    Running,
    /// 服务器已确认。
    #[serde(rename = "succeeded")]
    Succeeded,
    /// 失败，但可以重试（网络故障、服务器忙……）。
    #[serde(rename = "retryable")]
    Retryable,
    /// 失败，且不该重试（如 epoch 变了、意图自相矛盾）。
    #[serde(rename = "failed")]
    Failed,
    /// 不知道服务器到底执行了没有。
    ///
    /// 例如：命令发出去了，但没收到服务器的确认。
    /// **不能盲目重试**——可能已经执行过了（见 ARCHITECTURE §5）。
    #[serde(rename = "uncertain")]
    Uncertain,
}

impl OperationState {
    /// 是否还没完结（需要继续处理）。
    pub fn is_pending(self) -> bool {
        matches!(
            self,
            OperationState::Queued | OperationState::Running | OperationState::Retryable
        )
    }

    /// 是否是终态。
    pub fn is_terminal(self) -> bool {
        !self.is_pending()
    }

    /// 是否还可以重试。
    pub fn is_retryable(self) -> bool {
        matches!(self, OperationState::Queued | OperationState::Retryable)
    }
}

/// 本地待提交操作的类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum OperationKind {
    /// 标记变更（bit 级增删）。
    #[serde(rename = "flags")]
    Flags,
}

/// 标记变更的意图载荷。
///
/// 存进 `mail_operation.intent` 列（JSON 文本）。
///
/// **只是意图，不是命令**：它说「用户想让这封信变成什么样」，
/// 实际要发什么命令由同步层根据服务器快照算出来
/// （见 [ARCHITECTURE §6](../../../docs/ARCHITECTURE.md)）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct FlagIntentPayload {
    /// 要置位的位。
    pub add: MessageFlags,
    /// 要清除的位。
    pub remove: MessageFlags,
}

impl FlagIntentPayload {
    /// 新建一个意图载荷。
    pub fn new(add: MessageFlags, remove: MessageFlags) -> Self {
        Self { add, remove }
    }

    /// 是否自相矛盾（同一 bit 既置位又清除）。
    pub fn is_contradictory(&self) -> bool {
        !self.add.intersection(self.remove).is_empty()
    }
}

/// 一条本地待提交的操作。
///
/// 对应 `mail_operation` 表。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MailOperation {
    /// 本地 ID。
    pub id: String,
    /// 所属账户。
    pub account_id: String,
    /// 所属文件夹。
    pub folder_id: String,
    /// 目标邮件的 UID。
    pub uid: i64,
    /// 目标邮件所属的 **epoch**。
    ///
    /// epoch 变了以后，旧 UID 指向的是另一封信，
    /// 旧操作**不能**套用（见 [ARCHITECTURE §4.3](../../../docs/ARCHITECTURE.md)）。
    pub uidvalidity: i64,
    /// 操作类型。
    pub kind: OperationKind,
    /// 意图载荷。
    pub intent: FlagIntentPayload,
    /// 状态。
    pub state: OperationState,
    /// 本地单调序号——同一封邮件上多个意图的定序依据。
    pub seq: i64,
    /// 已知的服务器 MODSEQ 基线；无 CONDSTORE 时为 `None`。
    pub base_version: Option<i64>,
    /// 已尝试次数。
    pub attempts: i64,
    /// 失败分类。
    pub error_kind: Option<String>,
    /// 失败详情。
    pub error_message: Option<String>,
    /// 创建时间（epoch 毫秒）。
    pub created_at: i64,
    /// 更新时间（epoch 毫秒）。
    pub updated_at: i64,
}

/// Lifecycle state of a per-account synchronisation actor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncState {
    Idle,
    Connecting,
    Authenticating,
    Syncing,
    /// Connected and either idling or polling.
    Online,
    /// Lost the connection, backing off before the next attempt.
    Reconnecting,
    /// The server rejected our credentials; a human has to act.
    ReauthRequired,
    /// Gave up after repeated failures.
    Failed,
}

/// A message body, stored separately from its envelope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageBody {
    pub message_id: String,
    pub text_plain: Option<String>,
    /// Raw, un-sanitised HTML. Sanitisation happens in the render layer.
    pub html_raw: Option<String>,
    pub charset: Option<String>,
}

/// An attachment descriptor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Attachment {
    pub id: String,
    pub message_id: String,
    pub filename: Option<String>,
    pub mime: Option<String>,
    pub size: Option<i64>,
    /// `Content-ID` for inline parts.
    pub content_id: Option<String>,
    pub inline: bool,
    /// Path of the downloaded blob, `None` while not yet fetched.
    pub local_path: Option<String>,
    pub sha256: Option<String>,
    pub fetched: bool,
}

#[cfg(test)]
mod flags_tests {
    use super::MessageFlags;

    const SEEN: u32 = MessageFlags::SEEN;
    const FLAGGED: u32 = MessageFlags::FLAGGED;
    const DRAFT: u32 = MessageFlags::DRAFT;

    #[test]
    fn union_keeps_every_bit() {
        let a = MessageFlags::empty().with(SEEN);
        let b = MessageFlags::empty().with(FLAGGED);

        let u = a.union(b);

        assert!(u.contains(SEEN));
        assert!(u.contains(FLAGGED));
    }

    #[test]
    fn intersection_keeps_only_shared_bits() {
        let a = MessageFlags::empty().with(SEEN).with(FLAGGED);
        let b = MessageFlags::empty().with(FLAGGED).with(DRAFT);

        let i = a.intersection(b);

        assert!(!i.contains(SEEN));
        assert!(i.contains(FLAGGED));
        assert!(!i.contains(DRAFT));
    }

    #[test]
    fn difference_removes_the_other_bits() {
        let a = MessageFlags::empty().with(SEEN).with(FLAGGED);
        let b = MessageFlags::empty().with(FLAGGED);

        let d = a.difference(b);

        assert!(d.contains(SEEN));
        assert!(!d.contains(FLAGGED));
    }

    #[test]
    fn difference_is_not_symmetric() {
        let a = MessageFlags::empty().with(SEEN).with(FLAGGED);
        let b = MessageFlags::empty().with(FLAGGED);

        // a 去掉 b 的位 → 只剩 SEEN
        assert!(a.difference(b).contains(SEEN));
        assert!(!a.difference(b).contains(FLAGGED));
        // b 去掉 a 的位 → 空
        assert!(b.difference(a).is_empty());
    }

    #[test]
    fn empty_set_is_empty() {
        assert!(MessageFlags::empty().is_empty());
        assert!(!MessageFlags::empty().with(SEEN).is_empty());
    }

    #[test]
    fn bits_round_trips() {
        let f = MessageFlags::empty().with(SEEN).with(DRAFT);

        assert_eq!(f.bits(), SEEN | DRAFT);
    }

    #[test]
    fn difference_of_equal_sets_is_empty() {
        let a = MessageFlags::empty().with(SEEN).with(FLAGGED);

        assert!(a.difference(a).is_empty());
    }
}
