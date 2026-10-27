//! 服务器上的文件夹。
//!
//! 同步引擎要认「哪个文件夹是已发送 / 草稿箱 / 垃圾邮件」，
//! 而**各服务商的给法不一样**：
//!
//! * 163 / 沃：LIST 返回 RFC 6154 的特殊用途属性（`\Sent` / `\Drafts` / …）；
//! * **QQ：不给**——实测只给 `\NoSelect` / `\HasChildren`，
//!   哪怕它宣告了 XLIST（2026-10-23）。
//!
//! 所以角色识别是**属性优先、名字兜底**。
//!
//! 文件夹名在线路上是 **modified UTF-7**（RFC 3501 §5.1.3），
//! 这里已经解码成明文。

use crate::error::ProtocolError;

/// 服务器上的一个文件夹。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteFolder {
    /// 文件夹名（**已解码** modified UTF-7，便于显示与认角色）。
    pub name: String,
    /// 文件夹名在**线路上的原样**（modified UTF-7）。
    ///
    /// **发命令必须用它**：服务器只认自己给出的那个形式。
    /// 拿解码后的名字去 `EXAMINE`，服务器会回
    /// `NO EXAMINE Folder not exist`（实测 2026-10-26）。
    pub raw_name: String,
    /// 层级分隔符。
    pub delimiter: Option<String>,
    /// 特殊用途角色：`sent` / `drafts` / `trash` / `junk` / `inbox` / `archive` / `flagged` / `all`。
    ///
    /// `None` 表示**认不出**——不要猜。
    pub role: Option<String>,
    /// 服务器侧的特殊用途属性（原始）。
    pub attributes: Vec<String>,
}

impl RemoteFolder {
    /// 是不是 INBOX。
    pub fn is_inbox(&self) -> bool {
        self.name.eq_ignore_ascii_case("INBOX")
    }

    /// 角色是否是 `role`。
    pub fn has_role(&self, role: &str) -> bool {
        self.role.as_deref() == Some(role)
    }
}

/// 列出服务器上的文件夹。
///
/// 只读命令（`LIST`）。
///
/// **不走 `session.list()`**：async-imap 的 `parse_names` 与 `parse_fetches`
/// 一样会**吞掉**带 tag 的 `NO` / `BAD`（见 [`crate::command`] 的说明），
/// 服务器拒绝时只会得到一个空列表——看起来像「这个邮箱没有文件夹」。
/// 这里用安全命令层自己读响应。
pub async fn list_folders(
    session: &mut crate::connection::Session,
) -> Result<Vec<RemoteFolder>, ProtocolError> {
    let outcome = crate::command::run_checked(session, "LIST \"\" \"*\"").await?;

    Ok(outcome
        .list()
        .iter()
        .map(|mailbox| {
            let name =
                crate::utf7::decode(&mailbox.raw_name).unwrap_or_else(|_| mailbox.raw_name.clone());
            RemoteFolder {
                name,
                raw_name: mailbox.raw_name.clone(),
                delimiter: mailbox.delimiter.clone(),
                role: mailbox.role.clone(),
                attributes: mailbox.attributes.clone(),
            }
        })
        .collect())
}

/// 按 RFC 6154 的特殊用途**属性**识别角色。
pub fn role_from_attributes(attributes: &[imap_proto::NameAttribute<'_>]) -> Option<String> {
    for attribute in attributes {
        let role = match attribute {
            imap_proto::NameAttribute::All => "all",
            imap_proto::NameAttribute::Archive => "archive",
            imap_proto::NameAttribute::Drafts => "drafts",
            imap_proto::NameAttribute::Flagged => "flagged",
            imap_proto::NameAttribute::Junk => "junk",
            imap_proto::NameAttribute::Sent => "sent",
            imap_proto::NameAttribute::Trash => "trash",
            _ => continue,
        };
        return Some(role.to_string());
    }
    None
}

/// 识别文件夹的特殊用途角色。
///
/// **属性优先，名字兜底**（QQ 不给属性，见模块文档）。
pub fn detect_role(mailbox: &async_imap::types::Name) -> Option<String> {
    if let Some(role) = role_from_attributes(mailbox.attributes()) {
        return Some(role);
    }

    // 没有特殊用途属性时，按**名字**认。
    let name = crate::utf7::decode(mailbox.name()).unwrap_or_else(|_| mailbox.name().to_string());
    if name.eq_ignore_ascii_case("INBOX") {
        return Some("inbox".to_string());
    }
    role_by_name(&name).map(str::to_string)
}

/// 按**名字**识别特殊用途角色的兼底表。
///
/// 来自三家服务商的**实测**（2026-10-23）。
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
pub fn role_by_name(name: &str) -> Option<&'static str> {
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

#[cfg(test)]
mod tests {
    use super::*;

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
