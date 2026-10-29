//! Xremail SMTP 协议层。
//!
//! 职责边界（见 [ARCHITECTURE §2](../../../docs/ARCHITECTURE.md)）：
//! * SMTP 会话能力（EHLO 扩展、STARTTLS、AUTH）。
//! * **不发信**：本 crate 只实现「只读探测」所需的命令。
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
pub mod transport;

pub use error::SmtpError;
pub use probe::{
    AuthOutcome, Extension, ExtensionSupport, ProbeConfig, ProbeReport, TlsFallback,
    auth_mechanisms, extension_support, parse_extensions, probe, probe_with_trust,
};
pub use session::{AuthMechanism, SmtpResponse, SmtpSession};
pub use transport::{CertificateInfo, SmtpConfig, TlsInfo, TlsMode, TlsTrust, TlsVersions};
