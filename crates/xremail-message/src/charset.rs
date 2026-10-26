//! 字符集解码。
//!
//! ## 为什么需要它
//!
//! 2026-10-23 实测三家国内邮箱：
//!
//! | 服务商 | 正文 charset | 主题 charset |
//! |---|---|---|
//! | 163 | UTF-8 | UTF-8 |
//! | QQ | **GB2312** + UTF-8 | **GB2312** + UTF-8 |
//! | 沃 | **GBK** + UTF-8 | **GBK** + UTF-8 |
//!
//! 而且**大量邮件根本没有 charset**（QQ 20 封里有 11 封、沃 6 封里有 5 封），
//! 所以「猜」是必须的能力，不是可选项。
//!
//! ## 猜测策略
//!
//! 1. 有 charset → 按它解（`gb2312` 走 GBK 解码器，两者兼容）；
//! 2. 没 charset → **先严格试 UTF-8**（能过就用 UTF-8）；
//! 3. UTF-8 不过 → **试 GB18030**（GBK / GB2312 都是它的子集）；
//! 4. 还不行 → 退到 windows-1252（几乎不会失败，至少不会丢字节）。
//!
//! ## 为什么用 encoding_rs
//!
//! 它是 WHATWG Encoding Standard 的 Rust 实现，覆盖 UTF-8 / GBK / GB18030 /
//! Big5 / Shift_JIS / ISO-8859-* 等，而且**解码不会丢字节**
//! （非法序列替换成 U+FFFD，但会告诉你「有错误」）。

use encoding_rs::{Encoding, UTF_8};

/// 解析失败（不该发生——解码器总是能给出结果）。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum CharsetError {
    /// 编码标签不认识，且也没给猜测的机会。
    #[error("不认识的字符集: {0}")]
    UnknownLabel(String),
}

/// 把编码标签解析成编码。
///
/// 标签比较不区分大小写，且接受带引号的值（`charset="gb2312"`）。
pub fn encoding_for_label(label: &str) -> Option<&'static Encoding> {
    let label = label.trim().trim_matches('"').trim();
    if label.is_empty() {
        return None;
    }
    Encoding::for_label(label.as_bytes())
}

/// 猜一个编码（当邮件没给 charset 时用）。
///
/// 见模块文档的策略。
pub fn guess_encoding(bytes: &[u8]) -> &'static Encoding {
    // 1. 严格 UTF-8：BOM 也要认。
    if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        return UTF_8;
    }
    if std::str::from_utf8(bytes).is_ok() {
        return UTF_8;
    }

    // 2. GB18030：GBK / GB2312 都是它的子集。
    //
    // 注意：**不能**只靠「是不是合法 GBK」来判断——GBK 的字节空间太大，
    // 几乎任何随机字节都是「合法 GBK」。所以这里用 GB18030 只是**兜底**，
    // 不声称它一定对。
    if let Some(encoding) = Encoding::for_label(b"gb18030") {
        let (_, _, had_errors) = encoding.decode(bytes);
        if !had_errors {
            return encoding;
        }
    }

    // 3. windows-1252：几乎所有字节都有对应，不会丢。
    Encoding::for_label(b"windows-1252").unwrap_or(UTF_8)
}

/// 解码字节为文本。
///
/// 返回 `(文本, 实际用的编码, 是否出现了解码错误)`。
pub fn decode(bytes: &[u8], charset: Option<&str>) -> (String, &'static Encoding, bool) {
    let encoding = match charset {
        Some(label) => encoding_for_label(label).unwrap_or_else(|| guess_encoding(bytes)),
        None => guess_encoding(bytes),
    };

    let (text, _, had_errors) = encoding.decode(bytes);
    (text.into_owned(), encoding, had_errors)
}

/// 解码字节为文本，忽略「有没有错」。
pub fn decode_lossy(bytes: &[u8], charset: Option<&str>) -> String {
    decode(bytes, charset).0
}

/// 规范化编码标签（用于存库与比较）。
///
/// `gb2312` 与 `gbk` **不合并**——它们在数据上可能不同，
/// 合并会丢信息。统一小写、去空白、去引号。
pub fn normalize_label(label: &str) -> String {
    label.trim().trim_matches('"').trim().to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_labels_resolve() {
        assert!(encoding_for_label("utf-8").is_some());
        assert!(encoding_for_label("UTF-8").is_some());
        assert!(encoding_for_label("gb2312").is_some());
        assert!(encoding_for_label("gbk").is_some());
        assert!(encoding_for_label("gb18030").is_some());
        assert!(encoding_for_label("big5").is_some());
    }

    #[test]
    fn quoted_and_padded_labels_resolve() {
        assert!(encoding_for_label("  \"gb2312\" ").is_some());
    }

    #[test]
    fn unknown_label_is_none() {
        assert!(encoding_for_label("x-unknown-charset").is_none());
        assert!(encoding_for_label("").is_none());
    }

    #[test]
    fn gb2312_bytes_decode_to_chinese() {
        // “中文” 的 GB2312 编码。
        let bytes = [0xD6, 0xD0, 0xCE, 0xC4];

        let (text, _, errors) = decode(&bytes, Some("gb2312"));

        assert_eq!(text, "中文");
        assert!(!errors, "合法的 GB2312 不该有解码错误");
    }

    #[test]
    fn utf8_is_used_when_no_charset_and_content_is_valid() {
        let bytes = "中文".as_bytes();

        let (text, encoding, _) = decode(bytes, None);

        assert_eq!(text, "中文");
        assert_eq!(encoding.name(), "UTF-8");
    }

    #[test]
    fn gbk_is_guessed_when_utf8_fails() {
        // “中啊” 的 GBK 字节：是合法 GB18030，但不是合法 UTF-8。
        let bytes = [0xD6, 0xD0, 0xB0, 0xA1];

        let (text, _, _) = decode(&bytes, None);

        assert!(text.contains('中'), "应当猜成 GBK 一类，实际：{text}");
        assert!(text.contains('啊'), "GBK 扩展字也要解出来，实际：{text}");
    }

    #[test]
    fn unknown_charset_falls_back_to_guessing() {
        let bytes = "中文".as_bytes();

        let (text, _, _) = decode(bytes, Some("x-nonsense"));

        assert_eq!(text, "中文");
    }

    #[test]
    fn labels_are_normalized() {
        assert_eq!(normalize_label("  \"GB2312\" "), "gb2312");
        assert_eq!(normalize_label("UTF-8"), "utf-8");
    }
}
