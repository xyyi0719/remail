//! IMAP sequence set 解析。
//!
//! 语法（RFC 3501 §9）：
//!
//! ```text
//! sequence-set = (seq-number / seq-range) *("," sequence-set)
//! seq-number  = nz-number / "*"
//! seq-range   = seq-number ":" seq-number
//! ```
//!
//! `*` 是「当前最大值」：对 `UID` 命令来说是邮箱里**最大的 UID**。
//!
//! ## 为什么解析器在**服务端**这一侧
//!
//! 客户端（async-imap）把 sequence set 当**原始字符串**发出去
//! （`uid_fetch(uid_set: impl AsRef<str>, ...)`），并不自己解析。
//! 所以「把 `1:10,15,20:*` 变成一个 UID 集合」是**服务端**的活。
//!
//! 顺带一提：客户端和服务器各写一份解析器是**故意的**——
//! 如果两边共用一份有 bug 的实现，测试可能两边一起错、却看不出来。

use std::fmt;

/// sequence set 解析错误。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SequenceError {
    /// 空字符串。
    #[error("sequence set 为空")]
    Empty,
    /// 语法不对。
    #[error("sequence set 语法错误: {0}")]
    Invalid(String),
}

/// 序列号的一端。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bound {
    /// 明确的数字。
    Number(u32),
    /// `*`（当前最大值）。
    Star,
}

impl Bound {
    /// 解析一端。
    fn parse(token: &str) -> Result<Self, SequenceError> {
        if token == "*" {
            return Ok(Bound::Star);
        }
        if token.is_empty() {
            return Err(SequenceError::Invalid("空的序列号".into()));
        }
        if !token.bytes().all(|b| b.is_ascii_digit()) {
            return Err(SequenceError::Invalid(format!("序列号不是数字: {token}")));
        }
        token
            .parse::<u32>()
            .map(Bound::Number)
            .map_err(|_| SequenceError::Invalid(format!("序列号溢出: {token}")))
    }

    /// 用当前最大值把 `*` 换成具体数字。
    fn resolve(self, max: u32) -> u32 {
        match self {
            Bound::Number(n) => n,
            Bound::Star => max,
        }
    }
}

/// 一段范围。
///
/// * `1` → `Single`
/// * `1:10` → `Range`
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Segment {
    /// 单个序号。
    Single(Bound),
    /// 范围。`end` 为 `None` 表示 `a:*`。
    Range(Bound, Option<Bound>),
}

/// 一个 sequence set。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SequenceSet {
    segments: Vec<Segment>,
}

impl SequenceSet {
    /// 解析。
    ///
    /// ```
    /// use xremail_testkit::sequence::SequenceSet;
    /// let set = SequenceSet::parse("1:10,15,20:*").unwrap();
    /// assert_eq!(set.len(), 3);
    /// ```
    pub fn parse(input: &str) -> Result<Self, SequenceError> {
        let input = input.trim();
        if input.is_empty() {
            return Err(SequenceError::Empty);
        }

        let mut segments = Vec::new();
        for part in input.split(',') {
            let part = part.trim();
            if part.is_empty() {
                return Err(SequenceError::Invalid("空的区间".into()));
            }
            match part.split_once(':') {
                Some((start, end)) => {
                    let start = Bound::parse(start.trim())?;
                    let end = Bound::parse(end.trim())?;
                    // `a:b` 里 a > b 是合法输入，但结果为空（RFC 3501 §9）。
                    segments.push(Segment::Range(start, Some(end)));
                }
                None => segments.push(Segment::Single(Bound::parse(part)?)),
            }
        }

        Ok(Self { segments })
    }

    /// 区间个数。
    pub fn len(&self) -> usize {
        self.segments.len()
    }

    /// 是否为空（永远为 false——解析成功的 set 至少有一段）。
    pub fn is_empty(&self) -> bool {
        self.segments.is_empty()
    }

    /// 判断某个序号是否落在集合里。
    ///
    /// `max` 是当前最大值（`*` 的含义）。
    pub fn contains(&self, value: u32, max: u32) -> bool {
        self.segments.iter().any(|segment| match segment {
            Segment::Single(bound) => bound.resolve(max) == value,
            Segment::Range(start, end) => {
                let start = start.resolve(max);
                let end = end.map_or(max, |e| e.resolve(max));
                start <= value && value <= end
            }
        })
    }

    /// 展开成具体序号列表（升序、去重）。
    ///
    /// `max` 是当前最大值。超过 `max` 的部分会被截断——
    /// 服务器不会返回不存在的邮件。
    ///
    /// **注意**：这是**展开**，`1:*` 在大 UID 空间下可能产生上亿个元素。
    /// 处理真实邮箱时请用 [`SequenceSet::contains`] 逐个判断，不要展开。
    pub fn resolve(&self, max: u32) -> Vec<u32> {
        let mut out: Vec<u32> = Vec::new();
        for segment in &self.segments {
            match segment {
                Segment::Single(bound) => {
                    let value = bound.resolve(max);
                    if value <= max {
                        out.push(value);
                    }
                }
                Segment::Range(start, end) => {
                    let start = start.resolve(max);
                    let end = end.map_or(max, |e| e.resolve(max));
                    // 起点大于终点 → 空区间（RFC 3501 §9）。
                    if start > end {
                        continue;
                    }
                    for value in start..=end.min(max) {
                        out.push(value);
                    }
                }
            }
        }
        out.sort_unstable();
        out.dedup();
        out
    }
}

impl fmt::Display for SequenceSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let parts: Vec<String> = self
            .segments
            .iter()
            .map(|segment| match segment {
                Segment::Single(Bound::Number(n)) => n.to_string(),
                Segment::Single(Bound::Star) => "*".to_string(),
                Segment::Range(start, end) => {
                    let start = match start {
                        Bound::Number(n) => n.to_string(),
                        Bound::Star => "*".to_string(),
                    };
                    let end = match end {
                        Some(Bound::Number(n)) => n.to_string(),
                        Some(Bound::Star) => "*".to_string(),
                        None => "*".to_string(),
                    };
                    format!("{start}:{end}")
                }
            })
            .collect();
        write!(f, "{}", parts.join(","))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_single_number() {
        let set = SequenceSet::parse("42").unwrap();

        assert_eq!(set.len(), 1);
        assert!(set.contains(42, 100));
        assert!(!set.contains(43, 100));
    }

    #[test]
    fn parses_range() {
        let set = SequenceSet::parse("1:10").unwrap();

        assert!(set.contains(1, 100));
        assert!(set.contains(10, 100));
        assert!(!set.contains(11, 100));
        assert!(!set.contains(0, 100));
    }

    #[test]
    fn parses_mixed_list() {
        let set = SequenceSet::parse("1:10,15,20:*").unwrap();

        assert_eq!(set.len(), 3);
        assert!(set.contains(5, 100));
        assert!(set.contains(15, 100));
        assert!(set.contains(20, 100));
        assert!(set.contains(100, 100));
        assert!(!set.contains(16, 100));
    }

    #[test]
    fn star_means_current_maximum() {
        let set = SequenceSet::parse("*").unwrap();

        assert!(set.contains(7, 7));
        assert!(!set.contains(8, 7));
    }

    #[test]
    fn range_to_star_is_capped_at_maximum() {
        let set = SequenceSet::parse("5:*").unwrap();

        assert!(set.contains(5, 10));
        assert!(set.contains(10, 10));
        assert!(!set.contains(11, 10));
    }

    #[test]
    fn resolve_expands_and_clamps() {
        let set = SequenceSet::parse("1:3,100:*").unwrap();

        // 100 超过最大值 50 → 被截断。
        assert_eq!(set.resolve(50), vec![1, 2, 3]);
    }

    #[test]
    fn contains_does_not_expand_the_range() {
        // 大 UID 空间（163 那样）下，`1:*` 展开会吃掉几个 GB。
        // `contains` 逐个判断，没有这个问题。
        let set = SequenceSet::parse("1:*").unwrap();
        let max = 1_792_658_703;

        assert!(set.contains(1, max));
        assert!(set.contains(1_792_658_703, max));
        assert!(!set.contains(1_792_658_704, max));
    }

    #[test]
    fn reversed_range_is_empty() {
        let set = SequenceSet::parse("10:1").unwrap();

        assert!(set.resolve(100).is_empty());
    }

    #[test]
    fn duplicate_values_are_deduplicated() {
        let set = SequenceSet::parse("1,1,1:3").unwrap();

        assert_eq!(set.resolve(100), vec![1, 2, 3]);
    }

    #[test]
    fn empty_input_is_an_error() {
        assert_eq!(SequenceSet::parse(""), Err(SequenceError::Empty));
        assert_eq!(SequenceSet::parse("   "), Err(SequenceError::Empty));
    }

    #[test]
    fn malformed_input_is_an_error() {
        assert!(matches!(
            SequenceSet::parse("1:"),
            Err(SequenceError::Invalid(_))
        ));
        assert!(matches!(
            SequenceSet::parse("abc"),
            Err(SequenceError::Invalid(_))
        ));
        assert!(matches!(
            SequenceSet::parse("1,,2"),
            Err(SequenceError::Invalid(_))
        ));
    }

    #[test]
    fn display_round_trips() {
        for input in ["1", "1:10", "1:10,15,20:*", "*", "5:*"] {
            let set = SequenceSet::parse(input).unwrap();
            assert_eq!(set.to_string(), input);
        }
    }
}
