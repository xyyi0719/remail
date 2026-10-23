//! 平台凭据存储契约。
//!
//! 与两件东西**不是一回事**，别混：
//!
//! * **不是** SQLite 里的 `local_secret` 表——那是 [`crate::vault`] 的事。
//! * **不是** `xremail-crypto` 的密文封装——那是加解密的事。
//!
//! 这里的职责只有一件：把一条机密安全地放进操作系统的凭据保管设施
//! （Windows Credential Manager、Android Keystore、macOS Keychain），
//! 并能取回来。
//!
//! 参见 [ARCHITECTURE §2 与 §8](../../../docs/ARCHITECTURE.md)。

use async_trait::async_trait;
use zeroize::Zeroizing;

use crate::error::SecretStoreError;

/// 一条机密的值。
///
/// 内存中的机密在丢弃时会被清零，避免残留在堆里等被换出。
#[derive(Clone, PartialEq, Eq)]
pub struct SecretValue(Zeroizing<Vec<u8>>);

impl SecretValue {
    /// 用字节构造。
    pub fn new(bytes: impl Into<Vec<u8>>) -> Self {
        Self(Zeroizing::new(bytes.into()))
    }

    /// 用字符串构造。
    pub fn from_str_lossless(s: &str) -> Self {
        Self::new(s.as_bytes().to_vec())
    }

    /// 取出字节。
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// 取出 UTF-8 字符串。
    ///
    /// 返回 `None` 表示内容不是合法 UTF-8。
    /// **不**做有损转换——那会静默改掉凭据。
    /// 返回的是内部缓冲的借用，**不产生副本**（副本会绕过零化）。
    pub fn as_str(&self) -> Option<&str> {
        std::str::from_utf8(&self.0).ok()
    }

    /// 取出字节副本。
    pub fn to_vec(&self) -> Vec<u8> {
        self.0.to_vec()
    }

    /// 是否为空。
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl From<Vec<u8>> for SecretValue {
    fn from(value: Vec<u8>) -> Self {
        Self::new(value)
    }
}

impl std::fmt::Debug for SecretValue {
    /// **不打印内容**。机密出现在日志里就等于泄漏。
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SecretValue")
            .field("len", &self.0.len())
            .finish()
    }
}

/// 平台凭据存储。
///
/// 约定：
/// * `get` 对不存在的条目返回 `Ok(None)`——"没有"不是错误。
/// * `delete` 返回是否真的删掉了：区分"删掉了"与"本来就没有"。
/// * 失败语义明确：见 [`SecretStoreError`]。
#[async_trait]
pub trait SecretStore: Send + Sync {
    /// 读取一条机密。不存在时返回 `Ok(None)`。
    async fn get(&self, secret_ref: &str) -> Result<Option<SecretValue>, SecretStoreError>;

    /// 写入一条机密，覆盖同 `secret_ref` 的旧值。
    async fn set(&self, secret_ref: &str, value: &SecretValue) -> Result<(), SecretStoreError>;

    /// 删除一条机密。返回是否真的删掉了。
    async fn delete(&self, secret_ref: &str) -> Result<bool, SecretStoreError>;
}
