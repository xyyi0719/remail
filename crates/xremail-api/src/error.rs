//! 错误类型。
//!
//! 三个契约的错误**故意分开定义**：它们的失败原因与处置方式完全不同，
//! 混成一个大枚举会让调用方难以判断"该不该重试"。
//!
//! 共同原则（见 [ARCHITECTURE §9](../../../docs/ARCHITECTURE.md)）：
//! 错误按**可恢复原因**分类；不确定结果不伪装成确定结果。

use thiserror::Error;

/// 平台凭据存储的错误。
///
/// 凭据存储（操作系统钥匙串）与普通 SQLite 存储是**两回事**：
/// 前者失败通常意味着"这台机器上拿不到密钥"，不是"稍后重试就好"
/// （见 [ARCHITECTURE §8](../../../docs/ARCHITECTURE.md)）。
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum SecretStoreError {
    /// 底层钥匙串不可用（服务未运行、权限不足、被系统锁定等）。
    #[error("secret store unavailable: {0}")]
    Unavailable(String),

    /// 凭据已存在且不允许覆盖，或访问被拒绝。
    #[error("secret store access denied: {0}")]
    Denied(String),

    /// 凭据条目格式非法。
    #[error("invalid secret reference: {0}")]
    Invalid(String),
}

impl SecretStoreError {
    /// 是否值得重试。
    pub fn is_retryable(&self) -> bool {
        matches!(self, SecretStoreError::Unavailable(_))
    }
}

/// 本地机密库（vault）的错误。
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum VaultError {
    /// 底层存储失败。
    #[error("vault storage error: {0}")]
    Storage(String),

    /// 条目内容不合法。
    #[error("invalid vault entry: {0}")]
    Invalid(String),
}

impl VaultError {
    /// 是否值得重试。
    pub fn is_retryable(&self) -> bool {
        matches!(self, VaultError::Storage(_))
    }
}

/// 远端同步后端的错误。
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum SyncError {
    /// **冲突**：远端当前状态与提交方预期的不一致。
    ///
    /// 这**不是**传输故障，重试同一份提交不会成功；
    /// 调用方必须重新读取远端状态、合并意图后再提交。
    ///
    /// `current` 是远端当前的修订号；`None` 表示远端没有该条目。
    #[error("sync conflict (remote revision: {current:?})")]
    Conflict {
        /// 远端当前修订号，`None` 表示远端尚无该条目。
        current: Option<crate::sync::Revision>,
    },

    /// 传输失败（网络、握手、超时）。
    #[error("sync transport error: {0}")]
    Transport(String),

    /// 鉴权失败，需要用户重新授权。
    #[error("sync unauthorized: {0}")]
    Unauthorized(String),

    /// 远端明确拒绝（载荷过大、超出配额、条目不存在等）。
    #[error("sync rejected: {0}")]
    Rejected(String),
}

impl SyncError {
    /// 是否值得重试。
    ///
    /// 冲突**不**在列：重试同一份提交只会再次冲突。
    pub fn is_retryable(&self) -> bool {
        matches!(self, SyncError::Transport(_))
    }

    /// 是否为冲突。
    pub fn is_conflict(&self) -> bool {
        matches!(self, SyncError::Conflict { .. })
    }
}
