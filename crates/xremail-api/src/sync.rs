//! 远端同步后端契约。
//!
//! 这里是 P6（可选的 L2 加密同步）的接口预留，**当前不建设远端服务**
//! （见 [ARCHITECTURE §8](../../../docs/ARCHITECTURE.md)）。
//!
//! ## 为什么要条件提交
//!
//! 远端条目带一个**修订号（revision）**。提交时必须带上"我读到的那个修订号"，
//! 由后端**原子地**判断：修订号仍是我读到的那个才写入，否则报冲突。
//!
//! 常见的错误做法是"先查冲突，再无条件推送"：
//!
//! ```text
//!   A 读 rev=1  ──┐
//!                 ├─ A 检查：冲突了吗？没有 ── A 推送 rev=1 的内容
//!   B 读 rev=1  ──┘        ↑ 这中间 B 也推送了，A 的推送把 B 的覆盖掉了
//! ```
//!
//! 检查与推送之间状态可能已变，这叫 TOCTOU（检查时间与使用时间不一致），
//! 后果是**静默丢失更新**。因此本契约
//!
//! * 只提供**原子的** [`SyncBackend::compare_and_swap`]；
//! * **刻意不提供**"检查冲突"与"推送"的组合方法。
//!
//! 想要冲突信息的正确姿势是：CAS 失败时返回的 [`SyncError::Conflict`]
//! 里就带着远端当前的修订号，调用方据此重新读取并合并。

use async_trait::async_trait;

use crate::error::SyncError;

/// 远端条目的修订号。
///
/// 单调递增。修订号**不**代表时间顺序，也不跨条目有意义，
/// 只用于"我读到的还是这一份吗"的条件判断。
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Revision(pub u64);

impl Revision {
    /// 下一个修订号。
    pub fn next(self) -> Self {
        Revision(self.0 + 1)
    }
}

/// 远端的一条条目。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteEntry {
    /// 已封装的密文（不透明字节）。
    pub payload: Vec<u8>,
    /// 当前修订号。
    pub revision: Revision,
}

/// 远端同步后端。
#[async_trait]
pub trait SyncBackend: Send + Sync {
    /// 读取远端条目。条目不存在时返回 `Ok(None)`。
    async fn read(&self, key: &str) -> Result<Option<RemoteEntry>, SyncError>;

    /// **原子地**条件提交。
    ///
    /// * `expected = None`：要求远端**尚无**该条目（首次写）。
    /// * `expected = Some(r)`：要求远端当前修订号**正好**是 `r`。
    ///
    /// 成功时返回写入后的新修订号。
    /// 条件不满足时返回 [`SyncError::Conflict`]，其中 `current` 是远端当前修订号
    /// （`None` 表示远端没有该条目）。
    ///
    /// 本方法**必须**是原子的：判断与写入之间不能有可被观察的间隙。
    async fn compare_and_swap(
        &self,
        key: &str,
        expected: Option<Revision>,
        payload: &[u8],
    ) -> Result<Revision, SyncError>;
}
