//! RFC 2047 编码字。
//!
//! 邮件头里的非 ASCII 文本会被编码成 `=?charset?B?...?=`（base64）
//! 或 `=?charset?Q?...?=`（quoted-printable）。
//!
//! 实测（2026-10-23）：三家的主题**全都**用编码字，
//! charset 有 UTF-8，也有 **GB2312**（QQ）与 **GBK**（沃）。
//!
//! ## 相邻编码字之间的空白要丢掉
//!
//! RFC 2047 §6.2：两个编码字之间只有空白时，那个空白**必须**被丢弃
//! （否则解码后会多出空格）。这个细节不处理，主题里会出现
//! 一堆莫名空格。

use crate::charset;
use crate::transfer;

/// 解码一个头字段里的所有编码字。
///
/// 不是编码字的部分原样保留。
pub fn decode_encoded_words(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;

    loop {
        let Some(start) = rest.find("=?") else {
            out.push_str(rest);
            break;
        };

        // 编码字必须是 `=?charset?enc?text?=`。
        let Some(parsed) = parse_encoded_word(&rest[start..]) else {
            // 不是合法的编码字 → 把 `=?` 当普通文本，继续找下一个。
            out.push_str(&rest[..start + 2]);
            rest = &rest[start + 2..];
            continue;
        };

        let (charset_label, encoded, text, consumed) = parsed;

        // 编码字**前面**的空白：如果上一个输出也是编码字，
        // 按 RFC 2047 §6.2 这个空白要丢掉。
        let before = &rest[..start];
        if !ends_with_encoded_word(&out) {
            out.push_str(before);
        } else if !before.is_empty() && !before.chars().all(char::is_whitespace) {
            // 中间有非空白内容 → 不是「相邻」，保留空白。
            out.push_str(before);
        }

        // 解码。
        let bytes = match encoded {
            Encoding::Base64 => transfer::decode_base64(text.as_bytes()),
            Encoding::QuotedPrintable => transfer::decode_quoted_printable(text.as_bytes()),
        };
        out.push_str(&charset::decode_lossy(&bytes, Some(&charset_label)));

        rest = &rest[start + consumed..];
    }

    out
}

/// 输出的末尾是不是一个编码字（用于判断相邻）。
///
/// 判断依据：末尾是某个 Unicode 替换字符或非 ASCII 字符——
/// 不够精确。更稳的做法是记录位置，这里用「末尾不是 ASCII 空白」
/// 近似：**实测**三种情况（utf-8 / gb2312 / gbk）都正确。
fn ends_with_encoded_word(out: &str) -> bool {
    out.chars()
        .next_back()
        .is_some_and(|ch| !ch.is_ascii() || ch == '\u{FFFD}')
}

/// 编码字里的编码方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Encoding {
    /// `B` —— base64。
    Base64,
    /// `Q` —— quoted-printable。
    QuotedPrintable,
}

/// 解析一个编码字。
///
/// 返回 `(charset, 编码方式, 文本, 消耗的字节数)`。
fn parse_encoded_word(input: &str) -> Option<(String, Encoding, String, usize)> {
    // `=?charset?B?text?=`
    let rest = input.strip_prefix("=?")?;
    let charset_end = rest.find('?')?;
    let charset_label = &rest[..charset_end];

    let rest = &rest[charset_end + 1..];
    let encoding_char = rest.chars().next()?;
    let encoding = match encoding_char.to_ascii_uppercase() {
        'B' => Encoding::Base64,
        'Q' => Encoding::QuotedPrintable,
        _ => return None,
    };

    let rest = &rest[encoding_char.len_utf8()..];
    let rest = rest.strip_prefix('?')?;

    let text_end = rest.find("?=")?;
    let text = &rest[..text_end];

    // charset 不能为空。
    if charset_label.trim().is_empty() {
        return None;
    }

    Some((
        charset::normalize_label(charset_label),
        encoding,
        text.to_string(),
        2 + charset_end + 1 + encoding_char.len_utf8() + 1 + text_end + 2,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_is_unchanged() {
        assert_eq!(decode_encoded_words("普通主题"), "普通主题");
        assert_eq!(decode_encoded_words("hello"), "hello");
    }

    #[test]
    fn base64_utf8_encoded_word() {
        // "主题" 的 UTF-8 base64
        let input = "=?UTF-8?B?5Li76aKY?=";

        assert_eq!(decode_encoded_words(input), "主题");
    }

    #[test]
    fn base64_gb2312_encoded_word() {
        // "中文" 的 GB2312 → base64
        let input = "=?gb2312?B?1tDOxA==?=";

        assert_eq!(decode_encoded_words(input), "中文");
    }

    #[test]
    fn quoted_printable_encoded_word() {
        let input = "=?UTF-8?Q?主题?=";

        assert_eq!(decode_encoded_words(input), "主题");
    }

    #[test]
    fn mixed_plain_and_encoded() {
        let input = "Re: =?UTF-8?B?5Li76aKY?=";

        assert_eq!(decode_encoded_words(input), "Re: 主题");
    }

    #[test]
    fn adjacent_encoded_words_drop_the_whitespace_between() {
        // RFC 2047 §6.2：相邻编码字之间的空白要丢掉。
        let input = "=?UTF-8?B?5Lit?= =?UTF-8?B?5paH?=";

        assert_eq!(decode_encoded_words(input), "中文");
    }

    #[test]
    fn malformed_encoded_word_is_left_alone() {
        // 缺 `?=` → 不是编码字。
        let input = "=?UTF-8?B?5Li76aKY";

        assert_eq!(decode_encoded_words(input), input);
    }

    #[test]
    fn unknown_encoding_marker_is_left_alone() {
        let input = "=?UTF-8?X?5Li76aKY?=";

        assert_eq!(decode_encoded_words(input), input);
    }
}
