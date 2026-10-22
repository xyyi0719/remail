//! bit 级 flag 意图的合并与命令规划。
//!
//! ## 为什么意图要单独存
//!
//! 服务器的 flag 快照与本地「用户还没提交的操作」是**两份不同的数据**，
//! 不能揉在一起（见 [ARCHITECTURE §6](../../../docs/ARCHITECTURE.md)）：
//!
//! * 快照：服务器现在认为这封信是什么状态。
//! * 意图：用户在本机做了什么，还没被服务器确认。
//!
//! UI 显示的是「快照 + 意图」的结果，但**不能**因此认为服务器已经接受
//! （见 [ARCHITECTURE §5](../../../docs/ARCHITECTURE.md)：
//! 「UI 可立即显示带 pending 标识的本地结果，不能把它显示为服务器确认」）。
//!
//! ## 冲突是真的会发生的
//!
//! `+Seen` 与 `-Seen` **可以冲突**——「单 bit 都幂等所以没有冲突」是错的。
//! 同一封信上可能既有用户手动标记，又有规则引擎的处理，顺序决定结果。
//! 本模块用 `seq`（本地单调序号）定序：序号大的意图覆盖序号小的。
//!
//! ## 只发变化的位
//!
//! §5 明确：「可核对状态后重放；**不要整组替换 FLAGS**」。
//! 所以规划出的命令只包含**净变化**的位：
//!
//! * 净变化为空 → 不发命令（`None`）。
//! * 只变了一个位 → 只发那一个位，其余位不碰。
//!
//! 这样重试也只重试真正需要重试的部分。

use xremail_core::AppError;
use xremail_core::model::MessageFlags;

/// 本地待提交的一个 bit 级 flag 意图。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FlagIntent {
    /// 目标邮件的 UID。
    pub uid: i64,
    /// 目标邮件所属的 **epoch**。
    ///
    /// 意图必须绑定 epoch：epoch 变了以后，旧 UID 指向的是另一封信，
    /// 旧意图**不能**套用（见 [ARCHITECTURE §4.3](../../../docs/ARCHITECTURE.md)）。
    pub uidvalidity: i64,
    /// 要置位的位。
    pub add: MessageFlags,
    /// 要清除的位。
    pub remove: MessageFlags,
    /// 本地单调序号。决定同一 bit 上多个意图的先后。
    pub seq: u64,
    /// 用户做出该意图时看到的基线 MODSEQ（诊断用）。
    pub base_modseq: Option<i64>,
}

impl FlagIntent {
    /// 构造一个意图。
    ///
    /// 若同一 bit 既要置位又要清除，则**自相矛盾**，返回
    /// [`AppError::Invalid`]。这种意图说明调用方有 bug，
    /// 静默挑一个语义会掩盖它。
    pub fn new(
        uid: i64,
        uidvalidity: i64,
        add: MessageFlags,
        remove: MessageFlags,
        seq: u64,
        base_modseq: Option<i64>,
    ) -> Result<Self, AppError> {
        let intent = Self {
            uid,
            uidvalidity,
            add,
            remove,
            seq,
            base_modseq,
        };

        if intent.is_contradictory() {
            return Err(AppError::Invalid(format!(
                "flag 意图自相矛盾：uid {uid} 的同一 bit 既要置位又要清除"
            )));
        }

        Ok(intent)
    }

    /// 是否自相矛盾（同一 bit 既置位又清除）。
    pub fn is_contradictory(&self) -> bool {
        !self.add.intersection(self.remove).is_empty()
    }
}

/// 一条 bit 级 flag 命令。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FlagDelta {
    /// 目标 UID。
    pub uid: i64,
    /// 要置位的位。
    pub add: MessageFlags,
    /// 要清除的位。
    pub remove: MessageFlags,
    /// 条件写用的基线 MODSEQ。
    ///
    /// 有 CONDSTORE 时用它做 `UNCHANGEDSINCE` 条件；服务器发现 MODSEQ 变了
    /// 会回 `MODIFIED`，此时应重新读取并按意图重算
    /// （见 [ARCHITECTURE §6](../../../docs/ARCHITECTURE.md)）。
    pub unchangedsince: Option<i64>,
}

impl FlagDelta {
    /// 命令是否为空（没有任何位要改）。
    pub fn is_empty(&self) -> bool {
        self.add.is_empty() && self.remove.is_empty()
    }
}

/// 一次 flag 更新的规划结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlagPlan {
    /// 要发给服务器的命令。净变化为空时为 `None`（不需要发命令）。
    pub delta: Option<FlagDelta>,
    /// 绑定到**旧 epoch**、必须被隔离的意图。
    ///
    /// 它们不参与本次计算：旧 UID 在新 epoch 下指向另一封信。
    pub stale: Vec<FlagIntent>,
}

/// 条件写被服务器拒绝（`MODIFIED`）后，重新读取快照并重算的次数上限。
///
/// §6：「MODIFIED 后重新读取并按意图重算，**重试有界**」。
/// 无界重试会在服务器持续被其他客户端修改时变成活锁。
pub const MAX_REPLAN_ATTEMPTS: usize = 3;

/// 把待提交意图按顺序作用到服务器快照上，得到**本地视图**的最终 flags。
///
/// 多个意图对同一 bit 时，按 `seq` 升序生效，序号大的覆盖序号小的。
/// `seq` 相同的按输入顺序（稳定排序），调用方应保证 `seq` 唯一。
///
/// **注意**：结果只是本地视图，不代表服务器已接受这些改动。
pub fn apply_intents(snapshot: MessageFlags, intents: &[FlagIntent]) -> MessageFlags {
    let mut ordered: Vec<&FlagIntent> = intents.iter().collect();
    ordered.sort_by_key(|intent| intent.seq);

    let mut effective = snapshot;
    for intent in ordered {
        effective = effective.union(intent.add).difference(intent.remove);
    }
    effective
}

/// 规划一次 flag 更新。
///
/// * 只考虑 `uid` 匹配**且** epoch 匹配的意图。
/// * 绑定旧 epoch 的意图进 [`FlagPlan::stale`]，不参与计算。
/// * 净变化为空时 `delta` 为 `None`——**不要发空命令**。
/// * `known_modseq` 是客户端最后看到的该邮件 MODSEQ，
///   有 CONDSTORE 时作为 `UNCHANGEDSINCE` 条件。
pub fn plan_flag_update(
    uid: i64,
    snapshot: MessageFlags,
    uidvalidity: i64,
    known_modseq: Option<i64>,
    intents: &[FlagIntent],
) -> FlagPlan {
    let mut matching = Vec::new();
    let mut stale = Vec::new();

    for intent in intents {
        if intent.uid != uid {
            continue;
        }
        if intent.uidvalidity == uidvalidity {
            matching.push(*intent);
        } else {
            stale.push(*intent);
        }
    }

    let effective = apply_intents(snapshot, &matching);
    let add = effective.difference(snapshot);
    let remove = snapshot.difference(effective);

    let delta = if add.is_empty() && remove.is_empty() {
        None
    } else {
        Some(FlagDelta {
            uid,
            add,
            remove,
            unchangedsince: known_modseq,
        })
    };

    FlagPlan { delta, stale }
}

/// 找出绑定到旧 epoch 的意图。
///
/// 这些意图不得套用到新 epoch 的 UID 上，应被隔离并保留冲突原因
/// （见 [ARCHITECTURE §4.3](../../../docs/ARCHITECTURE.md)）。
pub fn stale_intents(intents: &[FlagIntent], uidvalidity: i64) -> Vec<FlagIntent> {
    intents
        .iter()
        .filter(|intent| intent.uidvalidity != uidvalidity)
        .copied()
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seen() -> MessageFlags {
        MessageFlags::empty().with(MessageFlags::SEEN)
    }

    fn flagged() -> MessageFlags {
        MessageFlags::empty().with(MessageFlags::FLAGGED)
    }

    fn intent(
        uid: i64,
        epoch: i64,
        add: MessageFlags,
        remove: MessageFlags,
        seq: u64,
    ) -> FlagIntent {
        FlagIntent::new(uid, epoch, add, remove, seq, None).expect("意图不应矛盾")
    }

    // ---------------- 构造与矛盾 ----------------

    #[test]
    fn contradictory_intent_is_rejected() {
        let err = FlagIntent::new(1, 42, seen(), seen(), 1, None).unwrap_err();

        assert!(!err.is_retryable(), "自相矛盾的意图重试没有意义");
        assert!(err.requires_user_action());
    }

    #[test]
    fn non_contradictory_intent_is_accepted() {
        let ok = FlagIntent::new(1, 42, seen(), flagged(), 1, None);

        assert!(ok.is_ok());
        assert!(!ok.unwrap().is_contradictory());
    }

    // ---------------- 意图合并 ----------------

    #[test]
    fn no_intents_keeps_snapshot() {
        let effective = apply_intents(seen(), &[]);

        assert_eq!(effective, seen());
    }

    #[test]
    fn add_intent_sets_the_bit() {
        let effective = apply_intents(
            MessageFlags::empty(),
            &[intent(1, 42, seen(), MessageFlags::empty(), 1)],
        );

        assert!(effective.is_read());
    }

    #[test]
    fn remove_intent_clears_the_bit() {
        let effective = apply_intents(seen(), &[intent(1, 42, MessageFlags::empty(), seen(), 1)]);

        assert!(!effective.is_read());
    }

    #[test]
    fn later_seq_wins_for_the_same_bit() {
        // 先 +Seen 再 -Seen → 最终没有 Seen
        let intents = [
            intent(1, 42, seen(), MessageFlags::empty(), 1),
            intent(1, 42, MessageFlags::empty(), seen(), 2),
        ];

        let effective = apply_intents(MessageFlags::empty(), &intents);

        assert!(!effective.is_read(), "序号大的意图应覆盖序号小的");
    }

    #[test]
    fn earlier_seq_wins_when_order_is_reversed() {
        // 同样的两个意图，seq 决定顺序
        let intents = [
            intent(1, 42, MessageFlags::empty(), seen(), 2),
            intent(1, 42, seen(), MessageFlags::empty(), 1),
        ];

        let effective = apply_intents(MessageFlags::empty(), &intents);

        assert!(!effective.is_read());
    }

    #[test]
    fn intents_on_different_bits_accumulate() {
        let intents = [
            intent(1, 42, seen(), MessageFlags::empty(), 1),
            intent(1, 42, flagged(), MessageFlags::empty(), 2),
        ];

        let effective = apply_intents(MessageFlags::empty(), &intents);

        assert!(effective.is_read());
        assert!(effective.is_starred());
    }

    // ---------------- 命令规划 ----------------

    #[test]
    fn no_change_means_no_command() {
        let plan = plan_flag_update(1, seen(), 42, None, &[]);

        assert!(plan.delta.is_none(), "净变化为空时不应发命令");
        assert!(plan.stale.is_empty());
    }

    #[test]
    fn net_zero_after_add_then_remove_means_no_command() {
        // 用户点了"标记已读"，又点了"取消" → 净变化为零
        let intents = [
            intent(1, 42, seen(), MessageFlags::empty(), 1),
            intent(1, 42, MessageFlags::empty(), seen(), 2),
        ];

        let plan = plan_flag_update(1, MessageFlags::empty(), 42, None, &intents);

        assert!(plan.delta.is_none(), "净变化为零时不应发命令");
    }

    #[test]
    fn net_zero_after_remove_then_add_means_no_command() {
        let intents = [
            intent(1, 42, MessageFlags::empty(), seen(), 1),
            intent(1, 42, seen(), MessageFlags::empty(), 2),
        ];

        let plan = plan_flag_update(1, seen(), 42, None, &intents);

        assert!(plan.delta.is_none());
    }

    #[test]
    fn only_changed_bits_are_sent() {
        // 快照已有 Seen + Draft，本地只想加 Flagged
        let snapshot = seen().union(MessageFlags::empty().with(MessageFlags::DRAFT));

        let plan = plan_flag_update(
            1,
            snapshot,
            42,
            None,
            &[intent(1, 42, flagged(), MessageFlags::empty(), 1)],
        );

        let delta = plan.delta.expect("应有变化");
        assert!(delta.add.is_starred());
        assert!(
            delta.add.bits() == MessageFlags::FLAGGED,
            "只应发送变化的位"
        );
        assert!(delta.remove.is_empty(), "不应清除未变化的位");
    }

    #[test]
    fn delta_reports_the_removed_bit() {
        let plan = plan_flag_update(
            1,
            seen(),
            42,
            None,
            &[intent(1, 42, MessageFlags::empty(), seen(), 1)],
        );

        let delta = plan.delta.expect("应有变化");
        assert!(delta.remove.is_read());
        assert!(delta.add.is_empty());
    }

    #[test]
    fn delta_carries_the_known_modseq() {
        let plan = plan_flag_update(
            1,
            MessageFlags::empty(),
            42,
            Some(99),
            &[intent(1, 42, seen(), MessageFlags::empty(), 1)],
        );

        assert_eq!(plan.delta.unwrap().unchangedsince, Some(99));
    }

    #[test]
    fn delta_without_modseq_has_no_condition() {
        let plan = plan_flag_update(
            1,
            MessageFlags::empty(),
            42,
            None,
            &[intent(1, 42, seen(), MessageFlags::empty(), 1)],
        );

        assert_eq!(plan.delta.unwrap().unchangedsince, None);
    }

    #[test]
    fn intents_for_other_uids_are_ignored() {
        let intents = [intent(2, 42, seen(), MessageFlags::empty(), 1)];

        let plan = plan_flag_update(1, MessageFlags::empty(), 42, None, &intents);

        assert!(plan.delta.is_none(), "别的邮件的意图不该影响这封");
    }

    // ---------------- epoch 隔离 ----------------

    #[test]
    fn intents_bound_to_the_old_epoch_are_stale_and_excluded() {
        let intents = [intent(1, 42, seen(), MessageFlags::empty(), 1)];

        // 当前 epoch 已经是 99
        let plan = plan_flag_update(1, MessageFlags::empty(), 99, None, &intents);

        assert_eq!(plan.stale.len(), 1);
        assert_eq!(plan.stale[0].uidvalidity, 42);
        assert!(plan.delta.is_none(), "旧 epoch 的意图不得套用");
    }

    #[test]
    fn stale_intents_helper_finds_them() {
        let intents = [
            intent(1, 42, seen(), MessageFlags::empty(), 1),
            intent(2, 99, seen(), MessageFlags::empty(), 2),
        ];

        let stale = stale_intents(&intents, 99);

        assert_eq!(stale.len(), 1);
        assert_eq!(stale[0].uid, 1);
    }

    #[test]
    fn replanning_with_a_fresh_snapshot_yields_no_command_when_server_agrees() {
        // 场景：本地标记已读并提交，服务器返回 MODIFIED；
        // 重新读取快照后发现服务器其实已经是 Seen → 无需再发命令。
        let intents = [intent(1, 42, seen(), MessageFlags::empty(), 1)];

        let first = plan_flag_update(1, MessageFlags::empty(), 42, Some(1), &intents);
        assert!(first.delta.is_some(), "首次应当有变化要发");

        // 重新读取后快照已是 Seen
        let second = plan_flag_update(1, seen(), 42, Some(2), &intents);
        assert!(second.delta.is_none(), "服务器已达成一致时不应再发命令");
    }

    #[test]
    fn replanning_with_a_fresh_snapshot_still_needs_the_change() {
        // 服务器其实没改（快照仍是未读）→ 仍需发送
        let intents = [intent(1, 42, seen(), MessageFlags::empty(), 1)];

        let plan = plan_flag_update(1, MessageFlags::empty(), 42, Some(2), &intents);

        assert!(plan.delta.is_some());
    }

    #[test]
    fn replan_attempts_are_bounded() {
        // 策略常量：改变它应当是一个明确的决定，而不是顺手改个数。
        assert_eq!(MAX_REPLAN_ATTEMPTS, 3);
    }

    #[test]
    fn empty_delta_is_recognised() {
        let delta = FlagDelta {
            uid: 1,
            add: MessageFlags::empty(),
            remove: MessageFlags::empty(),
            unchangedsince: None,
        };

        assert!(delta.is_empty());
    }
}
