/**
 * 与 Rust 侧领域模型对应的类型定义。
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

/** 传输安全方式。 */
export type TlsMode = "none" | "starttls" | "implicit";

/** SMTP 认证方式。 */
export type SmtpAuth = "none" | "plain" | "login" | "xoauth2";

/** 同步方式。 */
export type SyncMode = "idle" | "poll";

/** IMAP / SMTP 服务器配置。 */
export interface ServerConfig {
  imapHost: string;
  imapPort: number;
  imapTls: TlsMode;
  smtpHost: string;
  smtpPort: number;
  smtpTls: TlsMode;
  smtpAuth: SmtpAuth;
}

/** 一个已配置的账户。 */
export interface Account {
  id: string;
  email: string;
  provider: ProviderKind;
  displayName: string | null;
  color: string | null;
  authKind: AuthKind;
  server: ServerConfig;
  username: string;
  /** 操作系统钥匙串中机密的不透明引用；P0 恒为 null。 */
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

/** 应用基本信息（对应 Rust 侧的 `AppInfo`）。 */
export interface AppInfo {
  name: string;
  version: string;
  platform: string;
  dataDir: string | null;
}

/** 服务商的中文显示名。 */
export const PROVIDER_LABELS: Record<ProviderKind, string> = {
  netease_163: "163 邮箱",
  netease_126: "126 邮箱",
  netease_enterprise: "网易企业邮",
  tencent: "QQ 邮箱",
  wo: "沃邮箱",
  google: "Gmail",
  exchange: "Exchange",
  generic: "自定义 IMAP",
};

export const TLS_LABELS: Record<TlsMode, string> = {
  none: "明文",
  starttls: "STARTTLS",
  implicit: "隐式 TLS",
};

export const SMTP_AUTH_LABELS: Record<SmtpAuth, string> = {
  none: "无认证",
  plain: "AUTH PLAIN",
  login: "AUTH LOGIN",
  xoauth2: "AUTH XOAUTH2",
};

export const SYNC_MODE_LABELS: Record<SyncMode, string> = {
  idle: "IDLE 长连接",
  poll: "定时轮询",
};
