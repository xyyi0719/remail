//! 收件人地址校验与规范化。
//!
//! ## 职责边界
//!
//! 本模块**只校验单个地址**。把 `"张三 <a@b.com>, c@d.com; e@f.com"` 拆成
//! 若干个地址是**调用方**（前端输入框 / 后端命令）的事——分隔规则在不同入口
//! （To / Cc / Bcc 输入框、通讯录粘贴）不一样，收到本模块里反而会变成一套
//! 隐式约定。本模块因此假定传进来的每一项都是**一个完整地址**。
//!
//! ## 为什么只做「明显错误」级校验
//!
//! 这里的取舍是刻意的：**宁可放过，不可错杀**。
//!
//! * 不查 DNS：域名解析是收件服务器的事，而且校验时多半还没有网络；
//!   提前把 `example.internal` 这类内网/自建域名判死是误伤。
//! * 不查 TLD 列表：TLD 列表会过期，`.zip`、`.mov` 这类新顶级域进表有延迟，
//!   而 `.museum` 这种冷门后缀很容易被漏掉。
//! * 不套本地部分的完整 RFC 5322 语法（点号规则、特殊字符表……）：
//!   真实邮箱服务商的实现远比 RFC 宽松，照 RFC 严查会把合法地址判死。
//!
//! 因此这里只挡**一眼就能看出是错的**那几类：空串、缺 `@`、多个 `@`、
//! 空本地部分、空域名、夹空白、域名的点号形态不对。
//!
//! ## 规范化只做三件事
//!
//! 1. `trim`：去掉两端空白（用户从别处粘贴常带空格）。
//! 2. 域名转小写：域名大小写不敏感。
//! 3. **本地部分保持原样大小写**：RFC 5321 §2.4 明确本地部分大小写敏感，
//!    把 `User@` 改成 `user@` 可能投递到另一个账号（部分服务商这么实现）。
//!
//! 不做 IDNA / punycode 转换：那需要引入新依赖，而本 crate 明确要求零新增依赖。

use std::fmt;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// RFC 5321 §4.5.3.1.1：本地部分最长 64 个字符。
const MAX_LOCAL_PART_LEN: usize = 64;

/// RFC 5321 §4.5.3.1.2：域名最长 255 个字符。
const MAX_DOMAIN_LEN: usize = 255;

/// 一个通过了校验的地址。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NormalizedAddress {
    /// **原始输入**（未做 trim、未改大小写），用来回指 UI 里用户输入的那一栏。
    pub original: String,
    /// `@` 之前的本地部分。**保持原始大小写**（RFC 上大小写敏感）。
    pub local_part: String,
    /// `@` 之后的域名。已转小写。
    ///
    /// IP 字面量保留方括号（`[192.168.0.1]`），这样
    /// `local_part + "@" + domain` 永远等于规范化后的地址。
    pub domain: String,
    /// 本地部分或域名含**非 ASCII** 字符。
    ///
    /// 调用方据此决定要不要启用 SMTPUTF8（RFC 6531）：
    /// 为 `false` 时可以用纯 7-bit ASCII 通道发，为 `true` 时必须走 SMTPUTF8，
    /// 否则非 ASCII 会被按未定义行为处理（多半是乱码或被拒）。
    pub internationalized: bool,
    /// **只提示、不拦截**的问题（例如本地部分超长）。
    ///
    /// 长度上限是硬性协议规定，但现实里部分服务商的实现更宽松，
    /// 所以这里不因为超长就拒绝发送，只把提示交给调用方显示。
    pub warnings: Vec<AddressReason>,
}

impl NormalizedAddress {
    /// 规范化后的完整地址（`本地部分@域名`）。
    pub fn normalized(&self) -> String {
        format!("{}@{}", self.local_part, self.domain)
    }
}

impl fmt::Display for NormalizedAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.normalized())
    }
}

/// 地址出问题时的具体原因。
///
/// 变体名用英文（与 [`crate::model`] 的风格一致，便于 serde 线格式），
/// 中文说明见 [`AddressReason`] 的 `Display` 实现。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Error)]
#[serde(rename_all = "camelCase")]
pub enum AddressReason {
    /// 地址为空（或只有空白字符）。
    #[error("地址为空")]
    Empty,

    /// 缺少 `@`，没有分成本地部分和域名。
    #[error("缺少 '@'，地址格式应为 本地部分@域名")]
    MissingAt,

    /// 出现了多个 `@`。
    #[error("地址里有多个 '@'")]
    MultipleAt,

    /// 本地部分用引号包裹且里面含 `@`——这种写法暂不支持，直接判错。
    #[error("本地部分用引号包裹且含 '@'，暂不支持这种写法")]
    QuotedAtUnsupported,

    /// 本地部分为空（`@` 前面什么都没有）。
    #[error("缺少本地部分（'@' 前面是空的）")]
    EmptyLocalPart,

    /// 域名为空（`@` 后面什么都没有）。
    #[error("缺少域名（'@' 后面是空的）")]
    EmptyDomain,

    /// 本地部分或域名里含空白字符。
    #[error("本地部分或域名里含有空白字符")]
    Whitespace,

    /// 域名里有连续的点（`a..b.com`）。
    #[error("域名里有连续的点")]
    ConsecutiveDots,

    /// 域名以点开头或结尾（`.a.com` / `a.com.`）。
    #[error("域名以点开头或结尾")]
    DomainEdgeDot,

    /// 域名里没有点，而且也不是 IP 字面量（`user@localhost`）。
    #[error("域名里没有点，且不是 IP 字面量")]
    DomainNotDotted,

    /// 域名里有非法字符。
    #[error("域名里含有非法字符")]
    InvalidDomainChar,

    /// IP 字面量解析失败（`user@[192.168.0.999]`）。
    #[error("IP 字面量格式不合法")]
    InvalidIpLiteral,

    /// 本地部分超过 64 个字符——**只提示**。
    #[error("本地部分过长（超过 64 个字符）")]
    LocalPartTooLong,

    /// 域名超过 255 个字符——**只提示**。
    #[error("域名过长（超过 255 个字符）")]
    DomainTooLong,
}

/// 一个有问题的地址。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AddressProblem {
    /// 出问题的原始输入（未做任何处理）。
    pub original: String,
    /// 命中的原因，可能有多个（例如既含空白、域名又以点结尾）。
    pub reasons: Vec<AddressReason>,
}

impl AddressProblem {
    /// 中文说明，多个原因用 `；` 连接。
    pub fn message(&self) -> String {
        self.reasons
            .iter()
            .map(|r| r.to_string())
            .collect::<Vec<_>>()
            .join("；")
    }
}

impl fmt::Display for AddressProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}：{}", self.original, self.message())
    }
}

/// 一次校验的完整结果。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct AddressValidation {
    /// 通过校验的地址（保持输入顺序）。
    pub valid: Vec<NormalizedAddress>,
    /// 有问题的地址（保持输入顺序）。
    pub problems: Vec<AddressProblem>,
}

/// 校验并规范化一组地址。
///
/// **一次报出所有问题**：任何一个地址不合法都返回 `Err`，
/// 但 `Err` 里带着**全部**有问题的地址，而不是遇到第一个就停——
/// 否则用户得改一个、提交一次、再改一个，体验极差。
///
/// `Ok` 里是全部通过校验的地址（保持输入顺序）。
/// 输入为空列表时返回 `Ok(vec![])`：是否允许「没有收件人」是写信流程的规则，
/// 不是地址本身的合法性问题。
///
/// ```
/// use xremail_core::address::validate_addresses;
///
/// let ok = validate_addresses(&["Alice@Example.COM"]).unwrap();
/// assert_eq!(ok[0].local_part, "Alice");
/// assert_eq!(ok[0].domain, "example.com");
///
/// let err = validate_addresses(&["a@b.com", "bad address"]).unwrap_err();
/// assert_eq!(err.len(), 1);
/// assert_eq!(err[0].original, "bad address");
/// ```
pub fn validate_addresses<S: AsRef<str>>(
    addresses: &[S],
) -> Result<Vec<NormalizedAddress>, Vec<AddressProblem>> {
    let result = validate_addresses_detailed(addresses);
    if result.problems.is_empty() {
        Ok(result.valid)
    } else {
        Err(result.problems)
    }
}

/// 与 [`validate_addresses`] 同样的校验，但**同时**给出通过的和没通过的。
///
/// 写信面板需要这个：坏地址要标红提示，好地址还要留在收件人列表里。
/// [`validate_addresses`] 是「全有或全无」的形态，没法同时表达这两件事。
pub fn validate_addresses_detailed<S: AsRef<str>>(addresses: &[S]) -> AddressValidation {
    let mut valid = Vec::new();
    let mut problems = Vec::new();

    for raw in addresses {
        match validate_one(raw.as_ref()) {
            Ok(addr) => valid.push(addr),
            Err(reasons) => problems.push(AddressProblem {
                original: raw.as_ref().to_string(),
                reasons,
            }),
        }
    }

    AddressValidation { valid, problems }
}

/// 校验单个地址。
fn validate_one(raw: &str) -> Result<NormalizedAddress, Vec<AddressReason>> {
    let trimmed = raw.trim();

    if trimmed.is_empty() {
        return Err(vec![AddressReason::Empty]);
    }

    // `@` 的个数：0 个说明没写地址，1 个正常，2 个以上说明地址写串了。
    let at_count = trimmed.matches('@').count();
    if at_count == 0 {
        return Err(vec![AddressReason::MissingAt]);
    }
    if at_count > 1 {
        // 引号本地部分里带 `@`（`"a@b"@x.com`）这种写法在 RFC 上是合法的，
        // 但拆解规则明显更复杂，先明确判成错误——宁可报错也不要猜。
        let reason = if trimmed.starts_with('"') {
            AddressReason::QuotedAtUnsupported
        } else {
            AddressReason::MultipleAt
        };
        return Err(vec![reason]);
    }

    let (local, domain) = trimmed
        .split_once('@')
        .expect("at_count == 1 时必然能 split_once");

    let mut reasons = Vec::new();

    if local.is_empty() {
        reasons.push(AddressReason::EmptyLocalPart);
    }
    if domain.is_empty() {
        reasons.push(AddressReason::EmptyDomain);
    }

    // 空白：引号包裹的本地部分里允许有空格（RFC 5322 的 quoted-string），
    // 其余位置一律不允许。
    let quoted_local = local.len() >= 2 && local.starts_with('"') && local.ends_with('"');
    if !quoted_local && local.chars().any(char::is_whitespace) {
        reasons.push(AddressReason::Whitespace);
    }
    if domain.chars().any(char::is_whitespace) {
        reasons.push(AddressReason::Whitespace);
    }

    if !domain.is_empty() {
        if let Some(inner) = domain
            .strip_prefix('[')
            .and_then(|rest| rest.strip_suffix(']'))
        {
            // IP 字面量：`[192.168.0.1]` / `[::1]`。
            // 它允许没有点，所以走单独一支，不套用域名的点号规则。
            if inner.parse::<std::net::Ipv4Addr>().is_err()
                && inner.parse::<std::net::Ipv6Addr>().is_err()
            {
                reasons.push(AddressReason::InvalidIpLiteral);
            }
        } else {
            if domain.starts_with('.') || domain.ends_with('.') {
                reasons.push(AddressReason::DomainEdgeDot);
            }
            if domain.contains("..") {
                reasons.push(AddressReason::ConsecutiveDots);
            }
            // 没有点又没有方括号的域名（`user@localhost`）不是合法邮件域名。
            if !domain.contains('.') {
                reasons.push(AddressReason::DomainNotDotted);
            }
            // 域名允许：字母、数字、`-`、`_`、`.`，以及非 ASCII 字符
            // （国际化域名不做 punycode 转换，见模块文档）。
            if !domain
                .chars()
                .all(|c| c.is_alphanumeric() || c == '-' || c == '_' || c == '.')
            {
                reasons.push(AddressReason::InvalidDomainChar);
            }
        }
    }

    if !reasons.is_empty() {
        return Err(reasons);
    }

    // 长度只提示、不拦截：协议有上限，但现实里服务商实现更宽松，
    // 直接拒收会误伤合法地址。
    let mut warnings = Vec::new();
    if local.chars().count() > MAX_LOCAL_PART_LEN {
        warnings.push(AddressReason::LocalPartTooLong);
    }
    if domain.chars().count() > MAX_DOMAIN_LEN {
        warnings.push(AddressReason::DomainTooLong);
    }

    Ok(NormalizedAddress {
        original: raw.to_string(),
        local_part: local.to_string(),
        // 域名大小写不敏感，转小写；本地部分保持原样。
        domain: domain.to_lowercase(),
        internationalized: local.chars().chain(domain.chars()).any(|c| !c.is_ascii()),
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 便捷断言：单个地址应当通过校验。
    fn ok(addr: &str) -> NormalizedAddress {
        validate_addresses(&[addr])
            .unwrap_or_else(|e| panic!("「{addr}」应当合法，却报了：{e:?}"))
            .remove(0)
    }

    /// 便捷断言：单个地址应当被拒绝。
    fn bad(addr: &str) -> Vec<AddressReason> {
        validate_addresses(&[addr])
            .expect_err("该地址应当被拒绝")
            .remove(0)
            .reasons
    }

    #[test]
    fn accepts_ordinary_addresses() {
        let a = ok("alice@example.com");
        assert_eq!(a.local_part, "alice");
        assert_eq!(a.domain, "example.com");
        assert!(!a.internationalized);
        assert!(a.warnings.is_empty());
    }

    #[test]
    fn accepts_tagged_local_part() {
        // `+` 标签（`alice+news@example.com`）是 RFC 5322 的 sub-address，
        // 常见于邮件服务商，必须放行。
        let a = ok("alice+news@example.com");
        assert_eq!(a.local_part, "alice+news");
        assert_eq!(a.domain, "example.com");
    }

    #[test]
    fn accepts_quoted_local_part_with_space() {
        // 引号包裹的本地部分里允许有空格。
        let a = ok("\"a b\"@x.com");
        assert_eq!(a.local_part, "\"a b\"");
        assert_eq!(a.domain, "x.com");
    }

    #[test]
    fn accepts_ip_literal() {
        let a = ok("user@[192.168.0.1]");
        assert_eq!(a.local_part, "user");
        assert_eq!(a.domain, "[192.168.0.1]");
        assert_eq!(a.normalized(), "user@[192.168.0.1]");
    }

    #[test]
    fn accepts_ipv6_literal() {
        let a = ok("user@[::1]");
        assert_eq!(a.domain, "[::1]");
    }

    #[test]
    fn trims_surrounding_whitespace() {
        let a = ok("  alice@example.com \t");
        assert_eq!(a.local_part, "alice");
        assert_eq!(a.domain, "example.com");
        // original 保留用户原样输入，便于回指 UI。
        assert_eq!(a.original, "  alice@example.com \t");
    }

    #[test]
    fn lowercases_domain_but_not_local_part() {
        let a = ok("Alice@EXAMPLE.COM");
        // 域名不敏感 → 小写。
        assert_eq!(a.domain, "example.com");
        // 本地部分敏感 → 原样保留。
        assert_eq!(a.local_part, "Alice");
        assert_eq!(a.normalized(), "Alice@example.com");
    }

    #[test]
    fn detects_internationalized_addresses() {
        let a = ok("张三@例子.中国");
        assert!(a.internationalized);
        assert_eq!(a.local_part, "张三");

        let b = ok("user@例子.com");
        assert!(b.internationalized);

        let c = ok("user@example.com");
        assert!(!c.internationalized);
    }

    #[test]
    fn rejects_empty_address() {
        assert_eq!(bad(""), vec![AddressReason::Empty]);
        assert_eq!(bad("   "), vec![AddressReason::Empty]);
    }

    #[test]
    fn rejects_missing_at() {
        assert_eq!(bad("alice.example.com"), vec![AddressReason::MissingAt]);
    }

    #[test]
    fn rejects_multiple_at() {
        assert_eq!(bad("a@b@c.com"), vec![AddressReason::MultipleAt]);
    }

    #[test]
    fn rejects_quoted_local_part_containing_at() {
        // 引号里带 @ 的写法暂不支持，明确判错而不是猜。
        assert_eq!(
            bad("\"a@b\"@x.com"),
            vec![AddressReason::QuotedAtUnsupported]
        );
    }

    #[test]
    fn rejects_empty_local_part_and_domain() {
        assert_eq!(bad("@example.com"), vec![AddressReason::EmptyLocalPart]);
        assert_eq!(bad("alice@"), vec![AddressReason::EmptyDomain]);
        // 两边都空时两个原因都报出来。
        assert_eq!(
            bad("@"),
            vec![AddressReason::EmptyLocalPart, AddressReason::EmptyDomain]
        );
    }

    #[test]
    fn rejects_whitespace_inside_address() {
        assert!(bad("ali ce@example.com").contains(&AddressReason::Whitespace));
        assert!(bad("alice@exa mple.com").contains(&AddressReason::Whitespace));
        assert!(bad("alice @example.com").contains(&AddressReason::Whitespace));
    }

    #[test]
    fn rejects_malformed_domain_dots() {
        assert!(bad("alice@example..com").contains(&AddressReason::ConsecutiveDots));
        assert!(bad("alice@.example.com").contains(&AddressReason::DomainEdgeDot));
        assert!(bad("alice@example.com.").contains(&AddressReason::DomainEdgeDot));
    }

    #[test]
    fn rejects_domain_without_dot() {
        assert_eq!(bad("alice@localhost"), vec![AddressReason::DomainNotDotted]);
    }

    #[test]
    fn rejects_invalid_ip_literal() {
        assert_eq!(
            bad("user@[192.168.0.999]"),
            vec![AddressReason::InvalidIpLiteral]
        );
    }

    #[test]
    fn rejects_invalid_domain_characters() {
        assert!(bad("alice@exa<mple.com").contains(&AddressReason::InvalidDomainChar));
        assert!(bad("alice@exa(mple.com").contains(&AddressReason::InvalidDomainChar));
    }

    #[test]
    fn reports_every_problem_at_once() {
        let err = validate_addresses(&[
            "good@example.com",
            "bad address",
            "no-at-sign",
            "@example.com",
            "also-bad@@example.com",
        ])
        .unwrap_err();

        // 4 个坏地址一次全报出来，而不是遇到第一个就停。
        assert_eq!(err.len(), 4);
        assert_eq!(err[0].original, "bad address");
        assert_eq!(err[1].original, "no-at-sign");
        assert_eq!(err[2].original, "@example.com");
        assert_eq!(err[3].original, "also-bad@@example.com");
    }

    #[test]
    fn detailed_result_keeps_both_sides() {
        let r = validate_addresses_detailed(&["good@example.com", "bad address@example.com"]);
        assert_eq!(r.valid.len(), 1);
        assert_eq!(r.valid[0].normalized(), "good@example.com");
        assert_eq!(r.problems.len(), 1);
        assert!(r.problems[0].message().contains("空白"));
    }

    #[test]
    fn overlong_parts_are_warnings_not_errors() {
        let long_local = format!("{}@example.com", "a".repeat(70));
        let a = ok(&long_local);
        assert_eq!(a.warnings, vec![AddressReason::LocalPartTooLong]);

        // 域名里要有点，否则会先被「域名没有点」判成错误。
        let long_domain = format!("alice@{}.com", "b".repeat(300));
        let b = ok(&long_domain);
        assert_eq!(b.warnings, vec![AddressReason::DomainTooLong]);
    }

    #[test]
    fn empty_input_is_not_an_error() {
        assert!(validate_addresses::<String>(&[]).unwrap().is_empty());
    }

    #[test]
    fn problem_display_is_chinese_and_specific() {
        let err = validate_addresses(&["bad address@example.com"]).unwrap_err();
        let text = err[0].to_string();
        assert!(text.contains("bad address@example.com"), "{text}");
        assert!(text.contains("空白"), "{text}");
    }
}
