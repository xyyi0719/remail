//! 真实邮箱往返测试（**默认不跑**）。
//!
//! 用两个真实测试账户互发一封真邮件，验证发信链路在**真实服务器**上成立：
//!
//! 1. 真实 SMTP 提交能走通（`MAIL FROM` → `RCPT TO` → `DATA` → `QUIT`）；
//! 2. 对方**真的收到**（IMAP 拉取 + 正文里有标记）；
//! 3. 发件方自己的「已发送」里**正好有一份**这封信。
//!
//! 第 3 条是**自适应**副本策略的验收：163 的服务器会自己存一份，
//! QQ 不会——所以程序会先查、查不到才自己 `APPEND`。
//! 结果必须是**一份**：两份就是补重了（163 的坑），零份就是没落地。
//!
//! ## 为什么默认不跑
//!
//! 需要真实凭据（`%USERPROFILE%\.xremail\accounts.txt`，**在仓库之外**），
//! 而且会**真的发出邮件**。所以：
//!
//! * 用 `#[ignore]` 标记（CI 的 `cargo test` 不会执行它）；
//! * 还要设 `XREMAIL_REAL_ACCOUNTS=1` 才真正动手。
//!
//! 真实凭据测试不得进入普通 PR CI（见 [TESTING_STRATEGY §5.2](../../../../docs/TESTING_STRATEGY.md)）。
//!
//! ## 怎么跑
//!
//! ```powershell
//! $env:XREMAIL_REAL_ACCOUNTS = "1"
//! cargo test -p xremail-shell --test real_mail_roundtrip -- --ignored --nocapture
//! ```
//!
//! ## 用哪两个账户
//!
//! 从凭据文件里读 `163` 与 `qq` 两段。发信地址由用户名校准：
//! QQ 的用户名常常只是一串数字（`2219705565`），要补成 `2219705565@qq.com`
//! ——**用哪个地址登录，用哪个地址发信**，否则服务器会拒收。

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use xremail_api::{NewAccountRequest, SecretValue};
use xremail_core::model::{ProviderKind, SmtpAuth, SyncMode, TlsMode};
use xremail_protocol_imap::command::{CommandOutcome, examine_checked, run_checked};
use xremail_protocol_imap::connection::{ImapConfig, connect_and_login};
use xremail_protocol_imap::folders::list_folders;
use xremail_shell_lib::service::account::AccountService;
use xremail_shell_lib::service::compose::{ComposeService, DraftInput, SendResultState};

/// 一个邮箱账户：地址、授权码、服务器地址。
#[derive(Clone)]
struct Mailbox {
    /// 服务商段名（`163` / `qq`）。
    section: String,
    /// 完整地址（发信与登录都用它）。
    email: String,
    /// 客户端授权码。
    password: String,
    imap_host: String,
    smtp_host: String,
    provider: ProviderKind,
}

/// 最多等多久（投递可能要几秒）。
const DELIVERY_TIMEOUT: Duration = Duration::from_secs(120);
/// 轮询间隔。
const POLL_INTERVAL: Duration = Duration::from_secs(6);

#[tokio::test]
#[ignore = "需要真实邮箱凭据与网络（会真的发邮件）；用 --ignored 显式运行"]
async fn real_round_trip_between_two_mailboxes() {
    if std::env::var("XREMAIL_REAL_ACCOUNTS").as_deref() != Ok("1") {
        eprintln!("跳过：需要 XREMAIL_REAL_ACCOUNTS=1 才允许动真邮箱");
        return;
    }

    let accounts = load_accounts();
    let sender = require_mailbox(&accounts, "163");
    let receiver = require_mailbox(&accounts, "qq");

    println!("# 真实发信往返测试");
    println!(
        "发件方：{}（{}，段名 {}）",
        sender.email, sender.smtp_host, sender.section
    );
    println!(
        "收件方：{}（{}，段名 {}）",
        receiver.email, receiver.imap_host, receiver.section
    );

    // 方向可选（`XREMAIL_DIRECTION=1|2|both`，默认 both）——
    // 反复跑同一方向会往对方收件箱里堆测试信，没必要时别跑。
    let direction = std::env::var("XREMAIL_DIRECTION").unwrap_or_else(|_| "both".to_string());

    if direction == "1" || direction == "both" {
        run_direction(&sender, &receiver, "方向一：163 → QQ").await;
    }
    if direction == "2" || direction == "both" {
        run_direction(&receiver, &sender, "方向二：QQ → 163").await;
    }

    println!("\n跑完。");
}

/// 跑一个方向：提交 → 等投递 → 查已发送副本。
async fn run_direction(sender: &Mailbox, receiver: &Mailbox, title: &str) {
    println!("\n## {title}");
    let marker = match send_one(sender, receiver).await {
        Ok(marker) => {
            println!("提交：成功，标记 {marker}");
            marker
        }
        Err(error) => panic!("{title} 提交失败：{error}"),
    };

    match wait_for_delivery(receiver, &marker).await {
        Some(summary) => {
            println!(
                "投递：收件方**收到**了（文件夹 {}，UID {:?}，{} 字节）",
                summary.folder, summary.uid, summary.size
            );
            println!("投递摘要：{}", summary.subject);
            println!("正文含标记：{}", summary.body_has_marker);
            assert!(
                summary.body_has_marker,
                "收到的信里没有标记 —— 不是我们发的那封"
            );
        }
        None => panic!("{title} 提交成功但收件方 120 秒内没收到 —— 链路有问题"),
    }

    match wait_for_sent_copies(sender, &marker).await {
        Some(1) => println!("已发送副本：**正好一份**（服务器自己存的，或程序补的）"),
        Some(0) => panic!("{title} 发完「已发送」里没有这封信 —— 副本没落地"),
        Some(count) => panic!(
            "{title} 发完「已发送」里有 {count} 份同样的信 —— 补重了（服务器自己存了一份，程序又补了一份？）"
        ),
        None => println!("已发送副本：查不到（文件夹打不开，这一项没法验）"),
    }
}

/// 轮询发件方的「已发送」，等副本出现（最多等两分钟）。
///
/// ## 为什么要等
///
/// **163 的副本几乎立刻就有，QQ 的要等几十秒**（实测 2026-10-30：
/// QQ 是 SMTP 侧异步落盘，提交后 30–90 秒才在 IMAP 里看得到）。
/// 查一次就下结论会把「还没到」误判成「没有」。
async fn wait_for_sent_copies(sender: &Mailbox, marker: &str) -> Option<usize> {
    let deadline = std::time::Instant::now() + Duration::from_secs(120);
    let mut last = None;
    while std::time::Instant::now() < deadline {
        match sent_folder_copies(sender, marker).await {
            Some(count) => {
                last = Some(count);
                if count > 0 {
                    return Some(count);
                }
            }
            None => return None,
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
    last
}

/// 一封发到对端的邮件摘要（含**在哪个文件夹**找到的）。
struct Delivery {
    /// 落在哪个文件夹（`INBOX` / `垃圾邮件` / …）。
    folder: String,
    uid: Option<u32>,
    size: usize,
    subject: String,
    body_has_marker: bool,
}

/// 发一封真邮件。返回正文里的标记（成功时）。
async fn send_one(sender: &Mailbox, receiver: &Mailbox) -> Result<String, String> {
    let marker = format!(
        "xremail-roundtrip-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or_default()
    );

    let pool = xremail_store::open_pool_in_memory(&xremail_store::PoolConfig::default())
        .await
        .map_err(|e| format!("打开内存库失败：{e}"))?;
    xremail_store::migrate(&pool)
        .await
        .map_err(|e| format!("应用迁移失败：{e}"))?;

    let accounts = AccountService::new(pool.clone());
    let account = accounts
        .create(NewAccountRequest {
            email: sender.email.clone(),
            provider: sender.provider,
            display_name: None,
            imap_host: sender.imap_host.clone(),
            imap_port: 993,
            imap_tls: TlsMode::Implicit,
            smtp_host: sender.smtp_host.clone(),
            smtp_port: 465,
            smtp_tls: TlsMode::Implicit,
            smtp_auth: SmtpAuth::Login,
            username: sender.email.clone(),
            sync_mode: SyncMode::Poll,
            poll_interval_secs: 300,
        })
        .await
        .map_err(|e| format!("建账户失败：{e}"))?;

    let service = ComposeService::new(pool.clone(), accounts);
    let draft = DraftInput {
        account_id: account.id.clone(),
        id: None,
        to: vec![receiver.email.clone()],
        cc: Vec::new(),
        bcc: Vec::new(),
        subject: format!("[xremail] 往返测试 {marker}"),
        body_text: format!(
            "这是一封由 xremail 真实发送的往返测试邮件。\r\n\r\n标记：{marker}\r\n发件方：{}\r\n",
            sender.email
        ),
        body_html: None,
        attachments: Vec::new(),
    };

    let credential = SecretValue::from_str_lossless(&sender.password);
    let result = service
        .send(&draft, &credential)
        .await
        .map_err(|e| format!("服务层返回错误：{e}"))?;

    println!(
        "发送结果：state={:?} message_id={} detail={}",
        result.state, result.message_id, result.detail
    );
    match result.state {
        SendResultState::Sent => Ok(marker),
        SendResultState::Uncertain => Err(format!(
            "**不确定**（命令发出去了但没拿到最终应答）：{} —— 不自动重发，先查收件箱",
            result.detail
        )),
        SendResultState::Failed => Err(format!("明确失败：{}", result.detail)),
    }
}

/// 轮询收件方，直到找到带标记的那封信。
///
/// ## 为什么搜所有文件夹
///
/// 只搜 `INBOX` 会漏掉一件事：**163 把来自 QQ 的信放进了垃圾箱**。
/// 服务商之间的垃圾过滤策略各不相同，
/// 「信没到」和「信到了但不在收件箱」是两回事——必须都看。
async fn wait_for_delivery(receiver: &Mailbox, marker: &str) -> Option<Delivery> {
    let deadline = std::time::Instant::now() + DELIVERY_TIMEOUT;
    while std::time::Instant::now() < deadline {
        if let Some(found) = fetch_by_subject(receiver, marker).await {
            return Some(found);
        }
        tokio::time::sleep(POLL_INTERVAL).await;
    }
    None
}

/// 在**所有**文件夹里找那封信，拉回来核对正文。
///
/// ## 为什么不用 `SEARCH`
///
/// **163 的 IMAP 对 `SEARCH` 的任何非 `ALL` 条件都静默返回空**
/// （`SUBJECT` / `HEADER` / `TEXT` 全部如此，**连 `NO` 都不报错**）。
/// 实测于 2026-10-29，QQ 上同样的命令正常。
///
/// 所以这里改成和同步引擎一样的做法：`UID FETCH 1:* (UID BODY.PEEK[HEADER])`
/// 拉回所有头，自己找带标记的那封。
/// 依赖服务端 SEARCH 的功能在 163 上会**静默失效**——
/// 这类“看起来成功但结果不对”的坑比报错难查得多（见 ISSUES_AND_RISKS R22）。
async fn fetch_by_subject(mailbox: &Mailbox, marker: &str) -> Option<Delivery> {
    let config = ImapConfig::imaps(mailbox.imap_host.clone());
    let (mut session, _caps, _tls) = connect_and_login(&config, &mailbox.email, &mailbox.password)
        .await
        .ok()?;

    let folders = list_folders(&mut session).await.ok()?;
    // INBOX 先找，其余文件夹随后。
    // **用 raw_name 开文件夹**：163 的名字在线路上是 modified UTF-7
    // （`&XfJT0ZAB-` = 已发送），拿解码后的名字去 EXAMINE 会得到
    // `NO EXAMINE Folder not exist`（实测 2026-10-29）。
    let mut targets: Vec<(String, String)> = vec![("INBOX".to_string(), "INBOX".to_string())];
    for folder in &folders {
        if folder.is_inbox() || targets.iter().any(|(name, _)| *name == folder.name) {
            continue;
        }
        targets.push((folder.name.clone(), folder.raw_name.clone()));
    }

    for (name, raw_name) in targets {
        // 某个文件夹开不了（`NoSelect`、名字对不上……）**不能**中断整个查找：
        // 信可能在下一个文件夹里。开不了就跳过这个。
        if examine_checked(&mut session, &raw_name).await.is_err() {
            continue;
        }

        // 一次拉回所有头，自己找带标记的那封。
        let Ok(outcome) = run_checked(
            &mut session,
            "UID FETCH 1:* (UID FLAGS RFC822.SIZE BODY.PEEK[HEADER])",
        )
        .await
        else {
            continue;
        };
        let CommandOutcome::Fetches(messages) = outcome else {
            continue;
        };
        let Some(hit) = messages.iter().rev().find(|message| {
            message
                .header
                .as_ref()
                .is_some_and(|header| String::from_utf8_lossy(header).contains(marker))
        }) else {
            continue;
        };
        let Some(uid) = hit.uid else { continue };

        // 找到头匹配的，再取全文核对正文里的标记。
        let Ok(outcome) =
            run_checked(&mut session, &format!("UID FETCH {uid} (BODY.PEEK[])")).await
        else {
            continue;
        };
        let CommandOutcome::Fetches(full) = outcome else {
            continue;
        };
        let Some(message) = full.first() else {
            continue;
        };

        let mut raw = Vec::new();
        if let Some(header) = &message.header {
            raw.extend_from_slice(header);
        }
        if let Some(body) = &message.body {
            raw.extend_from_slice(body);
        }
        if let Some(text) = &message.text {
            raw.extend_from_slice(text);
        }
        let text = String::from_utf8_lossy(&raw);

        return Some(Delivery {
            folder: name,
            uid: message.uid.or(Some(uid)),
            size: raw.len(),
            subject: decode_subject(&text),
            body_has_marker: text.contains(marker),
        });
    }
    None
}

/// 从原始邮件里粗略取出主题（够打印用）。
fn decode_subject(raw: &str) -> String {
    for line in raw.lines() {
        if let Some(rest) = line.strip_prefix("Subject: ") {
            // 编码字（RFC 2047）不解析，原样显示——这里只是给人看。
            return rest.trim().to_string();
        }
    }
    "<没找到 Subject>".to_string()
}

/// 「已发送」文件夹里**有几份**这封信。
///
/// 返回 `None` = 文件夹打不开（这一项没法验）。
/// 返回 `Some(n)`：n = 带这个标记的信的份数。
///
/// **必须是 1**：
///
/// * 服务器自己存（163）→ 程序不该再补 → 1；
/// * 服务器不存（QQ）→ 程序得补 → 1；
/// * 2 = 补重了。
async fn sent_folder_copies(mailbox: &Mailbox, marker: &str) -> Option<usize> {
    let config = ImapConfig::imaps(mailbox.imap_host.clone());
    let (mut session, _caps, _tls) = connect_and_login(&config, &mailbox.email, &mailbox.password)
        .await
        .ok()?;
    let folders = list_folders(&mut session).await.ok()?;

    let sent = folders
        .iter()
        .find(|folder| folder.role.as_deref() == Some("sent"))
        .or_else(|| {
            folders
                .iter()
                .find(|folder| folder.name.contains("Sent") || folder.name.contains("已发送"))
        });
    let Some(sent) = sent else {
        let names: Vec<&str> = folders.iter().map(|f| f.name.as_str()).collect();
        println!("（该账户的文件夹：{}）", names.join(" / "));
        return None;
    };

    // **必须用 raw_name**（线路上的原样）：163 的文件夹名在线路上是
    // modified UTF-7（`&XfJT0ZAB-` = 已发送），拿解码后的名字去 EXAMINE
    // 会得到 `NO EXAMINE Folder not exist`（实测 2026-10-29）。
    let _info = examine_checked(&mut session, &sent.raw_name).await.ok()?;
    let outcome = run_checked(
        &mut session,
        "UID FETCH 1:* (UID FLAGS RFC822.SIZE BODY.PEEK[HEADER])",
    )
    .await
    .ok()?;
    let CommandOutcome::Fetches(messages) = outcome else {
        return Some(0);
    };
    Some(
        messages
            .iter()
            .filter(|message| {
                message
                    .header
                    .as_ref()
                    .is_some_and(|header| String::from_utf8_lossy(header).contains(marker))
            })
            .count(),
    )
}

/// 凭据文件路径（**在仓库之外**）。
fn accounts_file_path() -> PathBuf {
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join(".xremail").join("accounts.txt")
}

/// 读凭据文件：段名 → {user, password}。
fn load_accounts() -> BTreeMap<String, (String, String)> {
    let path = accounts_file_path();
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("读不到凭据文件 {}：{e}", path.display()));
    let mut out: BTreeMap<String, (String, String)> = BTreeMap::new();
    let mut section = String::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            section = name.trim().to_string();
            out.entry(section.clone()).or_default();
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim().to_string();
        let entry = out.entry(section.clone()).or_default();
        match key.trim() {
            "user" => entry.0 = value,
            "password" => entry.1 = value,
            _ => {}
        }
    }
    out
}

/// 取某个服务商的账户，地址按服务商补全域名。
fn require_mailbox(accounts: &BTreeMap<String, (String, String)>, section: &str) -> Mailbox {
    let (user, password) = accounts
        .get(section)
        .unwrap_or_else(|| panic!("凭据文件里没有 [{section}] 段"));
    assert!(!user.is_empty(), "[{section}] 段的 user 是空的");
    assert!(!password.is_empty(), "[{section}] 段的 password 是空的");

    let (imap_host, smtp_host, provider, domain) = match section {
        "163" => (
            "imap.163.com",
            "smtp.163.com",
            ProviderKind::Netease163,
            "@163.com",
        ),
        "qq" => (
            "imap.qq.com",
            "smtp.qq.com",
            ProviderKind::Tencent,
            "@qq.com",
        ),
        other => panic!("没配 {other} 的服务器参数"),
    };

    // QQ 的用户名常常只是一串数字，要补成完整地址。
    let email = if user.contains('@') {
        user.clone()
    } else {
        format!("{user}{domain}")
    };

    Mailbox {
        section: section.to_string(),
        email,
        password: password.clone(),
        imap_host: imap_host.to_string(),
        smtp_host: smtp_host.to_string(),
        provider,
    }
}
