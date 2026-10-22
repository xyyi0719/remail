//! 内存版 [`VaultStore`]。

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;

use crate::error::VaultError;
use crate::vault::{VaultEntry, VaultStore};

/// 内存机密库。
#[derive(Debug, Default)]
pub struct InMemoryVaultStore {
    entries: Mutex<HashMap<String, VaultEntry>>,
}

impl InMemoryVaultStore {
    /// 新建一个空的库。
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
impl VaultStore for InMemoryVaultStore {
    async fn put(&self, entry: &VaultEntry) -> Result<(), VaultError> {
        if entry.id.is_empty() {
            return Err(VaultError::Invalid("id 不能为空".into()));
        }
        if entry.envelope.is_empty() {
            return Err(VaultError::Invalid("封装不能为空".into()));
        }

        let mut entries = self.entries.lock().expect("锁中毒");
        entries.insert(entry.id.clone(), entry.clone());
        Ok(())
    }

    async fn get(&self, id: &str) -> Result<Option<VaultEntry>, VaultError> {
        let entries = self.entries.lock().expect("锁中毒");
        Ok(entries.get(id).cloned())
    }

    async fn list(&self) -> Result<Vec<VaultEntry>, VaultError> {
        let entries = self.entries.lock().expect("锁中毒");
        // 稳定排序：列表顺序不应随哈希表迭代顺序抖动。
        let mut list: Vec<VaultEntry> = entries.values().cloned().collect();
        list.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(list)
    }

    async fn delete(&self, id: &str) -> Result<bool, VaultError> {
        let mut entries = self.entries.lock().expect("锁中毒");
        Ok(entries.remove(id).is_some())
    }
}
