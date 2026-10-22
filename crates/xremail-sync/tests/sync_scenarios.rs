//! T2 的跨模块场景测试：把 UID 差分、epoch 切换、bit 意图合起来跑一遍同步周期。
//!
//! 单元测试覆盖单个函数的边界；这里覆盖**它们串起来时**的不变量——
//! 正是这类组合最容易出错的地方。
//!
//! 场景都来自 [ARCHITECTURE §4 与 §5、§6](../../../docs/ARCHITECTURE.md)，
//! 不依赖真实邮箱、凭据、网络或本机路径
//! （见 [TESTING_STRATEGY §5.2](../../../docs/TESTING_STRATEGY.md)）。

use std::collections::BTreeSet;

use xremail_core::model::MessageFlags;
use xremail_sync::{
    EpochTransition, FlagIntent, PendingOperationBinding, StoredEpoch, UidSet, apply_intents,
    diff_uids, plan_epoch_transition, plan_flag_update, stale_intents,
};

fn seen() -> MessageFlags {
    MessageFlags::empty().with(MessageFlags::SEEN)
}

fn flagged() -> MessageFlags {
    MessageFlags::empty().with(MessageFlags::FLAGGED)
}

// ===========================================================================
// 场景一：首次同步
// ===========================================================================

#[test]
fn scenario_first_sync_fetches_everything() {
    // 本地没有这个文件夹的任何缓存。
    let stored = StoredEpoch::unsynced();
    let server_uids = UidSet::from_uids([1, 2, 3, 4, 5]);

    let transition = plan_epoch_transition(&stored, 42, &[]).unwrap();
    assert_eq!(transition, EpochTransition::Initial { uidvalidity: 42 });
    assert!(transition.requires_full_fetch());

    // 首次同步：服务器上的全部 UID 都要拉。
    let diff = diff_uids(&UidSet::new(), &server_uids);
    assert_eq!(diff.added, vec![1, 2, 3, 4, 5]);
    assert!(diff.removed.is_empty());

    // 分批拉取，避免一次拉太多。
    let batches = diff.chunk_added(2);
    assert_eq!(batches.len(), 3);
    assert!(batches.iter().all(|b| b.len() <= 2));
}

// ===========================================================================
// 场景二：增量同步
// ===========================================================================

#[test]
fn scenario_incremental_sync_fetches_only_new_uids() {
    let stored = StoredEpoch::at(42);
    let local = UidSet::from_uids([1, 2, 3]);
    let server = UidSet::from_uids([1, 2, 3, 4]);

    let transition = plan_epoch_transition(&stored, 42, &[]).unwrap();
    assert!(transition.can_continue_incremental());

    let diff = diff_uids(&local, &server);
    assert_eq!(diff.added, vec![4]);
    assert!(diff.removed.is_empty());
    assert_eq!(diff.retained, vec![1, 2, 3]);
}

// ===========================================================================
// 场景三：服务器删除
// ===========================================================================

#[test]
fn scenario_server_deletion_keeps_local_drafts() {
    let stored = StoredEpoch::at(42);
    // 本地有 1、2、3；服务器只剩 1。
    let local = UidSet::from_uids([1, 2, 3]);
    let server = UidSet::from_uids([1]);

    let transition = plan_epoch_transition(&stored, 42, &[]).unwrap();
    assert!(transition.can_continue_incremental());

    let mut diff = diff_uids(&local, &server);
    assert_eq!(diff.removed, vec![2, 3]);

    // 但 2 是本地草稿，有独立归属，不能因为服务器上没有就清掉。
    let protected: BTreeSet<i64> = [2].into_iter().collect();
    diff.retain_local(&protected);

    assert_eq!(diff.removed, vec![3], "草稿 2 应被保留");
    assert_eq!(diff.retained, vec![1]);
}

// ===========================================================================
// 场景四：epoch 变化
// ===========================================================================

#[test]
fn scenario_epoch_change_resets_snapshot_and_quarantines_old_intents() {
    // 本地在 epoch 42 下缓存了 1..3，并有一个待提交的"标记已读"意图。
    let stored = StoredEpoch::at(42);
    let local = UidSet::from_uids([1, 2, 3]);
    let old_intent = FlagIntent::new(1, 42, seen(), MessageFlags::empty(), 1, None).unwrap();
    let pending = vec![PendingOperationBinding::new("op1", 42)];

    // 服务器换成了 epoch 99，UID 集合完全不同。
    let server = UidSet::from_uids([7, 8]);

    let transition = plan_epoch_transition(&stored, 99, &pending).unwrap();
    assert!(transition.requires_reset());
    assert!(transition.requires_full_fetch());
    assert_eq!(transition.quarantined_operations(), &["op1".to_string()]);

    // 旧 epoch 的意图不得套用到新 UID 上。
    let stale = stale_intents(&[old_intent], 99);
    assert_eq!(stale.len(), 1);

    // 新 epoch 下重新全量建立快照。
    // 注意：这里**不能**拿旧缓存作差分基准——那会得到「服务器删了 1,2,3」
    // 这种完全错误的结论（旧 UID 在新世代里指的根本不是这些信）。
    let wrong = diff_uids(&local, &server);
    assert_eq!(wrong.removed, vec![1, 2, 3], "这正是必须避免的错误结论");

    let diff = diff_uids(&UidSet::new(), &server);
    assert_eq!(diff.added, vec![7, 8]);
    assert!(
        diff.removed.is_empty(),
        "epoch 变了之后旧 UID 全部作废，不能当作「服务器已删」"
    );
}

#[test]
fn scenario_old_uid_set_must_not_be_diffed_against_new_epoch() {
    // 这是个**约束**用例：跨 epoch 的差分没有意义。
    // 本地 UID {1,2,3} 属于 epoch 42，服务器 UID {1,2,3} 属于 epoch 99。
    // 数字相同，但指向的是完全不同的信。
    let stored = StoredEpoch::at(42);
    let transition = plan_epoch_transition(&stored, 99, &[]).unwrap();

    assert!(
        transition.requires_reset(),
        "必须先处理 epoch 变化，再谈差分"
    );

    // 正确做法：epoch 变化后清空旧 UID 映射，重新全量差分。
    let diff = diff_uids(&UidSet::new(), &UidSet::from_uids([1, 2, 3]));
    assert_eq!(diff.added, vec![1, 2, 3]);
    assert!(diff.removed.is_empty());
}

// ===========================================================================
// 场景五：flag 更新
// ===========================================================================

#[test]
fn scenario_local_view_merges_snapshot_with_pending_intent() {
    // 服务器快照：这封信未读、未标星。
    let snapshot = MessageFlags::empty();

    // 用户点了"标星"（seq 1），规则引擎又把它标为已读（seq 2）。
    let intents = [
        FlagIntent::new(1, 42, flagged(), MessageFlags::empty(), 1, None).unwrap(),
        FlagIntent::new(1, 42, seen(), MessageFlags::empty(), 2, None).unwrap(),
    ];

    // 本地视图：两个意图都体现出来。
    let local_view = apply_intents(snapshot, &intents);
    assert!(local_view.is_starred());
    assert!(local_view.is_read());

    // 但要发给服务器的只有净变化，且是 bit 级的。
    let plan = plan_flag_update(1, snapshot, 42, Some(7), &intents);
    let delta = plan.delta.expect("应有变化");

    assert!(delta.add.is_starred());
    assert!(delta.add.is_read());
    assert!(delta.remove.is_empty(), "未变化的位不应被清除");
    assert_eq!(
        delta.unchangedsince,
        Some(7),
        "有 CONDSTORE 时带上基线 MODSEQ"
    );
}

#[test]
fn scenario_toggling_read_on_then_off_sends_nothing() {
    let snapshot = MessageFlags::empty();
    let intents = [
        FlagIntent::new(1, 42, seen(), MessageFlags::empty(), 1, None).unwrap(),
        FlagIntent::new(1, 42, MessageFlags::empty(), seen(), 2, None).unwrap(),
    ];

    let plan = plan_flag_update(1, snapshot, 42, None, &intents);

    assert!(
        plan.delta.is_none(),
        "用户点了又取消，净变化为零，不该发命令"
    );
}

#[test]
fn scenario_rejected_conditional_write_is_replanned_against_fresh_snapshot() {
    let intents = [FlagIntent::new(1, 42, seen(), MessageFlags::empty(), 1, None).unwrap()];

    // 首次：快照未读 → 需要发"标记已读"。
    let first = plan_flag_update(1, MessageFlags::empty(), 42, Some(1), &intents);
    assert!(first.delta.is_some());

    // 服务器回了 MODIFIED（条件不满足），客户端重新读取快照：
    // 情形 A：服务器其实已经是 Seen → 意图已被吸收，不用再发。
    let after_agree = plan_flag_update(1, seen(), 42, Some(2), &intents);
    assert!(after_agree.delta.is_none(), "服务器已是目标状态时不应重发");

    // 情形 B：服务器没改 → 仍需发送。
    let after_disagree = plan_flag_update(1, MessageFlags::empty(), 42, Some(2), &intents);
    assert!(after_disagree.delta.is_some(), "服务器没改时仍需发送");
}

#[test]
fn scenario_stale_intent_never_reaches_the_server() {
    // 意图属于旧 epoch 42，而当前已是 99。
    let intents = [FlagIntent::new(1, 42, seen(), MessageFlags::empty(), 1, None).unwrap()];

    let plan = plan_flag_update(1, MessageFlags::empty(), 99, Some(5), &intents);

    assert_eq!(plan.stale.len(), 1, "旧意图必须被识别出来");
    assert!(
        plan.delta.is_none(),
        "旧 epoch 的意图绝不能被发到服务器——旧 UID 指向另一封信"
    );
}

#[test]
fn scenario_only_the_changed_bit_is_sent_not_the_whole_flag_set() {
    // 快照：已读 + 草稿。用户只想加"标星"。
    let snapshot = seen().union(MessageFlags::empty().with(MessageFlags::DRAFT));
    let intents = [FlagIntent::new(1, 42, flagged(), MessageFlags::empty(), 1, None).unwrap()];

    let plan = plan_flag_update(1, snapshot, 42, None, &intents);
    let delta = plan.delta.expect("应有变化");

    // 整组替换 FLAGS 会把 Seen、Draft 一起写回去；这里只写 Flagged。
    assert_eq!(delta.add.bits(), MessageFlags::FLAGGED);
    assert!(delta.remove.is_empty());
}
