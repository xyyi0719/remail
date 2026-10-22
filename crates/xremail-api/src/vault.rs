//! 本地机密库（vault）契约。
//!
//! vault 存的是**已封装的密文**与元数据：
//!
//! * 封装本身是**不透明字节**——本层不关心它是用什么算法、什么格式封的，
//!   那是 `xremail-crypto` 的事。这样密钥管理与存储布局才能各自演进。
//! * `version` 记录封装格式的版本，用于将来迁移。
//!
//! vault **不等于**操作系统钥匙串（那是 [`crate::secret`] 的事）：
//! 前者是应用自己管理的密文存储，后者是系统保管的凭据。
//!
//! 参见 [ARCHITECTURE §8](../../../docs/ARCHITECTURE.md)。

use async_trait::async_trait;

use crate::error::VaultError;

/// vault 里的一条记录。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VaultEntry {
    /// 本地唯一 ID。
    pub id: String,
    /// 所属账户；`None` 表示不绑定具体账户的通用条目。
    pub account_id: Option<String>,
    /// 条目种类（如 `credential`、`label`、`rule`）。
    pub kind: String,
    /// 封装格式版本。
    pub version: u32,
    /// **不透明**的已封装密文。
    pub envelope: Vec<u8>,
    /// 创建时间，epoch 毫秒。
    pub created_at: i64,
    /// 更新时间，epoch 毫秒。
    pub updated_at: i64,
}

impl VaultEntry {
    /// 新建一条记录。
    pub fn new(
        id: impl Into<String>,
        kind: impl Into<String>,
        envelope: Vec<u8>,
        now: i64,
    ) -> Self {
        Self {
            id: id.into(),
            account_id: None,
            kind: kind.into(),
            version: 1,
            envelope,
            created_at: now,
            updated_at: now,
        }
    }

    /// 绑定到某个账户。
    pub fn with_account(mut self, account_id: impl Into<String>) -> Self {
        self.account_id = Some(account_id.into());
        self
    }
}

/// 本地机密库。
///
/// 约定：
/// * `put` 按 `id` 覆盖写入。
/// * `get` 对不存在的条目返回 `Ok(None)`。
/// * `delete` 返回是否真的删掉了。
#[async_trait]
pub trait VaultStore: Send + Sync {
    /// 写入（或覆盖）一条记录。
    async fn put(&self, entry: &VaultEntry) -> Result<(), VaultError>;

    /// 读取一条记录。不存在时返回 `Ok(None)`。
    async fn get(&self, id: &str) -> Result<Option<VaultEntry>, VaultError>;

    /// 列出全部记录。
    async fn list(&self) -> Result<Vec<VaultEntry>, VaultError>;

    /// 删除一条记录。返回是否真的删掉了。
    async fn delete(&self, id: &str) -> Result<bool, VaultError>;
}
