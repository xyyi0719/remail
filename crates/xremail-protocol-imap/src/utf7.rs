//! modified UTF-7 编解码（RFC 3501 §5.1.3）。
//!
//! ## 为什么需要
//!
//! IMAP 的**邮箱名**（不是邮件头！）在线路上用 modified UTF-7 传输。
//! 国内服务商的默认文件夹名是中文（已发送、草稿箱、垃圾邮件……），
//! 因此**必须**解码才能知道哪个文件夹是「已发送」。
//!
//! 163 的实测：文件夹名在线路上是 `&XfJT0ZAB-` 这样的形式，
//! 解码后是「已发送」。不解码的话，客户端只能看到乱码般的字符串，
//! 也无法把文件夹映射到语义角色。
//!
//! ## 算法要点
//!
//! * US-ASCII 字符原样表示；`&` 写成 `&-`。
//! * `&` 到 `-` 之间是**移位段**，用 **modified BASE64** 编码 UTF-16BE：
//!   字母表与标准 BASE64 相同，但 `/` 写成 ``，` 且**不带填充**。
//! * 解出来的字节按 UTF-16BE 解释（代理对由 `String::from_utf16` 处理）。
//!
//! 注意：邮件**头**的 charset（GBK/GB18030 等）是另一回事，
//! 属于 `xremail-message` 的职责（见 [SECURITY](../../../docs/SECURITY.md)）。

use thiserror::Error;

/// UTF-7 编解码错误。
#[derive(Debug, Error, PartialEq, Eq)]
#[non_exhaustive]
pub enum Utf7Error {
    /// 移位段没有结束符 `-`。
    #[error("unterminated shifted sequence")]
    Unterminated,

    /// 移位段里有非法字符。
    #[error("invalid base64 character: {0:?}")]
    InvalidBase64(char),

    /// 移位段解码后长度不是偶数（UTF-16BE 的字节数必须为偶数）。
    #[error("shifted sequence has odd byte length")]
    OddLength,

    /// UTF-16 解码失败（如孤立的代理项）。
    #[error("invalid utf-16: {0}")]
    InvalidUtf16(String),
}

/// 解码 modified UTF-7。
///
/// 非法输入返回 [`Utf7Error`]，**不**静默替换成 `U+FFFD`——
/// 文件夹名解码错了会导致存进错误的文件夹，比报错更糟。
pub fn decode(input: &str) -> Result<String, Utf7Error> {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;

    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        rest = &rest[amp..];

        // 找到移位段的结束符。
        let end = rest[1..]
            .find('-')
            .map(|idx| idx + 1)
            .ok_or(Utf7Error::Unterminated)?;
        let segment = &rest[1..end];

        if segment.is_empty() {
            // "&-" 表示字面的 '&'。
            out.push('&');
        } else {
            let bytes = modified_b64_decode(segment)?;
            if bytes.len() % 2 != 0 {
                return Err(Utf7Error::OddLength);
            }
            let units: Vec<u16> = bytes
                .as_chunks::<2>()
                .0
                .iter()
                .map(|pair| u16::from_be_bytes([pair[0], pair[1]]))
                .collect();
            out.push_str(
                &String::from_utf16(&units).map_err(|e| Utf7Error::InvalidUtf16(e.to_string()))?,
            );
        }

        rest = &rest[end + 1..];
    }

    out.push_str(rest);
    Ok(out)
}

/// 编码为 modified UTF-7。
///
/// 连续的**非**可打印 ASCII 段合并为一个移位段。
pub fn encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut pending: Vec<u16> = Vec::new();

    let flush = |pending: &mut Vec<u16>, out: &mut String| {
        if pending.is_empty() {
            return;
        }
        let mut bytes = Vec::with_capacity(pending.len() * 2);
        for unit in pending.iter() {
            bytes.extend_from_slice(&unit.to_be_bytes());
        }
        out.push('&');
        out.push_str(&modified_b64_encode(&bytes));
        out.push('-');
        pending.clear();
    };

    for ch in input.chars() {
        if ch == '&' {
            // '&' 本身必须写成 "&-"（移位段为空），而不是当作普通字符进移位段。
            flush(&mut pending, &mut out);
            out.push_str("&-");
        } else if is_ascii_printable(ch) {
            flush(&mut pending, &mut out);
            out.push(ch);
        } else {
            // 代理对要按 UTF-16 码元处理，不能逐 char 转 u16。
            let mut buf = [0u16; 2];
            for unit in ch.encode_utf16(&mut buf) {
                pending.push(*unit);
            }
        }
    }
    flush(&mut pending, &mut out);

    out
}

/// 是否是可打印 ASCII（RFC 3501 允许的移位触发字符集）。
///
/// RFC 3501 §5.1.3：所有可打印 US-ASCII 字符（0x20–0x7E）都可直接表示，
/// `&` 除外。
fn is_ascii_printable(ch: char) -> bool {
    (' '..='~').contains(&ch)
}

/// modified BASE64 解码（`/` 写作 `,`，无填充）。
fn modified_b64_decode(input: &str) -> Result<Vec<u8>, Utf7Error> {
    let mut out = Vec::with_capacity(input.len() * 3 / 4);
    let mut buffer: u32 = 0;
    let mut bits: u32 = 0;

    for ch in input.chars() {
        let value = match ch {
            'A'..='Z' => ch as u32 - 'A' as u32,
            'a'..='z' => ch as u32 - 'a' as u32 + 26,
            '0'..='9' => ch as u32 - '0' as u32 + 52,
            '+' => 62,
            ',' => 63,
            other => return Err(Utf7Error::InvalidBase64(other)),
        };

        buffer = (buffer << 6) | value;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((buffer >> bits) & 0xff) as u8);
        }
    }

    // 残留位必须是 0（规范编码）。
    if bits >= 6 || (bits > 0 && (buffer & ((1 << bits) - 1)) != 0) {
        return Err(Utf7Error::InvalidBase64('?'));
    }

    Ok(out)
}

/// modified BASE64 编码（`/` 写作 `,`，无填充）。
fn modified_b64_encode(input: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+,";

    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    let mut buffer: u32 = 0;
    let mut bits: u32 = 0;

    for &byte in input {
        buffer = (buffer << 8) | u32::from(byte);
        bits += 8;
        while bits >= 6 {
            bits -= 6;
            out.push(ALPHABET[((buffer >> bits) & 0x3f) as usize] as char);
        }
    }

    if bits > 0 {
        // 补足到 6 位的倍数（无填充）。
        out.push(ALPHABET[((buffer << (6 - bits)) & 0x3f) as usize] as char);
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_ascii_passes_through() {
        assert_eq!(decode("INBOX").unwrap(), "INBOX");
        assert_eq!(encode("INBOX"), "INBOX");
    }

    #[test]
    fn ampersand_is_escaped() {
        assert_eq!(decode("&-").unwrap(), "&");
        assert_eq!(encode("&"), "&-");
    }

    #[test]
    fn decodes_163_observed_names() {
        // 这三个是 163 实测在线路上看到的文件夹名（脱敏记录在 PROTOCOLS.md）。
        assert_eq!(decode("&XfJT0ZAB-").unwrap(), "已发送");
        assert_eq!(decode("&XfJSIJZk-").unwrap(), "已删除");
    }

    #[test]
    fn round_trips_chinese_names() {
        for name in ["已发送", "草稿箱", "垃圾邮件", "已删除", "所有邮件"] {
            let encoded = encode(name);
            assert!(
                encoded.starts_with('&') && encoded.ends_with('-'),
                "{name} 编码结果应形如 &...-，实际是 {encoded}"
            );
            assert_eq!(decode(&encoded).unwrap(), name);
        }
    }

    #[test]
    fn round_trips_mixed_names() {
        let name = "INBOX/子目录";
        assert_eq!(decode(&encode(name)).unwrap(), name);
    }

    #[test]
    fn round_trips_emoji_via_surrogate_pair() {
        // 代理对：encode 必须是 UTF-16 码元，不能逐 char。
        let name = "📮";
        let encoded = encode(name);
        assert_eq!(decode(&encoded).unwrap(), name);
    }

    #[test]
    fn unterminated_sequence_is_an_error() {
        assert_eq!(decode("&XfJT0ZAB").unwrap_err(), Utf7Error::Unterminated);
    }

    #[test]
    fn invalid_base64_character_is_an_error() {
        assert!(matches!(
            decode("&XYZ!ZAB-").unwrap_err(),
            Utf7Error::InvalidBase64('!')
        ));
    }

    #[test]
    fn odd_length_is_an_error() {
        // "AQ" 解出 1 个字节 → 奇数长度。
        assert_eq!(decode("&AQ-").unwrap_err(), Utf7Error::OddLength);
    }

    #[test]
    fn empty_string_is_empty() {
        assert_eq!(decode("").unwrap(), "");
        assert_eq!(encode(""), "");
    }

    #[test]
    fn text_around_shifted_segments_is_preserved() {
        assert_eq!(decode("INBOX/&XfJT0ZAB-/x").unwrap(), "INBOX/已发送/x");
    }
}
