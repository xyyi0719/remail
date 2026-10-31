//! Xremail SMTP 协议层。
//!
//! 职责边界（见 [ARCHITECTURE §2](../../../docs/ARCHITECTURE.md)）：
//! * SMTP 会话能力（EHLO 扩展、STARTTLS、AUTH）。
//! * **只读探测**（[`probe`]）与**真发信**（[`submit`]）两条路径。
//! * 不写库、**不决定重试**（那是 `xremail-sync` 的事）。
//! * 不认识 Tauri，也不认识 UI。
//!
//! ## 安全约定
//!
//! * **强制 TLS**：隐式 TLS（465）或明文 + STARTTLS（587）。
//!   STARTTLS 失败**不降级**——不会退回明文上认证
//!   （见 [SECURITY §2.2](../../../docs/SECURITY.md)）。
//! * 使用**系统根**校验服务端证书，不提供「跳过校验」的开关。
//! * 错误信息**不含**用户名或口令。
//! * 探测流程**严格只读**：只执行 `EHLO` / `STARTTLS` / `AUTH` / `QUIT`。
//!
//! ## 现在支持 / 不支持什么
//!
//! ### ESMTP 能力协商
//!
//! [`session::SmtpCapabilities`] 是**唯一的权威**：服务器在 `EHLO` 里宣告什么，
//! 客户端才用什么。
//!
//! | 能力 | 关键字 | 状态 |
//! |---|---|---|
//! | 8 位正文 | `8BITMIME`（RFC 6152） | ✅ 已支持：服务器宣告了才带 `BODY=8BITMIME`；没宣告而邮件确实是 8 位 → **报错** |
//! | UTF-8 地址 | `SMTPUTF8`（RFC 6531） | ✅ 已支持：地址含非 ASCII 时自动带 `SMTPUTF8`；服务器没宣告 → **报错**（不发出去） |
//! | 认证 | `AUTH PLAIN` / `AUTH LOGIN` | ✅ 已支持（按服务器宣告的机制选） |
//! | `XOAUTH2` | `AUTH XOAUTH2` | ❌ 未实现（需要 OAuth 令牌，不是账号口令） |
//! | 分块传输 | `BDAT` / `CHUNKING` | ❌ 未实现（固定用 `DATA`） |
//! | 传输层二进制 | `BINARYMIME` | ❌ 未实现 |
//!
//! ### 实测的各家服务器（2026-10，见 `docs/PROTOCOLS.md`）
//!
//! | 服务器 | TLS | `AUTH` | `8BITMIME` | `SMTPUTF8` |
//! |---|---|---|---|---|
//! | `smtp.163.com:465` | 1.3 | `LOGIN` / `PLAIN` / `XOAUTH2` | ✅ | ❌ |
//! | `smtp.qq.com:465` | 1.2 | `LOGIN` / `PLAIN` / `XOAUTH` / `XOAUTH2` | ✅ | ✅ |
//!
//! （两家都宣告了 `XOAUTH2`，但本客户端只实现了 `PLAIN` 与 `LOGIN`——
//! OAuth 令牌不是账号口令，机制不一样。）
//!
//! **后果**：非 ASCII 邮箱地址（比如国际域名、中文邮箱名）在 163 上**发不出去**。
//! 这不是 bug，是服务器没这个能力——客户端唯一正确的做法是**提前报错讲清楚**，
//! 而不是发一个必然被拒的事务。
//!
//! ### 不做的事
//!
//! * 不做 `RSET`（一次提交只发一封信）。
//! * 不做流式 `DATA`（正文先在内存里拼好）。
//! * 不做 `XOAUTH2`。
//! * 不做投递状态通知（`DSN`）的**请求**（服务器宣告了也只是记下来）。
//!
//! ## 为什么需要它
//!
//! QQ / 163 / Gmail 的 SMTP 参数——端口、STARTTLS 语义、AUTH 机制、
//! `8BITMIME` 支持度、服务端证书——**无法从代码推断，必须实测**。
//! 本 crate 加上 `xremail-smtp-probe` 就是干这个的：
//! 用户在自己机器上跑一次，把 markdown 报告贴回来。
//!
//! 详见 [`probe`]。

pub mod error;
pub mod probe;
pub mod session;
pub mod submit;
pub mod transport;

pub use error::SmtpError;
pub use probe::{
    AuthOutcome, Extension, ExtensionSupport, ProbeConfig, ProbeReport, TlsFallback,
    auth_mechanisms, extension_support, parse_extensions, probe, probe_with_trust,
};
pub use session::{
    AuthMechanism, MailFromParams, SmtpCapabilities, SmtpResponse, SmtpSession,
    address_requires_smtputf8,
};
pub use submit::{Envelope, SubmitOutcome, SubmitRequest, SubmitStage, submit_message};
pub use transport::{CertificateInfo, SmtpConfig, TlsInfo, TlsMode, TlsTrust, TlsVersions};
