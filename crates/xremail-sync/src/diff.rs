//! UID 差分。
//!
//! 同步的核心动作之一：拿「服务器现有的 UID 集合」和「本地已缓存的 UID 集合」
//! 作差，得到**要拉取的**与**服务器已删的**。
//!
//! ## 前提：差分只在同一个 epoch 内有意义
//!
//! UID 只在 `(folder, UIDVALIDITY)` 内有效。服务器换 epoch 后，
//! 旧的 UID 全部作废，拿旧集合跟新集合作差会得到**完全错误**的结果
//! （见 [ARCHITECTURE §4.1 与 §4.3](../../../docs/ARCHITECTURE.md)）。
//! epoch 变化的处置在 [`crate::epoch`]。
//!
//! ## 前提：本地草稿有独立归属
//!
//! 服务器没有的本地邮件**不等于**可以删：本地草稿与未发送内容有自己的归属，
//! 不能因为服务器上不存在就被顺手清掉
//! （见 [ARCHITECTURE §4.2 第 6 步](../../../docs/ARCHITECTURE.md)）。
//! 这一点由 [`UidDiff::retain_local`] 落实。

use std::collections::BTreeSet;

/// 一个文件夹中已知的 UID 集合。
///
/// 内部用 `BTreeSet`：UID 天然有序，且差分结果需要稳定的升序输出。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UidSet {
    uids: BTreeSet<i64>,
}

impl UidSet {
    /// 空集合。
    pub fn new() -> Self {
        Self::default()
    }

    /// 由 UID 列表构造。重复的 UID 会被去重。
    pub fn from_uids(uids: impl IntoIterator<Item = i64>) -> Self {
        Self {
            uids: uids.into_iter().collect(),
        }
    }

    /// 插入一个 UID。返回是否为新加入的。
    ///
    /// UID 必须是正数：IMAP 中 UID 0 非法（见 RFC 3501 §2.3.1.1）。
    /// 正常路径上由协议层保证；这里用 `debug_assert` 在开发期挡住。
    pub fn insert(&mut self, uid: i64) -> bool {
        debug_assert!(uid > 0, "IMAP 的 UID 必须是正数，UID 0 非法");
        self.uids.insert(uid)
    }

    /// 是否包含该 UID。
    pub fn contains(&self, uid: i64) -> bool {
        self.uids.contains(&uid)
    }

    /// 元素个数。
    pub fn len(&self) -> usize {
        self.uids.len()
    }

    /// 是否为空。
    pub fn is_empty(&self) -> bool {
        self.uids.is_empty()
    }

    /// 按升序迭代。
    pub fn iter(&self) -> impl Iterator<Item = i64> + '_ {
        self.uids.iter().copied()
    }

    /// 最小 UID。
    pub fn min(&self) -> Option<i64> {
        self.uids.iter().next().copied()
    }

    /// 最大 UID。
    pub fn max(&self) -> Option<i64> {
        self.uids.iter().next_back().copied()
    }

    /// 交集。
    pub fn intersection(&self, other: &UidSet) -> UidSet {
        UidSet {
            uids: self.uids.intersection(&other.uids).copied().collect(),
        }
    }

    /// 差集：`self` 去掉 `other` 的元素。
    pub fn difference(&self, other: &UidSet) -> UidSet {
        UidSet {
            uids: self.uids.difference(&other.uids).copied().collect(),
        }
    }

    /// 并集。
    pub fn union(&self, other: &UidSet) -> UidSet {
        UidSet {
            uids: self.uids.union(&other.uids).copied().collect(),
        }
    }
}

impl FromIterator<i64> for UidSet {
    fn from_iter<T: IntoIterator<Item = i64>>(iter: T) -> Self {
        Self::from_uids(iter)
    }
}

impl<'a> IntoIterator for &'a UidSet {
    type Item = i64;
    type IntoIter = std::iter::Copied<std::collections::btree_set::Iter<'a, i64>>;

    fn into_iter(self) -> Self::IntoIter {
        self.uids.iter().copied()
    }
}

/// UID 差分结果。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UidDiff {
    /// 服务器有、本地没有：需要拉取。
    pub added: Vec<i64>,
    /// 本地有、服务器没有：服务器已删除。
    ///
    /// **这些不能直接删缓存**——先过 [`UidDiff::retain_local`] 的保留策略。
    pub removed: Vec<i64>,
    /// 两边都有：保留。
    pub retained: Vec<i64>,
}

impl UidDiff {
    /// 应用保留策略：把受保护的本地 UID 从 `removed` 中剔除。
    ///
    /// 受保护的是**有独立归属**的本地内容，例如本地草稿与未发送内容。
    /// 它们即便在服务器上不存在，也不能被当成「服务器已删」清掉。
    pub fn retain_local(&mut self, protected: &BTreeSet<i64>) {
        self.removed.retain(|uid| !protected.contains(uid));
    }

    /// 把待拉取的 UID 分批。
    ///
    /// 同步时摘要分批 upsert，避免一次拉太多
    /// （见 [ARCHITECTURE §4.2 第 4 步](../../../docs/ARCHITECTURE.md)）。
    ///
    /// `batch_size` 为 0 时返回空列表（不做除零，也不 panic）。
    pub fn chunk_added(&self, batch_size: usize) -> Vec<Vec<i64>> {
        if batch_size == 0 {
            return Vec::new();
        }
        self.added.chunks(batch_size).map(<[i64]>::to_vec).collect()
    }

    /// 差分是否为空（既无新增也无删除）。
    pub fn is_empty(&self) -> bool {
        self.added.is_empty() && self.removed.is_empty()
    }
}

/// 计算 UID 差分。
///
/// **调用前提**：两个集合必须属于同一个 epoch。
/// 跨 epoch 比较会得到错误结果——epoch 变化时先走 [`crate::epoch::plan_epoch_transition`]。
pub fn diff_uids(local: &UidSet, remote: &UidSet) -> UidDiff {
    UidDiff {
        added: remote.difference(local).into_iter().collect(),
        removed: local.difference(remote).into_iter().collect(),
        retained: local.intersection(remote).into_iter().collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(uids: &[i64]) -> UidSet {
        UidSet::from_uids(uids.iter().copied())
    }

    #[test]
    fn empty_local_and_empty_remote_diff_is_empty() {
        let diff = diff_uids(&UidSet::new(), &UidSet::new());

        assert!(diff.is_empty());
        assert!(diff.added.is_empty());
        assert!(diff.removed.is_empty());
        assert!(diff.retained.is_empty());
    }

    #[test]
    fn remote_only_uids_are_added() {
        let diff = diff_uids(&UidSet::new(), &set(&[1, 2, 3]));

        assert_eq!(diff.added, vec![1, 2, 3]);
        assert!(diff.removed.is_empty());
        assert!(diff.retained.is_empty());
    }

    #[test]
    fn local_only_uids_are_removed() {
        let diff = diff_uids(&set(&[1, 2, 3]), &UidSet::new());

        assert!(diff.added.is_empty());
        assert_eq!(diff.removed, vec![1, 2, 3]);
    }

    #[test]
    fn common_uids_are_retained() {
        let diff = diff_uids(&set(&[1, 2, 3]), &set(&[2, 3, 4]));

        assert_eq!(diff.added, vec![4]);
        assert_eq!(diff.removed, vec![1]);
        assert_eq!(diff.retained, vec![2, 3]);
    }

    #[test]
    fn identical_sets_produce_no_changes() {
        let diff = diff_uids(&set(&[1, 2, 3]), &set(&[1, 2, 3]));

        assert!(diff.is_empty());
        assert_eq!(diff.retained, vec![1, 2, 3]);
    }

    #[test]
    fn output_is_sorted_ascending() {
        let diff = diff_uids(&set(&[9, 1, 5]), &set(&[7, 3]));

        assert_eq!(diff.added, vec![3, 7]);
        assert_eq!(diff.removed, vec![1, 5, 9]);
    }

    #[test]
    fn duplicate_uids_are_deduplicated() {
        let diff = diff_uids(&set(&[1, 1, 2]), &set(&[2, 2]));

        assert_eq!(diff.retained, vec![2]);
        assert_eq!(diff.removed, vec![1]);
    }

    #[test]
    fn protected_local_uids_are_not_removed() {
        let mut diff = diff_uids(&set(&[1, 2, 3]), &set(&[3]));
        // 本地草稿有独立归属，不能因为服务器上没有就清掉。
        let protected: BTreeSet<i64> = [1, 2].into_iter().collect();

        diff.retain_local(&protected);

        assert_eq!(diff.removed, Vec::<i64>::new());
        assert_eq!(diff.retained, vec![3]);
    }

    #[test]
    fn protection_only_removes_the_protected_ones() {
        let mut diff = diff_uids(&set(&[1, 2, 3]), &set(&[3]));
        let protected: BTreeSet<i64> = [2].into_iter().collect();

        diff.retain_local(&protected);

        assert_eq!(diff.removed, vec![1]);
    }

    #[test]
    fn chunking_splits_added_uids() {
        let diff = diff_uids(&UidSet::new(), &set(&[1, 2, 3, 4, 5]));

        let batches = diff.chunk_added(2);

        assert_eq!(batches, vec![vec![1, 2], vec![3, 4], vec![5]]);
    }

    #[test]
    fn chunking_with_zero_batch_size_returns_empty() {
        let diff = diff_uids(&UidSet::new(), &set(&[1, 2, 3]));

        assert!(diff.chunk_added(0).is_empty());
    }

    #[test]
    fn chunking_with_larger_batch_size_gives_one_batch() {
        let diff = diff_uids(&UidSet::new(), &set(&[1, 2, 3]));

        assert_eq!(diff.chunk_added(100), vec![vec![1, 2, 3]]);
    }

    #[test]
    fn set_operations() {
        let a = set(&[1, 2, 3]);
        let b = set(&[2, 3, 4]);

        assert_eq!(
            a.intersection(&b).into_iter().collect::<Vec<_>>(),
            vec![2, 3]
        );
        assert_eq!(a.difference(&b).into_iter().collect::<Vec<_>>(), vec![1]);
        assert_eq!(
            a.union(&b).into_iter().collect::<Vec<_>>(),
            vec![1, 2, 3, 4]
        );
        assert_eq!(a.min(), Some(1));
        assert_eq!(a.max(), Some(3));
        assert_eq!(a.len(), 3);
        assert!(!a.is_empty());
    }

    #[test]
    fn empty_set_has_no_bounds() {
        let a = UidSet::new();

        assert_eq!(a.min(), None);
        assert_eq!(a.max(), None);
        assert!(a.is_empty());
    }
}
