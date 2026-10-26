//! MIME 传输编码（Content-Transfer-Encoding）。
//!
//! 实测（2026-10-23）QQ 的邮件用 `quoted-printable` 与 `8bit`，
//! 沃用 `7bit`。所以这三种都要处理，`base64` 也要（附件与部分正文）。

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;

/// 解码传输编码后的字节。
///
/// 未知编码按 `8bit` 处理（RFC 2045 §6.4：未知编码按 8bit 解读）。
pub fn decode_transfer(bytes: &[u8], encoding: Option<&str>) -> Vec<u8> {
    match encoding
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("base64") => decode_base64(bytes),
        Some("quoted-printable") => decode_quoted_printable(bytes),
        // 7bit / 8bit / binary / 未知：原样。
        _ => bytes.to_vec(),
    }
}

/// 解码 base64。
///
/// **容错**：RFC 2045 §6.8 规定「不在 base64 字母表里的字符一律忽略」。
/// 真实邮件里偶尔有脏数据（被别的软件改过），直接报错会丢掉整封正文。
pub fn decode_base64(bytes: &[u8]) -> Vec<u8> {
    // 只保留 base64 字母表与填充。
    let cleaned: Vec<u8> = bytes
        .iter()
        .copied()
        .filter(|b| b.is_ascii_alphanumeric() || *b == b'+' || *b == b'/' || *b == b'=')
        .collect();

    match STANDARD.decode(&cleaned) {
        Ok(decoded) => decoded,
        Err(_) => {
            // 逐块试：base64 以 4 字符为一组，丢掉不成组��尾巴。
            let mut out = Vec::new();
            let mut chunk = Vec::with_capacity(4);
            for byte in cleaned {
                chunk.push(byte);
                if chunk.len() == 4 {
                    if let Ok(part) = STANDARD.decode(&chunk) {
                        out.extend_from_slice(&part);
                    }
                    chunk.clear();
                }
            }
            out
        }
    }
}

/// 解码 quoted-printable。
///
/// * `=XX` → 一个字节；
/// * 行尾的 `=` → 软换行，去掉；
/// * 其余原样。
pub fn decode_quoted_printable(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;

    while index < bytes.len() {
        let byte = bytes[index];

        if byte == b'=' {
            // 软换行：`=\r\n` 或 `=\n`
            if index + 1 < bytes.len()
                && bytes[index + 1] == b'\r'
                && index + 2 < bytes.len()
                && bytes[index + 2] == b'\n'
            {
                index += 3;
                continue;
            }
            if index + 1 < bytes.len() && bytes[index + 1] == b'\n' {
                index += 2;
                continue;
            }

            // `=XX`
            if index + 2 < bytes.len() {
                let high = (bytes[index + 1] as char).to_digit(16);
                let low = (bytes[index + 2] as char).to_digit(16);
                if let (Some(high), Some(low)) = (high, low) {
                    out.push((high * 16 + low) as u8);
                    index += 3;
                    continue;
                }
            }

            // 不合法的 `=`：原样保留。
            out.push(byte);
            index += 1;
            continue;
        }

        out.push(byte);
        index += 1;
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_round_trips() {
        let raw = b"hello world";
        let encoded = STANDARD.encode(raw);

        assert_eq!(decode_base64(encoded.as_bytes()), raw);
    }

    #[test]
    fn base64_ignores_whitespace() {
        let encoded = "aGVs\r\nbG8g\r\nd29y bGQ=";

        assert_eq!(decode_base64(encoded.as_bytes()), b"hello world");
    }

    #[test]
    fn base64_skips_invalid_characters() {
        // 中间插了一个非法字符 `!`（RFC 2045：非字母表字符一律忽略）。
        let encoded = "aGVs!bG8=";

        assert_eq!(decode_base64(encoded.as_bytes()), b"hello");
    }

    #[test]
    fn quoted_printable_decodes_hex_escapes() {
        // "中文" 的 GB2312 字节被编码成 =D6=D0=CE=C4
        let input = b"=D6=D0=CE=C4";

        assert_eq!(decode_quoted_printable(input), vec![0xD6, 0xD0, 0xCE, 0xC4]);
    }

    #[test]
    fn quoted_printable_removes_soft_breaks() {
        let input = b"long=\r\nline";

        assert_eq!(decode_quoted_printable(input), b"longline");
    }

    #[test]
    fn quoted_printable_keeps_invalid_escapes() {
        let input = b"a=Zb";

        assert_eq!(decode_quoted_printable(input), b"a=Zb");
    }

    #[test]
    fn unknown_encoding_passes_through() {
        let input = b"raw bytes";

        assert_eq!(decode_transfer(input, Some("8bit")), input);
        assert_eq!(decode_transfer(input, None), input);
        assert_eq!(decode_transfer(input, Some("x-nonsense")), input);
    }

    #[test]
    fn transfer_dispatches_to_the_right_decoder() {
        let raw = b"hello";

        assert_eq!(
            decode_transfer(&STANDARD.encode(raw).into_bytes(), Some("base64")),
            raw
        );
        assert_eq!(
            decode_transfer(b"=68=65=6C=6C=6F", Some("quoted-printable")),
            raw
        );
    }
}
