//! IMAP 搜索键解析与求值。
//!
//! 语法（RFC 3501 §6.4.4）：
//!
//! ```text
//! search = 1*search-key
//! search-key = "ALL" / "ANSWERED" / ... / search-key *(SP search-key)
//! ```
//!
//! 多个搜索键之间是**隐式 AND**。
//!
//! ## 范围
//!
//! 支持标记类、UID 集合、头字段、正文/全文、日期类，以及 `NOT` / `OR`。
//! 不支持的键**显式报错**，而不是悄悄当成「匹配全部」——
//! 后者会让测试看起来通过、实际什么都没测。

use crate::config::MessageFixture;
use crate::sequence::SequenceSet;

/// 搜索键解析错误。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SearchError {
    /// 语法错误。
    #[error("搜索键语法错误: {0}")]
    Invalid(String),
    /// 不支持的搜索键。
    #[error("不支持的搜索键: {0}")]
    Unsupported(String),
}

/// 一个搜索键。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchKey {
    /// 全部邮件。
    All,
    /// 有 `\Seen`。
    Seen,
    /// 没有 `\Seen`。
    Unseen,
    /// 有 `\Flagged`。
    Flagged,
    /// 没有 `\Flagged`。
    Unflagged,
    /// 有 `\Deleted`。
    Deleted,
    /// 没有 `\Deleted`。
    Undeleted,
    /// 有 `\Answered`。
    Answered,
    /// 没有 `\Answered`。
    Unanswered,
    /// 有 `\Draft`。
    Draft,
    /// 没有 `\Draft`。
    Undraft,
    /// 最近到达且未读。
    New,
    /// 最近到达。
    Recent,
    /// 不是最近到达。
    Old,
    /// 有关键字。
    Keyword(String),
    /// 没有关键字。
    Unkeyword(String),
    /// UID 落在集合里。
    Uid(SequenceSet),
    /// 某个头字段包含给定文本。
    Header(String, String),
    /// 主题包含给定文本。
    Subject(String),
    /// 发件人包含给定文本。
    From(String),
    /// 收件人包含给定文本。
    To(String),
    /// 抄送包含给定文本。
    Cc(String),
    /// 密送包含给定文本。
    Bcc(String),
    /// 正文包含给定文本。
    Body(String),
    /// 整封邮件包含给定文本。
    Text(String),
    /// 日期在给定日期之后。
    Since(String),
    /// 日期在给定日期之前。
    Before(String),
    /// 日期就是给定日期。
    On(String),
    /// 取反。
    Not(Box<SearchKey>),
    /// 二选一。
    Or(Box<SearchKey>, Box<SearchKey>),
    /// 多个键（隐式 AND）。
    And(Vec<SearchKey>),
}

/// 解析搜索表达式。
///
/// ```
/// use xremail_testkit::search::{parse, SearchKey};
/// let key = parse("UNSEEN").unwrap();
/// assert_eq!(key, SearchKey::Unseen);
/// ```
pub fn parse(input: &str) -> Result<SearchKey, SearchError> {
    let tokens = tokenize(input)?;
    let mut parser = Parser { tokens, pos: 0 };
    let key = parser.parse_list()?;
    if parser.pos < parser.tokens.len() {
        return Err(SearchError::Invalid("有多余的内容".into()));
    }
    Ok(key)
}

/// 一个词法单元。
#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    /// 裸词。
    Atom(String),
    /// 带引号的字符串。
    Quoted(String),
    /// `(`。
    Open,
    /// `)`。
    Close,
}

/// 切词。
fn tokenize(input: &str) -> Result<Vec<Token>, SearchError> {
    let mut tokens = Vec::new();
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            c if c.is_whitespace() => {}
            '(' => tokens.push(Token::Open),
            ')' => tokens.push(Token::Close),
            '"' => {
                let mut value = String::new();
                loop {
                    let Some(inner) = chars.next() else {
                        return Err(SearchError::Invalid("引号没有闭合".into()));
                    };
                    match inner {
                        '\\' => {
                            let Some(escaped) = chars.next() else {
                                return Err(SearchError::Invalid("转义没有闭合".into()));
                            };
                            value.push(escaped);
                        }
                        '"' => break,
                        other => value.push(other),
                    }
                }
                tokens.push(Token::Quoted(value));
            }
            _ => {
                let mut atom = String::new();
                atom.push(ch);
                while let Some(&next) = chars.peek() {
                    if next.is_whitespace() || next == '(' || next == ')' {
                        break;
                    }
                    atom.push(next);
                    chars.next();
                }
                tokens.push(Token::Atom(atom));
            }
        }
    }

    if tokens.is_empty() {
        return Err(SearchError::Invalid("空的搜索表达式".into()));
    }
    Ok(tokens)
}

/// 递归下降解析器。
struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    /// 解析一串键（隐式 AND）。
    fn parse_list(&mut self) -> Result<SearchKey, SearchError> {
        let mut keys = Vec::new();
        while self.pos < self.tokens.len() {
            if self.tokens[self.pos] == Token::Close {
                break;
            }
            keys.push(self.parse_key()?);
        }

        match keys.len() {
            0 => Err(SearchError::Invalid("空的搜索表达式".into())),
            1 => Ok(keys.remove(0)),
            _ => Ok(SearchKey::And(keys)),
        }
    }

    /// 解析一个键。
    fn parse_key(&mut self) -> Result<SearchKey, SearchError> {
        let Some(token) = self.tokens.get(self.pos).cloned() else {
            return Err(SearchError::Invalid("搜索键不完整".into()));
        };
        self.pos += 1;

        let atom = match token {
            Token::Atom(atom) => atom,
            Token::Quoted(value) => {
                return Err(SearchError::Invalid(format!(
                    "不该出现在这里的关键字: {value}"
                )));
            }
            Token::Open => {
                // 括号包住一串键。
                let key = self.parse_list()?;
                if self.tokens.get(self.pos) != Some(&Token::Close) {
                    return Err(SearchError::Invalid("括号没有闭合".into()));
                }
                self.pos += 1;
                return Ok(key);
            }
            Token::Close => return Err(SearchError::Invalid("多余的右括号".into())),
        };

        let upper = atom.to_ascii_uppercase();
        match upper.as_str() {
            "ALL" => Ok(SearchKey::All),
            "SEEN" => Ok(SearchKey::Seen),
            "UNSEEN" => Ok(SearchKey::Unseen),
            "FLAGGED" => Ok(SearchKey::Flagged),
            "UNFLAGGED" => Ok(SearchKey::Unflagged),
            "DELETED" => Ok(SearchKey::Deleted),
            "UNDELETED" => Ok(SearchKey::Undeleted),
            "ANSWERED" => Ok(SearchKey::Answered),
            "UNANSWERED" => Ok(SearchKey::Unanswered),
            "DRAFT" => Ok(SearchKey::Draft),
            "UNDRAFT" => Ok(SearchKey::Undraft),
            "NEW" => Ok(SearchKey::New),
            "RECENT" => Ok(SearchKey::Recent),
            "OLD" => Ok(SearchKey::Old),
            "KEYWORD" => Ok(SearchKey::Keyword(self.expect_value()?)),
            "UNKEYWORD" => Ok(SearchKey::Unkeyword(self.expect_value()?)),
            "UID" => {
                let value = self.expect_value()?;
                let set = SequenceSet::parse(&value)
                    .map_err(|e| SearchError::Invalid(format!("UID 集合: {e}")))?;
                Ok(SearchKey::Uid(set))
            }
            "HEADER" => {
                let field = self.expect_value()?;
                let value = self.expect_value()?;
                Ok(SearchKey::Header(field, value))
            }
            "SUBJECT" => Ok(SearchKey::Subject(self.expect_value()?)),
            "FROM" => Ok(SearchKey::From(self.expect_value()?)),
            "TO" => Ok(SearchKey::To(self.expect_value()?)),
            "CC" => Ok(SearchKey::Cc(self.expect_value()?)),
            "BCC" => Ok(SearchKey::Bcc(self.expect_value()?)),
            "BODY" => Ok(SearchKey::Body(self.expect_value()?)),
            "TEXT" => Ok(SearchKey::Text(self.expect_value()?)),
            "SINCE" => Ok(SearchKey::Since(self.expect_value()?)),
            "BEFORE" => Ok(SearchKey::Before(self.expect_value()?)),
            "ON" => Ok(SearchKey::On(self.expect_value()?)),
            "NOT" => {
                let inner = self.parse_key()?;
                Ok(SearchKey::Not(Box::new(inner)))
            }
            "OR" => {
                let left = self.parse_key()?;
                let right = self.parse_key()?;
                Ok(SearchKey::Or(Box::new(left), Box::new(right)))
            }
            other => Err(SearchError::Unsupported(other.to_string())),
        }
    }

    /// 取一个值（裸词或引号字符串）。
    fn expect_value(&mut self) -> Result<String, SearchError> {
        match self.tokens.get(self.pos).cloned() {
            Some(Token::Atom(atom)) => {
                self.pos += 1;
                Ok(atom)
            }
            Some(Token::Quoted(value)) => {
                self.pos += 1;
                Ok(value)
            }
            _ => Err(SearchError::Invalid("缺少参数".into())),
        }
    }
}

/// 求值：这一封邮件是否命中。
///
/// `max_uid` 是邮箱里最大的 UID（`UID` 搜索键要用）。
pub fn matches(key: &SearchKey, message: &MessageFixture, max_uid: u32) -> bool {
    match key {
        SearchKey::All => true,
        SearchKey::Seen => message.has_flag("\\Seen"),
        SearchKey::Unseen => !message.has_flag("\\Seen"),
        SearchKey::Flagged => message.has_flag("\\Flagged"),
        SearchKey::Unflagged => !message.has_flag("\\Flagged"),
        SearchKey::Deleted => message.has_flag("\\Deleted"),
        SearchKey::Undeleted => !message.has_flag("\\Deleted"),
        SearchKey::Answered => message.has_flag("\\Answered"),
        SearchKey::Unanswered => !message.has_flag("\\Answered"),
        SearchKey::Draft => message.has_flag("\\Draft"),
        SearchKey::Undraft => !message.has_flag("\\Draft"),
        SearchKey::New => message.has_flag("\\Recent") && !message.has_flag("\\Seen"),
        SearchKey::Recent => message.has_flag("\\Recent"),
        SearchKey::Old => !message.has_flag("\\Recent"),
        SearchKey::Keyword(keyword) => message.has_flag(keyword),
        SearchKey::Unkeyword(keyword) => !message.has_flag(keyword),
        SearchKey::Uid(set) => set.contains(message.uid, max_uid),
        SearchKey::Header(field, value) => message
            .header_values(field)
            .iter()
            .any(|header| contains_ignore_case(header, value)),
        SearchKey::Subject(value) => message
            .header_values("Subject")
            .iter()
            .any(|header| contains_ignore_case(header, value)),
        SearchKey::From(value) => message
            .header_values("From")
            .iter()
            .any(|header| contains_ignore_case(header, value)),
        SearchKey::To(value) => message
            .header_values("To")
            .iter()
            .any(|header| contains_ignore_case(header, value)),
        SearchKey::Cc(value) => message
            .header_values("Cc")
            .iter()
            .any(|header| contains_ignore_case(header, value)),
        SearchKey::Bcc(value) => message
            .header_values("Bcc")
            .iter()
            .any(|header| contains_ignore_case(header, value)),
        SearchKey::Body(value) => contains_ignore_case(message.body(), value),
        SearchKey::Text(value) => contains_ignore_case(message.content(), value),
        SearchKey::Since(date) => message
            .date()
            .is_some_and(|d| d >= parse_search_date(date).unwrap_or(i32::MIN)),
        SearchKey::Before(date) => message
            .date()
            .is_some_and(|d| d < parse_search_date(date).unwrap_or(i32::MAX)),
        SearchKey::On(date) => message
            .date()
            .is_some_and(|d| d == parse_search_date(date).unwrap_or(i32::MIN)),
        SearchKey::Not(inner) => !matches(inner, message, max_uid),
        SearchKey::Or(left, right) => {
            matches(left, message, max_uid) || matches(right, message, max_uid)
        }
        SearchKey::And(keys) => keys.iter().all(|key| matches(key, message, max_uid)),
    }
}

/// 大小写不敏感的子串匹配。
fn contains_ignore_case(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    haystack.to_lowercase().contains(&needle.to_lowercase())
}

/// 解析搜索键里的日期（`01-Jan-2024`）。
fn parse_search_date(token: &str) -> Option<i32> {
    let parts: Vec<&str> = token.split('-').collect();
    if parts.len() != 3 {
        return None;
    }
    let day: u32 = parts[0].trim().parse().ok()?;
    let month = month_number(parts[1].trim())?;
    let year: i32 = parts[2].trim().parse().ok()?;
    Some(year * 10_000 + month * 100 + day as i32)
}

/// 月份名 → 月份号。
fn month_number(name: &str) -> Option<i32> {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    MONTHS
        .iter()
        .position(|month| month.eq_ignore_ascii_case(name))
        .map(|index| index as i32 + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_flag_keys() {
        assert_eq!(parse("ALL").unwrap(), SearchKey::All);
        assert_eq!(parse("unseen").unwrap(), SearchKey::Unseen);
        assert_eq!(parse("FLAGGED").unwrap(), SearchKey::Flagged);
    }

    #[test]
    fn parses_quoted_values() {
        assert_eq!(
            parse(r#"SUBJECT "会议纪要""#).unwrap(),
            SearchKey::Subject("会议纪要".into())
        );
    }

    #[test]
    fn parses_uid_key() {
        let key = parse("UID 1:10").unwrap();

        assert!(matches!(key, SearchKey::Uid(_)));
    }

    #[test]
    fn parses_header_key_with_two_values() {
        assert_eq!(
            parse("HEADER Subject hello").unwrap(),
            SearchKey::Header("Subject".into(), "hello".into())
        );
    }

    #[test]
    fn implicit_and_becomes_and_node() {
        let key = parse("UNSEEN NOT FROM spam").unwrap();

        match key {
            SearchKey::And(keys) => assert_eq!(keys.len(), 2),
            other => panic!("期望 And，实际是 {other:?}"),
        }
    }

    #[test]
    fn parses_not_and_or() {
        assert!(matches!(parse("NOT SEEN").unwrap(), SearchKey::Not(_)));
        assert!(matches!(
            parse("OR SEEN FLAGGED").unwrap(),
            SearchKey::Or(_, _)
        ));
    }

    #[test]
    fn parentheses_group_keys() {
        let key = parse("(SEEN FLAGGED) UNSEEN").unwrap();

        match key {
            SearchKey::And(keys) => assert_eq!(keys.len(), 2),
            other => panic!("期望 And，实际是 {other:?}"),
        }
    }

    #[test]
    fn unsupported_key_is_an_explicit_error() {
        assert!(matches!(parse("BOGUS"), Err(SearchError::Unsupported(_))));
    }

    #[test]
    fn malformed_input_is_an_error() {
        assert!(parse("").is_err());
        assert!(parse("NOT").is_err());
        assert!(parse("HEADER Subject").is_err());
        assert!(parse("(SEEN").is_err());
    }

    #[test]
    fn parses_dates() {
        assert_eq!(parse_search_date("01-Jan-2024"), Some(20240101));
        assert_eq!(parse_search_date("31-Dec-1999"), Some(19991231));
        assert_eq!(parse_search_date("garbage"), None);
    }
}
