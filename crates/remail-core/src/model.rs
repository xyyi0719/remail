//! Domain model for ReMail.
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    /// 163.com personal mailboxes.
    Netease163,
    /// 126.com personal mailboxes.
    Netease126,
    /// NetEase enterprise mail (Coremail-like, region specific hosts).
    NeteaseEnterprise,
    /// QQ Mail (Tencent).
    Tencent,
    /// Wo Mail (Coremail).
    Wo,
    /// Gmail / Google Workspace.
    ///
    /// Workspace 账号自 2025-05-01 起禁用密码基础认证；个人账号在开启两步验证后
    /// 仍可用“应用专用密码”走基础认证。Gmail 当前暂缓接入，见 ADR-0003。
    Google,
    /// Microsoft Exchange / Outlook 365.
    Exchange,
    /// Anything else, configured manually.
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthKind {
    /// Password or provider-issued authorisation code (163/QQ/Wo).
    Password,
    /// OAuth 2.0 with the `XOAUTH2` SASL mechanism (Google/Microsoft).
    OAuth2,
}

/// Transport security of a protocol endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TlsMode {
    /// Plaintext (never used for real accounts).
    None,
    /// Opportunistic TLS: connect in plaintext, then `STARTTLS`.
    Starttls,
    /// Implicit TLS: connect directly on a TLS socket (IMAPS/SMTPS).
    Implicit,
}

/// SMTP authentication mechanism.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SmtpAuth {
    /// No authentication (rarely useful).
    None,
    /// `AUTH PLAIN`.
    Plain,
    /// `AUTH LOGIN`.
    Login,
    /// `AUTH XOAUTH2`.
    XOAuth2,
}

/// How aggressively the client keeps the IMAP connection open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncMode {
    /// Long-lived `IDLE` connection; lowest latency, higher battery use.
    Idle,
    /// Periodic polling; gentler on servers and on battery.
    Poll,
}

/// One mailbox / directory endpoint pair.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
}

impl Folder {
    /// Display name, honouring the user's override.
    pub fn label(&self) -> &str {
        self.local_name.as_deref().unwrap_or(&self.remote_name)
    }
}

/// An email address with an optional display name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
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
}

/// A message envelope as stored locally.
///
/// The body lives in a separate table (`message_body`) so that listing a
/// folder never has to touch large text columns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
pub struct MessageBody {
    pub message_id: String,
    pub text_plain: Option<String>,
    /// Raw, un-sanitised HTML. Sanitisation happens in the render layer.
    pub html_raw: Option<String>,
    pub charset: Option<String>,
}

/// An attachment descriptor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
