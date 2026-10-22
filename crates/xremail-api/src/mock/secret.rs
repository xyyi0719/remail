//! 内存版 [`SecretStore`]。

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;

use crate::error::SecretStoreError;
use crate::secret::{SecretStore, SecretValue};

/// 内存凭据存储。
///
/// 可用于测试，也可作为"没有平台钥匙串可用"时的**显式降级**实现——
/// 但降级必须由上层明确选择并告知用户，不能静默发生
/// （见 [SECURITY](../../../../docs/SECURITY.md)）。
#[derive(Debug, Default)]
pub struct InMemorySecretStore {
    entries: Mutex<HashMap<String, SecretValue>>,
}

impl InMemorySecretStore {
    /// 新建一个空的存储。
    pub fn new() -> Self {
        Self::default()
    }

    /// 当前条目数。
    pub fn len(&self) -> usize {
        self.entries.lock().expect("锁中毒").len()
    }

    /// 是否为空。
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[async_trait]
impl SecretStore for InMemorySecretStore {
    async fn get(&self, secret_ref: &str) -> Result<Option<SecretValue>, SecretStoreError> {
        let entries = self.entries.lock().expect("锁中毒");
        Ok(entries.get(secret_ref).cloned())
    }

    async fn set(&self, secret_ref: &str, value: &SecretValue) -> Result<(), SecretStoreError> {
        if secret_ref.is_empty() {
            return Err(SecretStoreError::Invalid("引用不能为空".into()));
        }

        let mut entries = self.entries.lock().expect("锁中毒");
        entries.insert(secret_ref.to_string(), value.clone());
        Ok(())
    }

    async fn delete(&self, secret_ref: &str) -> Result<bool, SecretStoreError> {
        let mut entries = self.entries.lock().expect("锁中毒");
        Ok(entries.remove(secret_ref).is_some())
    }
}
