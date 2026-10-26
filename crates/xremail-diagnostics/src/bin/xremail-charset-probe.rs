//! 字符集抽样工具（只读）。
//!
//! ## 目的
//!
//! 在写 MIME/charset 解码器**之前**，先搞清楚真实邮箱里到底用的是什么编码。
//! 不靠猜——三家服务商的实现可能完全不同。
//!
//! ## 严格只输出编码名
//!
//! 本工具**只**打印：
//! * `Content-Type` 里的 `charset` 参数；
//! * `Content-Transfer-Encoding` 的值；
//! * `Subject` 是否用 RFC 2047 编码字（`=?charset?B?...?=`）及其 charset。
//!
//! **绝不**打印主题、正文、发件人、地址或任何邮件内容。
//!
//! ## 用法
//!
//! ```text
//! xremail-charset-probe --provider 163 [--provider qq] [--sample 20]
//! ```

use std::collections::BTreeMap;
use std::time::Duration;

use xremail_message::mime::{HeaderMap, split_head_body, split_parts};
use xremail_message::parse_message;
use xremail_protocol_imap::credential::{CredentialStore, default_accounts_path};
use xremail_protocol_imap::{CommandOutcome, ImapConfig, TlsTrust, connect_and_login_with};

/// 抽样数量上限。
const DEFAULT_SAMPLE: usize = 20;

#[tokio::main]
async fn main() {
    if let Err(message) = run().await {
        eprintln!("失败: {message}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), String> {
    let mut providers: Vec<String> = Vec::new();
    let mut sample = DEFAULT_SAMPLE;
    let mut host_override: Option<String> = None;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--provider" | "-p" => {
                let value = args.next().ok_or("--provider 需要一个值")?;
                providers.push(value);
            }
            "--sample" | "-n" => {
                let value = args.next().ok_or("--sample 需要一个值")?;
                sample = value
                    .parse()
                    .map_err(|_| format!("--sample 需要一个数字，收到 {value}"))?;
            }
            "--host" | "-H" => {
                host_override = Some(args.next().ok_or("--host 需要一个值")?);
            }
            "--help" | "-h" => {
                println!(
                    "用法：xremail-charset-probe --provider <段名> [--provider ...] [--sample N] [--host H]"
                );
                return Ok(());
            }
            other => return Err(format!("未知参数: {other}")),
        }
    }

    if providers.is_empty() {
        providers.push("163".to_string());
    }

    let store = CredentialStore::read_from_file(&default_accounts_path())
        .map_err(|e| format!("读凭据失败: {e}"))?;

    for provider in &providers {
        let account = store
            .get(provider)
            .map_err(|e| format!("取 {provider} 的凭据失败: {e}"))?;
        let host = host_override
            .clone()
            .unwrap_or_else(|| default_host(provider).to_string());

        println!("======== {provider}（{host}）========");
        match sample_one(&host, account, sample).await {
            Ok(report) => print_report(&report),
            Err(message) => println!("  采样失败: {message}"),
        }
        println!();
    }

    Ok(())
}

/// 各服务商的默认 IMAP 主机。
fn default_host(provider: &str) -> &'static str {
    match provider {
        "qq" => "imap.qq.com",
        "wo" => "imap.wo.cn",
        "126" => "imap.126.com",
        _ => "imap.163.com",
    }
}

/// 一封抽样邮件的编码信息。
#[derive(Debug, Default)]
struct HeaderSample {
    /// 顶层 `Content-Type` 的 charset。
    charset: Option<String>,
    /// 顶层 `Content-Transfer-Encoding`。
    transfer_encoding: Option<String>,
    /// `Subject` 用的是 RFC 2047 编码字。
    subject_encoded: bool,
    /// `Subject` 编码字里的 charset。
    subject_charset: Option<String>,
    /// 顶层是不是 multipart。
    multipart: bool,
}

/// 一轮采样的统计。
#[derive(Debug, Default)]
struct SampleReport {
    /// 抽样了多少封。
    sampled: usize,
    /// 成功拿到整封的封数。
    fetched: usize,
    /// **正文部分**的 charset → 出现次数。
    body_charsets: BTreeMap<String, usize>,
    /// 正文部分没有 charset 的封数。
    body_without_charset: usize,
    /// 顶层 `Content-Type` 的 charset → 次数。
    top_charsets: BTreeMap<String, usize>,
    /// transfer encoding → 出现次数。
    transfer_encodings: BTreeMap<String, usize>,
    /// 用 RFC 2047 编码主题的封数。
    subject_encoded: usize,
    /// 主题编码字的 charset → 次数。
    subject_charsets: BTreeMap<String, usize>,
    /// 顶层是 multipart 的封数。
    multipart: usize,
    /// **解码出错**（出现 U+FFFD）的封数。
    decode_errors: usize,
}

/// 打印抽样结果（只有编码名与计数）。
fn print_report(report: &SampleReport) {
    println!("  抽样封数        : {}", report.sampled);
    println!("  拿到整封的封数  : {}", report.fetched);
    println!(
        "  ★正文 charset  : {}",
        format_counts(&report.body_charsets)
    );
    println!("  正文无 charset  : {}", report.body_without_charset);
    println!(
        "  顶层 charset    : {}",
        format_counts(&report.top_charsets)
    );
    println!(
        "  传输编码        : {}",
        format_counts(&report.transfer_encodings)
    );
    println!("  顶层 multipart  : {}", report.multipart);
    println!("  ★解码出错封数   : {}", report.decode_errors);
    println!("  Subject 用 2047 : {}", report.subject_encoded);
    println!(
        "  Subject charset : {}",
        format_counts(&report.subject_charsets)
    );
}

/// 把计数表格式化成一行。
fn format_counts(counts: &BTreeMap<String, usize>) -> String {
    if counts.is_empty() {
        return "（无）".to_string();
    }
    counts
        .iter()
        .map(|(name, count)| format!("{name}×{count}"))
        .collect::<Vec<String>>()
        .join("、")
}

/// 采一个服务商。
async fn sample_one(
    host: &str,
    account: &xremail_protocol_imap::AccountCredential,
    sample: usize,
) -> Result<SampleReport, String> {
    let mut config = ImapConfig::imaps(host);
    config.connect_timeout = Duration::from_secs(10);
    config.command_timeout = Duration::from_secs(30);

    let (mut session, _caps, _tls) = connect_and_login_with(
        &config,
        &TlsTrust::SystemRoots,
        &account.user,
        account
            .password
            .as_str()
            .ok_or("授权码不是合法的 UTF-8 文本")?,
    )
    .await
    .map_err(|e| format!("登录失败: {e}"))?;

    session
        .examine("INBOX")
        .await
        .map_err(|e| format!("EXAMINE 失败: {e}"))?;

    // 取 UID 列表。
    let outcome = xremail_protocol_imap::run_checked(&mut session, "UID SEARCH ALL")
        .await
        .map_err(|e| format!("UID SEARCH 失败: {e}"))?;
    let mut uids: Vec<u32> = match outcome {
        CommandOutcome::Search(ids) => ids,
        _ => Vec::new(),
    };

    if uids.is_empty() {
        return Ok(SampleReport::default());
    }

    // 取最近的几封（UID 大的通常是新的）。
    uids.sort_unstable();
    let picked: Vec<u32> = uids
        .iter()
        .rev()
        .take(sample)
        .copied()
        .collect::<Vec<u32>>()
        .into_iter()
        .rev()
        .collect();

    // 拉**整封**（PEEK：不置 \Seen）。
    //
    // 必须拉整封——正文部分的 `Content-Type` 在**部件的头**里，
    // 只拉顶层头是看不到的。
    let set = picked
        .iter()
        .map(u32::to_string)
        .collect::<Vec<String>>()
        .join(",");
    let command = format!("UID FETCH {set} (UID BODY.PEEK[])");
    let outcome = xremail_protocol_imap::run_checked(&mut session, &command)
        .await
        .map_err(|e| format!("UID FETCH 失败: {e}"))?;

    let mut report = SampleReport {
        sampled: picked.len(),
        ..Default::default()
    };

    let CommandOutcome::Fetches(messages) = outcome else {
        return Err("服务器没有返回 FETCH 结果".into());
    };

    for message in messages {
        let Some(raw) = message.body else {
            continue;
        };
        report.fetched += 1;

        let (headers, body) = split_head_body(&raw);
        let sample = parse_header_sample(&headers);

        report.multipart += usize::from(sample.multipart);
        if let Some(charset) = sample.charset {
            *report.top_charsets.entry(charset).or_insert(0) += 1;
        }
        if let Some(encoding) = sample.transfer_encoding {
            *report.transfer_encodings.entry(encoding).or_insert(0) += 1;
        }
        if sample.subject_encoded {
            report.subject_encoded += 1;
            if let Some(charset) = sample.subject_charset {
                *report.subject_charsets.entry(charset).or_insert(0) += 1;
            }
        }

        // 用真正的解析器跑一遍：看解码有没有出错。
        if let Ok(parsed) = parse_message(&raw) {
            report.decode_errors += usize::from(parsed.decode_errors);
        } else {
            report.decode_errors += 1;
        }

        // **递归**拆 MIME，看正文部分的 charset。
        let parts = split_parts(&headers, &body);
        let mut body_charset = None;
        for part in &parts {
            if part.content_type.mime_type == "text/plain"
                || part.content_type.mime_type == "text/html"
            {
                body_charset = part.content_type.charset.clone();
                break;
            }
        }
        match body_charset {
            Some(charset) => {
                *report.body_charsets.entry(charset).or_insert(0) += 1;
            }
            None => report.body_without_charset += 1,
        }
    }

    Ok(report)
}

/// 从一封邮件的头里提取编码信息（只看顶层）。
fn parse_header_sample(headers: &HeaderMap) -> HeaderSample {
    let mut sample = HeaderSample::default();

    let content_type = xremail_message::mime::parse_content_type(headers.get("Content-Type"));
    sample.multipart = content_type.is_multipart();
    sample.charset = content_type.charset.clone();

    sample.transfer_encoding = headers
        .get("Content-Transfer-Encoding")
        .map(|value| value.trim().to_ascii_lowercase())
        .filter(|value| !value.is_empty());

    if let Some(subject) = headers.get("Subject")
        && let Some((charset, _)) = find_encoded_word(subject)
    {
        sample.subject_encoded = true;
        sample.subject_charset = Some(charset);
    }

    sample
}

/// 找出 RFC 2047 编码字及其 charset。
///
/// 只取 charset，**不解码内容**。
fn find_encoded_word(line: &str) -> Option<(String, bool)> {
    let start = line.find("=?")?;
    let rest = &line[start + 2..];
    let end = rest.find('?')?;
    let charset = rest[..end].trim().to_ascii_lowercase();
    if charset.is_empty() {
        return None;
    }
    Some((charset, true))
}
