//! 统一错误类型。
//!
//! 分层约定（见 [ARCHITECTURE §9](../../../docs/ARCHITECTURE.md)）：
//! 只有传输类与存储类错误重试；认证类立即停；用户输入类立即返回。

use std::fmt;

/// 本 crate 的 `Result` 别名。
pub type AppResult<T> = Result<T, AppError>;

/// 应用级错误。
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum AppError {
    /// 找不到请求的资源。
    #[error("not found: {0}")]
    NotFound(String),

    /// 调用方传入的参数不合法。
    #[error("invalid input: {0}")]
    Invalid(String),

    /// 网络传输失败，通常值得重试。
    #[error("network error: {0}")]
    Network(String),

    /// 认证被拒绝。**不要重试**，需要用户重新授权。
    #[error("authentication failed: {0}")]
    Auth(String),

    /// 协议层错误（服务器返回了非法或意外的响应）。
    #[error("protocol error: {0}")]
    Protocol(String),

    /// 加密 / 解密 / 密钥派生失败。
    #[error("crypto error: {0}")]
    Crypto(String),

    /// 本地存储错误，通常值得重试。
    #[error("storage error: {0}")]
    Storage(String),

    /// 文件系统错误。
    #[error("io error: {0}")]
    Io(String),

    /// 当前版本不支持该能力。
    #[error("unsupported: {0}")]
    Unsupported(String),

    /// 其它未归类错误。
    #[error("{0}")]
    Other(String),
}

impl AppError {
    /// 该错误是否值得自动重试。
    pub fn is_retryable(&self) -> bool {
        matches!(
            self,
            AppError::Network(_) | AppError::Storage(_) | AppError::Io(_)
        )
    }

    /// 该错误是否必须由用户介入处理。
    pub fn requires_user_action(&self) -> bool {
        matches!(self, AppError::Auth(_) | AppError::Invalid(_))
    }

    /// 附加上下文，同时**保留错误类别**。
    ///
    /// 保留类别是必要的：`is_retryable()` 与 `requires_user_action()` 依赖类别判断。
    /// 若把错误统一折叠成 `Other`，重试策略与“需要重新授权”的提示都会失效。
    pub fn with_context(self, msg: impl fmt::Display) -> Self {
        let prefix = msg.to_string();
        let wrap = |m: String| format!("{prefix}: {m}");
        match self {
            AppError::NotFound(m) => AppError::NotFound(wrap(m)),
            AppError::Invalid(m) => AppError::Invalid(wrap(m)),
            AppError::Network(m) => AppError::Network(wrap(m)),
            AppError::Auth(m) => AppError::Auth(wrap(m)),
            AppError::Protocol(m) => AppError::Protocol(wrap(m)),
            AppError::Crypto(m) => AppError::Crypto(wrap(m)),
            AppError::Storage(m) => AppError::Storage(wrap(m)),
            AppError::Io(m) => AppError::Io(wrap(m)),
            AppError::Unsupported(m) => AppError::Unsupported(wrap(m)),
            AppError::Other(m) => AppError::Other(wrap(m)),
        }
    }
}

impl From<std::io::Error> for AppError {
    fn from(value: std::io::Error) -> Self {
        AppError::Io(value.to_string())
    }
}

/// 便于在 `map_err` 中附加上下文。
pub trait Context<T> {
    /// 给错误附加上下文信息。
    fn ctx(self, msg: impl fmt::Display) -> AppResult<T>;
}

impl<T, E> Context<T> for Result<T, E>
where
    E: Into<AppError>,
{
    fn ctx(self, msg: impl fmt::Display) -> AppResult<T> {
        self.map_err(|e| e.into().with_context(msg))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ctx_preserves_retryable_category() {
        let err = Err::<(), _>(AppError::Network("connection reset".into()))
            .ctx("imap connect")
            .unwrap_err();

        assert!(err.is_retryable());
        assert!(!err.requires_user_action());
        assert!(err.to_string().contains("imap connect"));
    }

    #[test]
    fn ctx_preserves_user_action_category() {
        let err = Err::<(), _>(AppError::Auth("invalid credentials".into()))
            .ctx("login")
            .unwrap_err();

        assert!(err.requires_user_action());
        assert!(!err.is_retryable());
    }

    #[test]
    fn io_error_maps_to_retryable_io() {
        let err: AppError = std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "eof").into();

        assert!(err.is_retryable());
    }

    #[test]
    fn with_context_does_not_change_category() {
        let cases = [
            (AppError::Network("m".into()), true, false),
            (AppError::Storage("m".into()), true, false),
            (AppError::Io("m".into()), true, false),
            (AppError::Auth("m".into()), false, true),
            (AppError::Invalid("m".into()), false, true),
            (AppError::Protocol("m".into()), false, false),
        ];

        for (err, retryable, user_action) in cases {
            let err = err.with_context("ctx");
            assert_eq!(err.is_retryable(), retryable, "retryable mismatch");
            assert_eq!(
                err.requires_user_action(),
                user_action,
                "user action mismatch"
            );
        }
    }
}
