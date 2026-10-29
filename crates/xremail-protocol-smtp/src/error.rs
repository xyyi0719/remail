//! SMTP 协议层错误。
//!
//! 分类原则与 IMAP 层保持一致（见 [ARCHITECTURE §9](../../../docs/ARCHITECTURE.md)）：
//! 错误按**可恢复原因**分类，且**不得**把凭据带进错误信息。
//!
//! 与 IMAP 层的对应关系：
//!
//! | 情况 | IMAP | SMTP |
//! |---|---|---|
//! | 建连失败 | `ProtocolError::Connect` | `SmtpError::Connect` |
//! | TLS 失败（**不降级**） | `ProtocolError::Tls` | `SmtpError::Tls` |
//! | 传输层 IO | `ProtocolError::Io` | `SmtpError::Io` |
//! | 协议层响应不对 | `ProtocolError::Imap` | `SmtpError::Smtp` |
//! | 凭据被拒 | `ProtocolError::Auth` | `SmtpError::Auth` |
//! | 超时 | `ProtocolError::Timeout` | `SmtpError::Timeout` |
//! | 入参不合法 | `ProtocolError::Invalid` | `SmtpError::Invalid` |
//! | 能力不支持 | `ProtocolError::Unsupported` | `SmtpError::Unsupported` |

use thiserror::Error;

/// SMTP 协议层错误。
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum SmtpError {
    /// TCP 建连失败。
    #[error("连接失败: {0}")]
    Connect(String),

    /// TLS 握手或证书校验失败。
    ///
    /// **不降级**：证书校验失败就是失败，不提供「跳过校验」的开关
    /// （见 [SECURITY §2.2](../../../docs/SECURITY.md)）。
    #[error("TLS 失败: {0}")]
    Tls(String),

    /// 服务器**拒绝**了我们的 TLS ClientHello（回了 fatal alert）。
    ///
    /// 这**不是证书问题**——证书校验失败会归到 [`SmtpError::Tls`]。
    /// 服务器拒绝 ClientHello 有两种常见原因：
    ///
    /// 1. **不认这个协议版本**：只认 TLS 1.2 的服务器见到 TLS 1.3 的
    ///    ClientHello 直接回 `HandshakeFailure`。换版本重试就能连上
    ///    （[`TlsVersions::Tls12Only`](crate::TlsVersions)）。
    /// 2. **不认我们的密码套件**：极少数老服务器要求 ClientHello 里
    ///    带**静态 RSA 密钥交换**套件（`TLS_RSA_WITH_*`），否则直接断。
    ///    这时换版本**没用**——rustls 根本不支持静态 RSA 密钥交换。
    ///
    /// 换版本重试与证书校验**一点不放松**：这正是它与 [`SmtpError::Tls`]
    /// 的区别——那个换版本也救不了，只能报错。
    #[error("服务器拒绝了 TLS 握手: {0}")]
    TlsVersionRejected(String),

    /// 底层 IO 错误。
    #[error("IO 失败: {0}")]
    Io(String),

    /// SMTP 协议层错误（服务器返回了非法或意外的响应）。
    #[error("SMTP 协议错误: {0}")]
    Smtp(String),

    /// 认证被拒绝。
    ///
    /// **不重试**：需要用户重新确认凭据。
    #[error("认证失败: {0}")]
    Auth(String),

    /// 操作超时。
    #[error("操作超时: {0}")]
    Timeout(String),

    /// 入参不合法。
    #[error("无效的输入: {0}")]
    Invalid(String),

    /// 服务器不支持所需能力。
    ///
    /// 最典型的场景：服务器没有宣告 `STARTTLS`，而我们要求先升级再认证——
    /// 这时**必须报错**，不能悄悄在明文上继续认证。
    #[error("服务器不支持该能力: {0}")]
    Unsupported(String),
}

impl SmtpError {
    /// 是否值得重试。
    ///
    /// 认证失败与协议错误**不**在列：重试同样的输入只会得到同样的错误。
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            SmtpError::Connect(_)
                | SmtpError::Io(_)
                | SmtpError::Timeout(_)
                | SmtpError::TlsVersionRejected(_)
        )
    }

    /// 是否需要用户介入（重新提供凭据等）。
    pub fn requires_user_action(&self) -> bool {
        matches!(self, SmtpError::Auth(_))
    }
}
