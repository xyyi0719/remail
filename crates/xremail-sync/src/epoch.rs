//! epoch（UIDVALIDITY）切换。
//!
//! UIDVALIDITY 是服务器给一个文件夹的**世代标识**。它一变，
//! 之前记下的所有 UID 就全部作废了——同一个 UID 数字在新世代里指的是另一封信。
//!
//! 因此 epoch 变化**不是**一次普通的增量更新，而是一次**重建**：
//! 在一个数据库事务内切换 epoch、失效旧 UID 映射与游标、
//! 隔离绑定旧 epoch 的待提交操作（见 [ARCHITECTURE §4.3](../../../docs/ARCHITECTURE.md)）。
//!
//! ## 最重要的一条
//!
//! **禁止把旧操作直接套到新 UID。** 用户在旧 epoch 下标记的"已读"，
//! 指向的是旧世代的那封信；套到新 UID 上会作用到**另一封信**上。
//! 本模块把这些操作识别出来（[`EpochTransition::Reset`] 的 `quarantined`），
//! 由上层保留记录与冲突原因，供恢复或提示用户。
//!
//! 另两条相关约束：
//!
//! * 本地时间戳 / HLC **不能**排序普通 IMAP 客户端的远端操作
//!   （见 [ARCHITECTURE §6](../../../docs/ARCHITECTURE.md)）。
//! * MODSEQ 不是跨 mailbox 的全局时间。

use xremail_core::AppError;

/// 本地记录的 epoch 与游标。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StoredEpoch {
    /// 服务器的 UIDVALIDITY。`None` 表示这个文件夹**从未同步过**。
    pub uidvalidity: Option<i64>,
    /// 已提交的最大 UID。
    pub last_uid: Option<i64>,
    /// 已知的最高 MODSEQ。
    pub highest_modseq: Option<i64>,
}

impl StoredEpoch {
    /// 尚未同步过的状态。
    pub fn unsynced() -> Self {
        Self::default()
    }

    /// 已同步到某个 epoch 的状态。
    pub fn at(uidvalidity: i64) -> Self {
        Self {
            uidvalidity: Some(uidvalidity),
            ..Self::default()
        }
    }
}

/// 绑定到某个 epoch 的待提交操作。
///
/// 这里只保留决策需要的最小信息：操作 ID 与它绑定的 epoch。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingOperationBinding {
    /// 操作 ID。
    pub id: String,
    /// 该操作绑定的 epoch。
    pub uidvalidity: i64,
}

impl PendingOperationBinding {
    /// 构造一个绑定。
    pub fn new(id: impl Into<String>, uidvalidity: i64) -> Self {
        Self {
            id: id.into(),
            uidvalidity,
        }
    }
}

/// epoch 切换的处置方案。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EpochTransition {
    /// epoch 未变：继续增量同步。
    Unchanged,
    /// 首次同步：本地没有这个文件夹的任何缓存。
    Initial {
        /// 服务器当前的 UIDVALIDITY。
        uidvalidity: i64,
    },
    /// epoch 变了：必须重建该文件夹的快照。
    Reset {
        /// 旧的 UIDVALIDITY。
        old_uidvalidity: i64,
        /// 服务器当前的 UIDVALIDITY。
        new_uidvalidity: i64,
        /// 因 epoch 失效而必须隔离的待提交操作 ID。
        ///
        /// 这些操作**不能**套用到新 epoch 的 UID 上。
        /// 记录与冲突原因要保留下来，供恢复或提示用户。
        quarantined: Vec<String>,
    },
}

impl EpochTransition {
    /// 是否需要**全量拉取**该文件夹。
    ///
    /// 首次同步（[`EpochTransition::Initial`]）与 epoch 变化
    /// （[`EpochTransition::Reset`]）都需要；只有 epoch 未变才走增量。
    pub fn requires_full_fetch(&self) -> bool {
        matches!(
            self,
            EpochTransition::Initial { .. } | EpochTransition::Reset { .. }
        )
    }

    /// 是否可以沿用已有游标，只拉取变化部分。
    pub fn can_continue_incremental(&self) -> bool {
        matches!(self, EpochTransition::Unchanged)
    }

    /// 是否需要重建该文件夹的快照。
    pub fn requires_reset(&self) -> bool {
        matches!(self, EpochTransition::Reset { .. })
    }

    /// 需要隔离的操作 ID。
    pub fn quarantined_operations(&self) -> &[String] {
        match self {
            EpochTransition::Reset { quarantined, .. } => quarantined,
            _ => &[],
        }
    }
}

/// 规划 epoch 切换。
///
/// * 本地没有 epoch（从未同步）→ [`EpochTransition::Initial`]
/// * epoch 未变 → [`EpochTransition::Unchanged`]
/// * epoch 变了 → [`EpochTransition::Reset`]，并列出绑定旧 epoch 的待提交操作
///
/// `observed_uidvalidity` 必须是正数：IMAP 的 UIDVALIDITY 不得为 0
/// （见 RFC 3501 §2.3.1.1），收到 0 说明响应有问题。
pub fn plan_epoch_transition(
    stored: &StoredEpoch,
    observed_uidvalidity: i64,
    pending: &[PendingOperationBinding],
) -> Result<EpochTransition, AppError> {
    if observed_uidvalidity <= 0 {
        return Err(AppError::Protocol(format!(
            "UIDVALIDITY 必须是正数，收到 {observed_uidvalidity}"
        )));
    }

    let Some(old_uidvalidity) = stored.uidvalidity else {
        // 首次同步：本地没有旧 UID 映射，也就没有绑定旧 epoch 的操作。
        return Ok(EpochTransition::Initial {
            uidvalidity: observed_uidvalidity,
        });
    };

    if old_uidvalidity == observed_uidvalidity {
        return Ok(EpochTransition::Unchanged);
    }

    // epoch 变了：绑定旧 epoch 的操作必须隔离，不能套用到新 UID 上。
    let mut quarantined: Vec<String> = pending
        .iter()
        .filter(|op| op.uidvalidity == old_uidvalidity)
        .map(|op| op.id.clone())
        .collect();
    quarantined.sort();

    Ok(EpochTransition::Reset {
        old_uidvalidity,
        new_uidvalidity: observed_uidvalidity,
        quarantined,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_sync_is_initial() {
        let transition = plan_epoch_transition(&StoredEpoch::unsynced(), 42, &[]).unwrap();

        assert_eq!(transition, EpochTransition::Initial { uidvalidity: 42 });
        assert!(!transition.requires_reset(), "没有旧快照可重建");
        assert!(
            transition.requires_full_fetch(),
            "首次同步是全量拉取，不是增量"
        );
        assert!(!transition.can_continue_incremental(), "没有可沿用的游标");
    }

    #[test]
    fn first_sync_quarantines_nothing() {
        // 从未同步过，就不存在绑定旧 epoch 的操作。
        let pending = vec![PendingOperationBinding::new("op1", 7)];

        let transition = plan_epoch_transition(&StoredEpoch::unsynced(), 42, &pending).unwrap();

        assert!(transition.quarantined_operations().is_empty());
    }

    #[test]
    fn same_epoch_is_unchanged() {
        let stored = StoredEpoch::at(42);

        let transition = plan_epoch_transition(&stored, 42, &[]).unwrap();

        assert_eq!(transition, EpochTransition::Unchanged);
        assert!(transition.can_continue_incremental());
        assert!(!transition.requires_reset());
    }

    #[test]
    fn changed_epoch_requires_reset() {
        let stored = StoredEpoch::at(42);

        let transition = plan_epoch_transition(&stored, 99, &[]).unwrap();

        assert_eq!(
            transition,
            EpochTransition::Reset {
                old_uidvalidity: 42,
                new_uidvalidity: 99,
                quarantined: vec![],
            }
        );
        assert!(transition.requires_reset());
        assert!(transition.requires_full_fetch());
        assert!(!transition.can_continue_incremental());
    }

    #[test]
    fn operations_bound_to_the_old_epoch_are_quarantined() {
        let stored = StoredEpoch::at(42);
        let pending = vec![
            PendingOperationBinding::new("op-old", 42),
            PendingOperationBinding::new("op-new", 99),
        ];

        let transition = plan_epoch_transition(&stored, 99, &pending).unwrap();

        assert_eq!(transition.quarantined_operations(), &["op-old".to_string()]);
    }

    #[test]
    fn quarantine_ids_are_sorted() {
        let stored = StoredEpoch::at(42);
        let pending = vec![
            PendingOperationBinding::new("z", 42),
            PendingOperationBinding::new("a", 42),
            PendingOperationBinding::new("m", 42),
        ];

        let transition = plan_epoch_transition(&stored, 99, &pending).unwrap();

        assert_eq!(
            transition.quarantined_operations(),
            &["a".to_string(), "m".to_string(), "z".to_string()]
        );
    }

    #[test]
    fn operations_bound_to_other_epochs_are_not_quarantined() {
        let stored = StoredEpoch::at(42);
        // 绑定在更早的 epoch 上的操作同样不能套用，但它们不是本次切换的责任对象；
        // 这里只隔离「本次被作废的那个 epoch」。
        let pending = vec![PendingOperationBinding::new("op-ancient", 7)];

        let transition = plan_epoch_transition(&stored, 99, &pending).unwrap();

        assert!(transition.quarantined_operations().is_empty());
    }

    #[test]
    fn zero_uidvalidity_is_rejected() {
        let stored = StoredEpoch::at(42);

        let err = plan_epoch_transition(&stored, 0, &[]).unwrap_err();

        assert!(!err.is_retryable(), "协议层错误重试没有意义");
    }

    #[test]
    fn negative_uidvalidity_is_rejected() {
        let stored = StoredEpoch::at(42);

        let err = plan_epoch_transition(&stored, -1, &[]).unwrap_err();

        assert!(!err.is_retryable());
    }

    #[test]
    fn zero_uidvalidity_is_rejected_even_on_first_sync() {
        let err = plan_epoch_transition(&StoredEpoch::unsynced(), 0, &[]).unwrap_err();

        assert!(!err.is_retryable());
    }

    #[test]
    fn stored_epoch_keeps_cursor_fields() {
        let stored = StoredEpoch {
            uidvalidity: Some(42),
            last_uid: Some(100),
            highest_modseq: Some(7),
        };

        let transition = plan_epoch_transition(&stored, 42, &[]).unwrap();

        assert_eq!(transition, EpochTransition::Unchanged);
        assert_eq!(stored.last_uid, Some(100));
        assert_eq!(stored.highest_modseq, Some(7));
    }
}
