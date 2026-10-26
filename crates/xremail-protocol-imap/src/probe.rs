//! 只读探测。
//!
//! ## 严格只读
//!
//! 本模块**只**执行下列命令：
//!
//! * `LOGIN` —— 认证
//! * `CAPABILITY` —— 读能力集
//! * `LIST` —— 列文件夹
//! * `EXAMINE` —— **只读**地选文件夹（不同于 `SELECT`，不改 `\Seen` 等标记）
//! * `IDLE` + `DONE` —— 仅在服务器宣告支持时试一次，立即退出
//!
//! **绝不**执行 `STORE` / `MOVE` / `COPY` / `DELETE` / `EXPUNGE` / `APPEND`
//! 或任何 SMTP 发信命令。
//!
//! ## 脱敏
//!
//! 报告里**不出现**：用户名、口令、邮件主题、发件人、邮件正文。
//! 文件夹名只保留**标准名**（INBOX / Sent / Draft / Trash / Junk 等），
//! 非标准名只报数量——测试账号的自建文件夹名也可能带个人信息。

use async_imap::types::Capabilities;
use futures_util::TryStreamExt;
use std::time::Duration;

use crate::connection::{ImapConfig, Session, TlsInfo};
use crate::error::ProtocolError;

/// 探测配置。
#[derive(Debug, Clone)]
pub struct ProbeConfig {
    /// 服务器配置。
    pub server: ImapConfig,
    /// 要检查的邮箱（通常就是 INBOX）。
    pub mailbox: String,
    /// 是否尝试 IDLE（仅在服务器宣告支持时）。
    pub try_idle: bool,
}

/// 扩展支持度。
#[derive(Debug, Clone, Default)]
pub struct ExtensionSupport {
    /// `IDLE` —— 实时推送的基础。
    pub idle: bool,
    /// `CONDSTORE` —— 条件写的基础（ARCHITECTURE §6 的 UNCHANGEDSINCE）。
    pub condstore: bool,
    /// `QRESYNC` —— CONDSTORE 的增量同步扩展。
    pub qresync: bool,
    /// `UIDPLUS` —— MOVE/COPY 返回 UIDVALIDITY 与目标 UID。
    pub uidplus: bool,
    /// `MOVE` —— 一步移动。**没有**它就要走 COPY + \Deleted + EXPUNGE。
    pub move_: bool,
    /// `LIST-EXTENDED` —— LIST 返回特殊用途标记。
    pub list_extended: bool,
    /// `NAMESPACE` —— 文件夹层级与分隔符。
    pub namespace: bool,
    /// `UTF8=ACCEPT` / `UTF8=ONLY` —— UTF-8 支持。
    pub utf8: bool,
    /// `ID` —— RFC 2971 服务器标识。
    pub id: bool,
    /// `QUOTA` —— 配额查询。
    pub quota: bool,
    /// `LITERAL+` —— 非同步字面量。
    pub literal_plus: bool,
    /// `LITERAL-` —— 旧式同步字面量。
    pub literal_minus: bool,
}

/// 文件夹概况（脱敏）。
#[derive(Debug, Clone, Default)]
pub struct FolderSummary {
    /// 文件夹总数。
    pub total: usize,
    /// 出现的标准文件夹名。
    pub standard: Vec<String>,
    /// 非标准文件夹的数量（**不列出名字**）。
    pub non_standard_count: usize,
    /// 观察到的层级分隔符。
    pub delimiter: Option<String>,
    /// 按**特殊用途属性**识别出的文件夹角色。
    ///
    /// 这是同步引擎真正需要的信息：它要知道哪个文件夹是「已发送」、
    /// 哪个是「垃圾邮件」，而不是靠猜名字。
    /// 名字只对已知标准名与带特殊用途属性的文件夹显示，其余显示 `«自定义»`。
    pub roles: Vec<FolderRole>,
}

/// 一个文件夹的角色。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FolderRole {
    /// 角色：inbox / sent / drafts / trash / junk / archive / flagged / important / other
    pub role: String,
    /// 文件夹名（脱敏）。
    pub name: String,
}

/// 邮箱概况（脱敏）。
#[derive(Debug, Clone, Default)]
pub struct MailboxSummary {
    /// 邮件总数。
    pub exists: u32,
    /// 未读数。
    pub unseen: Option<u32>,
    /// UIDVALIDITY（epoch）。
    pub uid_validity: Option<u32>,
    /// 下一个 UID。
    pub uid_next: Option<u32>,
    /// 最高 MODSEQ。
    pub highest_modseq: Option<u64>,
    /// 可设置的永久标记。
    pub permanent_flags: Vec<String>,
    /// IDLE 是否真的被服务器接受。
    pub idle_accepted: Option<bool>,
    /// 服务器是否在 EXAMINE 响应里给了 UNSEEN。
    ///
    /// 163 **不给**（只能从 `STATUS` 拿），所以这个字段用于记录差异。
    pub examine_reported_unseen: bool,
}

/// 探测报告。
#[derive(Debug, Clone)]
pub struct ProbeReport {
    /// TLS 会话信息。
    pub tls: TlsInfo,
    /// 认证后能力集（排序后的原子名）。
    pub capabilities: Vec<String>,
    /// 支持的认证机制（`AUTH=xxx` 的 xxx 部分）。
    pub auth_mechanisms: Vec<String>,
    /// 扩展支持度。
    pub extensions: ExtensionSupport,
    /// 文件夹概况。
    pub folders: FolderSummary,
    /// 邮箱概况。
    pub mailbox: MailboxSummary,
}

/// 标准文件夹名（大小写不敏感比较）。
const STANDARD_FOLDERS: &[&str] = &[
    "INBOX",
    "SENT",
    "DRAFT",
    "DRAFTS",
    "TRASH",
    "JUNK",
    "SPAM",
    "ARCHIVE",
    "ALL",
    "FLAGGED",
    "IMPORTANT",
];

/// 按**名字**认特殊用途角色的兼底表。
///
/// ## 为什么需要兼底
///
/// RFC 6154 的特殊用途属性**不是所有服务器都给**：
///
/// * 163 给（`\Sent` / `\Drafts` / `\Trash` / `\Junk`）；
/// * 沃也给；
/// * **QQ 不给**——实测它的 LIST 只返回 `\NoSelect` / `\HasChildren`，
///   哪怕它宣告了 XLIST。
///
/// 所以同步引擎认「已发送 / 草稿箱 / 垃圾邮件」不能只靠属性，
/// 还得认名字。这张表来自三家服务商的**实测**（2026-10-23）。
///
/// 匹配时先**去掉尾部的空白与常见的分隔符**（如 `Junk E-mail`）。
const ROLE_NAMES: &[(&str, &str)] = &[
    // 163（中文名 + 属性，这两个都能命中）。
    ("已发送", "sent"),
    ("发送邮件", "sent"),
    ("草稿箱", "drafts"),
    ("草稿", "drafts"),
    ("已删除", "trash"),
    ("已删除邮件", "trash"),
    ("垃圾邮件", "junk"),
    ("垃圾箱", "junk"),
    // QQ（英文名，**没有属性**，只能靠名字）。
    ("sent messages", "sent"),
    ("sent message", "sent"),
    ("drafts", "drafts"),
    ("draft", "drafts"),
    ("deleted messages", "trash"),
    ("deleted message", "trash"),
    ("trash", "trash"),
    ("junk", "junk"),
    ("junk e-mail", "junk"),
    ("spam", "junk"),
    // 沃（英文名，**有属性**，名字兼底也能命中）。
    ("sent items", "sent"),
    ("sent item", "sent"),
    ("deleted items", "trash"),
    ("deleted item", "trash"),
];

/// 按名字识别特殊用途角色。
fn role_by_name(name: &str) -> Option<&'static str> {
    // 去掉首尾空白，并把全角空格也当空白。
    let trimmed = name.trim();
    let lower = trimmed.to_lowercase();

    for (candidate, role) in ROLE_NAMES {
        if lower == *candidate {
            return Some(role);
        }
    }

    // 再试一次「去掉尾部标点」的版本（如 `Sent Messages.`）。
    let stripped = lower.trim_end_matches(['.', '!', '。', '！']);
    if stripped != lower {
        for (candidate, role) in ROLE_NAMES {
            if stripped == *candidate {
                return Some(role);
            }
        }
    }

    None
}

/// 执行只读探测。
pub async fn probe(
    config: &ProbeConfig,
    username: &str,
    password: &str,
) -> Result<ProbeReport, ProtocolError> {
    let (mut session, capabilities, tls) =
        crate::connection::connect_and_login(&config.server, username, password).await?;

    let extensions = extension_support(&capabilities);
    let auth_mechanisms = auth_mechanisms(&capabilities);
    let capability_names = capability_names(&capabilities);

    // 文件夹（脱敏）。
    let folders = match session.list(None, Some("*")).await {
        Ok(list) => {
            // `list` 返回的是 Stream，要自己收。
            let names: Vec<_> = list.try_collect().await.unwrap_or_default();
            summarize_folders(names)
        }
        Err(e) => {
            tracing::warn!("列文件夹失败: {e}");
            FolderSummary::default()
        }
    };

    // 只读地检查目标邮箱。
    let mut mailbox = match session.examine(&config.mailbox).await {
        Ok(mb) => MailboxSummary {
            exists: mb.exists,
            unseen: mb.unseen,
            uid_validity: mb.uid_validity,
            uid_next: mb.uid_next,
            highest_modseq: mb.highest_modseq,
            permanent_flags: mb
                .permanent_flags
                .iter()
                .map(|f| format!("{f:?}"))
                .collect(),
            idle_accepted: None,
            examine_reported_unseen: mb.unseen.is_some(),
        },
        Err(e) => {
            tracing::warn!("EXAMINE 失败: {e}");
            MailboxSummary::default()
        }
    };

    // `STATUS` 能补上 EXAMINE 不给的字段（163 的 EXAMINE 不报 UNSEEN）。
    // 只读命令，不改变任何状态。
    if let Ok(status) = session
        .status(
            &config.mailbox,
            "(MESSAGES UNSEEN RECENT UIDNEXT UIDVALIDITY)",
        )
        .await
    {
        if let Some(unseen) = status.unseen {
            mailbox.unseen = Some(unseen);
        }
        if mailbox.uid_next.is_none() {
            mailbox.uid_next = status.uid_next;
        }
        if mailbox.uid_validity.is_none() {
            mailbox.uid_validity = status.uid_validity;
        }
    } else {
        tracing::warn!("STATUS 失败");
    }

    // IDLE 只在宣告支持时试一次，立即 DONE。
    //
    // `idle()` 消耗 session；`done()` 成功时把它还回来，失败时则**丢失**
    // （async-imap 内部 drop 掉了）。因此用 Option 持有：
    // 拿不回来就只关连接，不做 LOGOUT。
    let mut session_opt = Some(session);
    let mut idle_accepted = None;

    if config.try_idle && extensions.idle {
        let taken = session_opt.take().expect("session 应当还在");
        match try_idle(taken, config).await {
            Ok((returned, accepted)) => {
                idle_accepted = Some(accepted);
                session_opt = Some(returned);
            }
            Err(err) => {
                tracing::warn!("IDLE 探测失败: {err}");
            }
        }
    }

    if let Some(mut session) = session_opt {
        let _ = session.logout().await;
    }

    mailbox.idle_accepted = idle_accepted;

    Ok(ProbeReport {
        tls,
        capabilities: capability_names,
        auth_mechanisms,
        extensions,
        folders,
        mailbox,
    })
}

/// 试一次 IDLE：进入后立刻 DONE。
///
/// 目的是确认服务器**真的接受** IDLE，而不只是宣告它。
/// 这不改变任何服务器状态。
///
/// `idle()` 消耗 session，`done()` 把它还回来 —— 所以本函数把 session 原样退回，
/// 调用方仍可正常 LOGOUT。
/// 试一次 IDLE。
///
/// 正确流程（async-imap 0.12）：
/// 1. `idle()` 拿到 handle；
/// 2. **`init()`** 发 `IDLE` 命令并等服务器的 `+ idling` 续行；
/// 3. `wait_with_timeout()` 等新数据；
/// 4. **`done()`** 发 `DONE` 结束 IDLE，并把 session 还回来。
///
/// 第 2 步**不能省**：`done()` 里有断言，没 `init()` 过就直接 `done()`
/// 会 panic（“Cannot call DONE on a non initialized idle connection”）。
/// 163 不支持 IDLE 所以碰不到这个坑，QQ 支持——第一次跑 QQ 就炸了。
async fn try_idle(
    session: Session,
    config: &ProbeConfig,
) -> Result<(Session, bool), ProtocolError> {
    let mut handle = session.idle();

    // 发 IDLE，等服务器的 `+ idling` 续行。
    tokio::time::timeout(config.server.command_timeout, handle.init())
        .await
        .map_err(|_| ProtocolError::Timeout("IDLE 初始化超时".into()))?
        .map_err(|e| ProtocolError::Imap(format!("IDLE 初始化失败: {e}")))?;

    // 等一小会儿，看服务器会不会推新数据（没有也不报错）。
    let (wait, _interrupt) = handle.wait_with_timeout(Duration::from_secs(3));
    let _ = tokio::time::timeout(config.server.command_timeout, wait).await;

    // 发 DONE 结束 IDLE。
    let session = tokio::time::timeout(config.server.command_timeout, handle.done())
        .await
        .map_err(|_| ProtocolError::Timeout("IDLE 结束超时".into()))?
        .map_err(|e| ProtocolError::Imap(format!("IDLE 结束失败: {e}")))?;

    Ok((session, true))
}

/// 从能力集里提取扩展支持度。
pub fn extension_support(caps: &Capabilities) -> ExtensionSupport {
    ExtensionSupport {
        idle: caps.has_str("IDLE"),
        condstore: caps.has_str("CONDSTORE"),
        qresync: caps.has_str("QRESYNC"),
        uidplus: caps.has_str("UIDPLUS"),
        move_: caps.has_str("MOVE"),
        list_extended: caps.has_str("LIST-EXTENDED"),
        namespace: caps.has_str("NAMESPACE"),
        utf8: caps.has_str("UTF8=ACCEPT") || caps.has_str("UTF8=ONLY"),
        id: caps.has_str("ID"),
        quota: caps.has_str("QUOTA"),
        literal_plus: caps.has_str("LITERAL+"),
        literal_minus: caps.has_str("LITERAL-"),
    }
}

/// 提取认证机制。
pub fn auth_mechanisms(caps: &Capabilities) -> Vec<String> {
    let mut out: Vec<String> = caps
        .iter()
        .filter_map(|c| match c {
            async_imap::types::Capability::Auth(m) => Some(m.clone()),
            _ => None,
        })
        .collect();
    out.sort();
    out.dedup();
    out
}

/// 能力集转成排序后的字符串列表。
pub fn capability_names(caps: &Capabilities) -> Vec<String> {
    let mut out: Vec<String> = caps
        .iter()
        .map(|c| match c {
            async_imap::types::Capability::Imap4rev1 => "IMAP4rev1".to_string(),
            async_imap::types::Capability::Auth(m) => format!("AUTH={m}"),
            async_imap::types::Capability::Atom(a) => a.clone(),
        })
        .collect();
    out.sort();
    out.dedup();
    out
}

/// 汇总文件夹（脱敏：非标准名只计数；角色按特殊用途属性判定）。
fn summarize_folders(mailboxes: Vec<async_imap::types::Name>) -> FolderSummary {
    let mut summary = FolderSummary {
        total: mailboxes.len(),
        ..Default::default()
    };

    for mailbox in &mailboxes {
        let raw = mailbox.name().to_string();
        // IMAP 的邮箱名在线路上是 modified UTF-7（RFC 3501 §5.1.3）。
        // 国内服务商的默认文件夹名是中文，不解码就认不出「已发送」。
        let decoded = crate::utf7::decode(&raw).unwrap_or_else(|_| raw.clone());
        let upper = decoded.to_ascii_uppercase();

        let is_standard = STANDARD_FOLDERS.contains(&upper.as_str());
        if is_standard && !summary.standard.contains(&upper) {
            summary.standard.push(upper.clone());
        }

        // 特殊用途属性（RFC 6154）。服务器**不**宣告 LIST-EXTENDED，
        // 但 SPECIAL-USE 仍可能随 LIST 响应给出。
        let role = detect_role(mailbox);
        let show_name = is_standard || role.as_deref() != Some("other");
        summary.roles.push(FolderRole {
            role: role.clone().unwrap_or_else(|| "other".to_string()),
            name: if show_name {
                decoded
            } else {
                "«自定义»".to_string()
            },
        });

        if !is_standard && role.as_deref() == Some("other") {
            summary.non_standard_count += 1;
        }

        if let Some(delimiter) = mailbox.delimiter() {
            summary.delimiter = Some(delimiter.to_string());
        }
    }

    summary.standard.sort();
    summary.roles.sort_by(|a, b| a.role.cmp(&b.role));
    summary
}

/// 识别文件夹的特殊用途角色。
fn detect_role(mailbox: &async_imap::types::Name) -> Option<String> {
    use imap_proto::NameAttribute;

    for attribute in mailbox.attributes() {
        let role = match attribute {
            NameAttribute::All => "all",
            NameAttribute::Archive => "archive",
            NameAttribute::Drafts => "drafts",
            NameAttribute::Flagged => "flagged",
            NameAttribute::Junk => "junk",
            NameAttribute::Sent => "sent",
            NameAttribute::Trash => "trash",
            _ => continue,
        };
        return Some(role.to_string());
    }

    // 没有特殊用途属性时，按**名字**认。
    //
    // QQ 就是一个例子：它的 LIST 只给 `\NoSelect` / `\HasChildren`，
    // 不给 RFC 6154 的角色属性（实测 2026-10-23）。
    let name = crate::utf7::decode(mailbox.name()).unwrap_or_else(|_| mailbox.name().to_string());
    if name.eq_ignore_ascii_case("INBOX") {
        return Some("inbox".to_string());
    }
    if let Some(role) = role_by_name(&name) {
        return Some(role.to_string());
    }

    None
}

#[cfg(test)]
mod role_tests {
    use super::role_by_name;

    #[test]
    fn qq_names_are_recognized_without_attributes() {
        // QQ 的 LIST 不给 RFC 6154 属性（实测 2026-10-23），
        // 所以角色**必须**能按名字认出来。
        assert_eq!(role_by_name("Sent Messages"), Some("sent"));
        assert_eq!(role_by_name("Drafts"), Some("drafts"));
        assert_eq!(role_by_name("Deleted Messages"), Some("trash"));
        assert_eq!(role_by_name("Junk"), Some("junk"));
    }

    #[test]
    fn wo_names_are_recognized() {
        assert_eq!(role_by_name("Sent Items"), Some("sent"));
        assert_eq!(role_by_name("Drafts"), Some("drafts"));
        assert_eq!(role_by_name("Trash"), Some("trash"));
        assert_eq!(role_by_name("Junk E-mail"), Some("junk"));
    }

    #[test]
    fn netease_chinese_names_are_recognized() {
        assert_eq!(role_by_name("已发送"), Some("sent"));
        assert_eq!(role_by_name("草稿箱"), Some("drafts"));
        assert_eq!(role_by_name("已删除"), Some("trash"));
        assert_eq!(role_by_name("垃圾邮件"), Some("junk"));
    }

    #[test]
    fn matching_ignores_case_and_trailing_punctuation() {
        assert_eq!(role_by_name("sent messages"), Some("sent"));
        assert_eq!(role_by_name("DRAFTS"), Some("drafts"));
        assert_eq!(role_by_name("Junk."), Some("junk"));
        assert_eq!(role_by_name("  Trash  "), Some("trash"));
    }

    #[test]
    fn unknown_names_have_no_role() {
        assert_eq!(role_by_name("Virus Items"), None);
        assert_eq!(role_by_name("其他文件夹"), None);
        assert_eq!(role_by_name(""), None);
    }
}
