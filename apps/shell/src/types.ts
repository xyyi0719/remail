/**
 * 与 Rust 侧对应的类型定义。
 *
 * 命名规则：camelCase（`xremail-core` 的 `Account` 标了
 * `#[serde(rename_all = "camelCase")]`）。
 *
 * 枚举值保持 Rust 侧的显式 serde 名（`netease_163` 等）——
 * 那些字符串同时也是数据库列值，改动需要写迁移。
 */

/** 服务商标记。 */
export type ProviderKind =
  | "netease_163"
  | "netease_126"
  | "netease_enterprise"
  | "tencent"
  | "wo"
  | "google"
  | "exchange"
  | "generic";

/** 认证方式。 */
export type AuthKind = "password" | "oauth2";

/** 传输加密方式。 */
export type TlsMode = "none" | "starttls" | "implicit";

/** SMTP 认证方式。 */
export type SmtpAuth = "none" | "plain" | "login" | "xoauth2";

/** 同步方式。 */
export type SyncMode = "idle" | "poll";

/** 服务商显示名。 */
export const PROVIDER_LABELS: Record<ProviderKind, string> = {
  netease_163: "163 邮箱",
  netease_126: "126 邮箱",
  netease_enterprise: "企业邮",
  tencent: "QQ 邮箱",
  wo: "沃邮箱",
  google: "Gmail",
  exchange: "Exchange",
  generic: "其它",
};

/** 加密方式显示名。 */
export const TLS_LABELS: Record<TlsMode, string> = {
  none: "明文",
  starttls: "STARTTLS",
  implicit: "SSL/TLS",
};

/** SMTP 认证方式显示名。 */
export const SMTP_AUTH_LABELS: Record<SmtpAuth, string> = {
  none: "不认证",
  plain: "PLAIN",
  login: "LOGIN",
  xoauth2: "XOAUTH2",
};

/** 同步方式显示名。 */
export const SYNC_MODE_LABELS: Record<SyncMode, string> = {
  idle: "实时推送",
  poll: "轮询",
};

/** 服务器配置（对应 `xremail_core::model::ServerConfig`）。 */
export interface ServerConfig {
  imapHost: string;
  imapPort: number;
  imapTls: TlsMode;
  smtpHost: string;
  smtpPort: number;
  smtpTls: TlsMode;
  smtpAuth: SmtpAuth;
}

/** 一个账户（对应 `xremail_core::model::Account`）。 */
export interface Account {
  id: string;
  email: string;
  provider: ProviderKind;
  displayName: string | null;
  color: string | null;
  authKind: AuthKind;
  server: ServerConfig;
  username: string;
  /** 凭据的引用（**不是凭据本身**）；`null` = 凭据没落盘。 */
  secretRef: string | null;
  oauthClientId: string | null;
  syncMode: SyncMode;
  pollIntervalSecs: number;
  signatureHtml: string | null;
  createdAt: number;
  updatedAt: number;
}

/** 新增账户的请求（对应 `xremail_api::NewAccountRequest`）。 */
export interface NewAccountRequest {
  email: string;
  provider: ProviderKind;
  displayName?: string | null;
  imapHost: string;
  imapPort: number;
  imapTls: TlsMode;
  smtpHost: string;
  smtpPort: number;
  smtpTls: TlsMode;
  smtpAuth: SmtpAuth;
  username: string;
  syncMode: SyncMode;
  pollIntervalSecs: number;
}

/**
 * 一个文件夹（对应 `service::message::FolderDto`）。
 *
 * `name` 是**解码后**的显示名（如「草稿箱」）；
 * `rawName` 是线路上的原样（modified UTF-7），发命令时用。
 */
export interface Folder {
  id: string;
  name: string;
  rawName: string;
  delimiter: string | null;
  uidvalidity: number | null;
  lastUid: number | null;
  lastSyncAt: number | null;
}

/** 一封邮件（对应 `xremail_core::model::Message`）。 */
export interface Message {
  id: string;
  accountId: string;
  folderId: string;
  uid: number;
  uidvalidity: number;
  /** RFC Message-ID——**只是线索**（可能缺失、不唯一）。 */
  messageId: string | null;
  subject: string | null;
  snippet: string | null;
  from: string[];
  to: string[];
  cc: string[];
  bcc: string[];
  dateSent: number | null;
  internalDate: number | null;
  flags: number;
  size: number | null;
  hasAttachment: boolean;
  createdAt: number;
  updatedAt: number;
}

/** 一封邮件的正文（对应 `xremail_core::model::MessageBody`）。 */
export interface MessageBody {
  messageId: string;
  textPlain: string | null;
  /** **未经清洗**的原始 HTML——清洗是渲染层的事（ARCHITECTURE §7）。 */
  htmlRaw: string | null;
  charset: string | null;
}

/** 同步结果（对应 `cmd::SyncResult`）。 */
export interface SyncResult {
  folders: number;
  added: number;
  removed: number;
}

/** 应用基本信息（对应 Rust 侧的 `AppInfo`）。 */
export interface AppInfo {
  name: string;
  version: string;
  platform: string;
  dataDir: string | null;
}

/** IMAP 标记位（与 `xremail_core::model::MessageFlags` 一致）。 */
export const FLAG_SEEN = 1 << 0;
export const FLAG_ANSWERED = 1 << 1;
export const FLAG_FLAGGED = 1 << 2;
export const FLAG_DELETED = 1 << 3;
export const FLAG_DRAFT = 1 << 4;
export const FLAG_RECENT = 1 << 5;

/** 这封邮件是不是「未读」。 */
export function isUnread(message: Message): boolean {
  return (message.flags & FLAG_SEEN) === 0;
}

/** 有没有星标。 */
export function isStarred(message: Message): boolean {
  return (message.flags & FLAG_FLAGGED) !== 0;
}
