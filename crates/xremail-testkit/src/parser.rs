//! 最小 IMAP 命令解析。
//!
//! 只解析假服务器需要覆盖的命令，**不**追求完整实现。
//! 目标是：能看清客户端到底发了什么，以便断言行为。

/// 一条解析出来的命令。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command {
    /// 命令标签（客户端生成的 `a1`、`C42` 之类）。
    pub tag: String,
    /// 命令名（大写）。
    pub name: String,
    /// 参数原文（已去掉命令名）。
    pub args: String,
}

impl Command {
    /// 解析一行。
    ///
    /// 返回 `None` 表示这不是一条命令（空行等）。
    pub fn parse(line: &str) -> Option<Self> {
        let line = line.trim_end_matches(['\r', '\n']);
        if line.trim().is_empty() {
            return None;
        }

        let (tag, rest) = line.split_once(' ')?;
        if tag.is_empty() {
            return None;
        }

        let rest = rest.trim_start();
        let (name, args) = match rest.split_once(' ') {
            Some((name, args)) => (name, args),
            None => (rest, ""),
        };

        Some(Self {
            tag: tag.to_string(),
            name: name.to_ascii_uppercase(),
            args: args.trim().to_string(),
        })
    }

    /// 参数里的第一个引号字符串。
    pub fn first_quoted(&self) -> Option<String> {
        quoted_values(&self.args).into_iter().next()
    }

    /// 参数里的所有引号字符串。
    pub fn quoted_values(&self) -> Vec<String> {
        quoted_values(&self.args)
    }

    /// 如果这是 `UID <子命令>` 形式，返回 `(子命令, 剩余参数)`。
    ///
    /// ```
    /// use xremail_testkit::parser::Command;
    /// let cmd = Command::parse("a1 UID FETCH 1:* (UID FLAGS)").unwrap();
    /// let (sub, rest) = cmd.uid_subcommand().unwrap();
    /// assert_eq!(sub, "FETCH");
    /// assert_eq!(rest, "1:* (UID FLAGS)");
    /// ```
    pub fn uid_subcommand(&self) -> Option<(String, &str)> {
        if self.name != "UID" {
            return None;
        }
        let rest = self.args.trim();
        let (sub, tail) = rest.split_once(char::is_whitespace)?;
        Some((sub.to_ascii_uppercase(), tail.trim()))
    }

    /// 命令里声明的**字面量长度**（`{123}` 或同步字面量 `{123+}`）。
    ///
    /// `APPEND` 的正文就是字面量：命令头里写着长度，
    /// 服务器回 `+` 之后客户端才发那么多字节过来。
    ///
    /// ```
    /// # use xremail_testkit::parser::Command;
    /// # let cmd = Command::parse("a1 APPEND \"INBOX\" {12+}").unwrap();
    /// # assert_eq!(cmd.literal_size(), Some(12));
    /// ```
    pub fn literal_size(&self) -> Option<usize> {
        let open = self.args.rfind('{')?;
        let rest = &self.args[open + 1..];
        let close = rest.find('}')?;
        let digits = &rest[..close];
        let digits = digits.strip_suffix('+').unwrap_or(digits);
        digits.parse().ok()
    }
}

/// 取出参数里所有引号字符串（`"..."`），并处理 `\"` 转义。
pub fn quoted_values(args: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut chars = args.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch != '"' {
            continue;
        }
        let mut value = String::new();
        while let Some(inner) = chars.next() {
            match inner {
                '\\' => {
                    if let Some(escaped) = chars.next() {
                        value.push(escaped);
                    }
                }
                '"' => break,
                other => value.push(other),
            }
        }
        out.push(value);
    }

    out
}

/// 把字符串按 IMAP 的 quoted-string 规则编码（含必要的转义）。
pub fn quote(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        if ch == '"' || ch == '\\' {
            out.push('\\');
        }
        out.push(ch);
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_tag_name_and_args() {
        let cmd = Command::parse("a1 LOGIN user pass").unwrap();

        assert_eq!(cmd.tag, "a1");
        assert_eq!(cmd.name, "LOGIN");
        assert_eq!(cmd.args, "user pass");
    }

    #[test]
    fn parses_command_without_args() {
        let cmd = Command::parse("a2 CAPABILITY").unwrap();

        assert_eq!(cmd.name, "CAPABILITY");
        assert_eq!(cmd.args, "");
    }

    #[test]
    fn normalizes_case() {
        let cmd = Command::parse("a3 select inbox").unwrap();

        assert_eq!(cmd.name, "SELECT");
    }

    #[test]
    fn extracts_quoted_values() {
        let cmd = Command::parse(r#"a4 LOGIN "user@163.com" "pwd""#).unwrap();

        let values = cmd.quoted_values();
        assert_eq!(values, vec!["user@163.com", "pwd"]);
    }

    #[test]
    fn handles_escaped_quote() {
        let cmd = Command::parse(r#"a5 LOGIN "we\"ird" "pwd""#).unwrap();

        assert_eq!(cmd.quoted_values()[0], "we\"ird");
    }

    #[test]
    fn empty_line_is_not_a_command() {
        assert!(Command::parse("   ").is_none());
        assert!(Command::parse("").is_none());
    }

    #[test]
    fn quote_escapes_specials() {
        assert_eq!(quote(r#"a"b\c"#), r#""a\"b\\c""#);
    }
}
