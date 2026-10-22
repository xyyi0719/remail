//! 内存版 [`SyncBackend`]。
//!
//! CAS 的实现要点：**判断与写入在同一把锁内完成**。
//! 若把"读当前修订号"和"写新值"拆成两步，中间就有可被观察的间隙，
//! 那正是 TOCTOU 的温床（见 [ARCHITECTURE §8](../../../../docs/ARCHITECTURE.md)）。

use std::collections::HashMap;
use std::sync::Mutex;

use async_trait::async_trait;

use crate::error::SyncError;
use crate::sync::{RemoteEntry, Revision, SyncBackend};

/// 远端条目与其修订号。
#[derive(Debug, Clone, PartialEq, Eq)]
struct Stored {
    payload: Vec<u8>,
    revision: Revision,
}

/// 内存同步后端。
///
/// 多个"设备"共享同一个实例即可模拟同一台远端服务上的并发写入。
#[derive(Debug, Default)]
pub struct InMemorySyncBackend {
    entries: Mutex<HashMap<String, Stored>>,
    /// 修订号计数器。**整个后端共享**，不是每个条目一个。
    revision_counter: Mutex<u64>,
}

impl InMemorySyncBackend {
    /// 新建一个空的后端。
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

    /// 分配下一个修订号。
    ///
    /// 与条目表分开加锁：分配修订号不必持有条目表的锁。
    fn next_revision(&self) -> Revision {
        let mut counter = self.revision_counter.lock().expect("锁中毒");
        *counter += 1;
        Revision(*counter)
    }
}

#[async_trait]
impl SyncBackend for InMemorySyncBackend {
    async fn read(&self, key: &str) -> Result<Option<RemoteEntry>, SyncError> {
        let entries = self.entries.lock().expect("锁中毒");
        Ok(entries.get(key).map(|stored| RemoteEntry {
            payload: stored.payload.clone(),
            revision: stored.revision,
        }))
    }

    async fn compare_and_swap(
        &self,
        key: &str,
        expected: Option<Revision>,
        payload: &[u8],
    ) -> Result<Revision, SyncError> {
        // 关键：判断与写入在**同一把锁**内完成，中间没有可观察的间隙。
        let mut entries = self.entries.lock().expect("锁中毒");

        let current = entries.get(key).map(|stored| stored.revision);
        if current != expected {
            return Err(SyncError::Conflict { current });
        }

        let revision = self.next_revision();
        entries.insert(
            key.to_string(),
            Stored {
                payload: payload.to_vec(),
                revision,
            },
        );
        Ok(revision)
    }
}
