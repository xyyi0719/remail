//! 真实邮箱的同步冒烟测试（只读）。
//!
//! 用**真实账号**跑一遍完整链路：连接 → 登录 → 同步 → 落库 → 读正文。
//!
//! ## 严格只输出统计
//!
//! **绝不打印**凭据、邮箱地址、主题、正文、发件人。
//! 只输出：文件夹数、邮件数、字符集分布、解码是否出错。
//!
//! 库落在**临时目录**（`tempfile`），跑完即弃——不碰用户的真实数据库。
//!
//! ## 用法
//!
//! ```text
//! xremail-sync-smoke --provider 163 [--keep]
//! ```

use std::time::Duration;

use xremail_core::model::{Account, AuthKind, ProviderKind, ServerConfig, SyncMode, TlsMode};
use xremail_protocol_imap::credential::{CredentialStore, default_accounts_path};
use xremail_protocol_imap::{ImapConfig, TlsTrust, connect_and_login_with};
use xremail_store::dao::message as message_dao;
use xremail_store::{PoolConfig, migrate, open_pool_in_memory};
use xremail_sync::sync_account;

#[tokio::main]
async fn main() {
    if let Err(message) = run().await {
        eprintln!("失败: {message}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), String> {
    // 让同步层的 per-folder 警告可见（它们默认被吞掉，只留日志）。
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("warn")),
        )
        .init();
    let mut provider = "163".to_string();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--provider" | "-p" => provider = args.next().ok_or("--provider 需要一个值")?,
            "--help" | "-h" => {
                println!("用法：xremail-sync-smoke --provider <段名>");
                return Ok(());
            }
            other => return Err(format!("未知参数: {other}")),
        }
    }

    let store = CredentialStore::read_from_file(&default_accounts_path())
        .map_err(|e| format!("读凭据失败: {e}"))?;
    let account_credential = store
        .get(&provider)
        .map_err(|e| format!("取 {provider} 的凭据失败: {e}"))?;
    let password = account_credential
        .password
        .as_str()
        .ok_or("授权码不是合法的 UTF-8 文本")?
        .to_string();

    let host = match provider.as_str() {
        "qq" => "imap.qq.com",
        "wo" => "imap.wo.cn",
        _ => "imap.163.com",
    };

    // 内存库：不碰用户的真实数据库。
    let pool = open_pool_in_memory(&PoolConfig::default())
        .await
        .map_err(|e| format!("开库失败: {e}"))?;
    migrate(&pool).await.map_err(|e| format!("迁移失败: {e}"))?;

    let account = Account {
        id: "smoke".to_string(),
        email: account_credential.user.clone(),
        provider: ProviderKind::Generic,
        display_name: None,
        color: None,
        auth_kind: AuthKind::Password,
        server: ServerConfig {
            imap_host: host.to_string(),
            imap_port: 993,
            imap_tls: TlsMode::Implicit,
            smtp_host: host.to_string(),
            smtp_port: 465,
            smtp_tls: TlsMode::Implicit,
            smtp_auth: xremail_core::model::SmtpAuth::Login,
        },
        username: account_credential.user.clone(),
        secret_ref: None,
        oauth_client_id: None,
        sync_mode: SyncMode::Poll,
        poll_interval_secs: 300,
        signature_html: None,
        created_at: 0,
        updated_at: 0,
    };

    let mut config = ImapConfig::imaps(host);
    config.port = 993;
    config.connect_timeout = Duration::from_secs(15);
    config.command_timeout = Duration::from_secs(60);

    // 账户必须先在库里（`folder.account_id` 有外键指着它）。
    xremail_store::dao::account::upsert_account(&pool, &account)
        .await
        .map_err(|e| format!("插入账户失败: {e}"))?;

    println!("== 同步冒烟（provider = {provider}）==");
    let (mut session, _caps, tls) = connect_and_login_with(
        &config,
        &TlsTrust::SystemRoots,
        &account.username,
        &password,
    )
    .await
    .map_err(|e| format!("连接或登录失败: {e}"))?;

    println!("TLS: {} / {}", tls.protocol, tls.cipher);

    let summary = sync_account(&pool, &mut session, &account, 0)
        .await
        .map_err(|e| format!("同步失败: {e}"))?;

    println!(
        "文件夹 {} 个，新拉 {} 封，服务器已删 {} 封",
        summary.folders, summary.added, summary.removed
    );

    // 统计：多少封有主题、多少封有发件人、字符集分布。
    let mut with_subject = 0usize;
    let mut with_sender = 0usize;
    let mut total = 0usize;
    let mut charsets: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();

    for report in &summary.reports {
        let messages = message_dao::list_messages(&pool, "smoke", &report.folder_id)
            .await
            .map_err(|e| format!("读邮件失败: {e}"))?;
        for message in &messages {
            total += 1;
            if message.subject.is_some() {
                with_subject += 1;
            }
            if !message.from.is_empty() {
                with_sender += 1;
            }
        }
    }

    // 正文：按需拉一封，验证 charset 解码。
    // 挑一个**有邮件**的文件夹（草稿箱往往是空的）。
    for report in &summary.reports {
        let messages = message_dao::list_messages(&pool, "smoke", &report.folder_id)
            .await
            .map_err(|e| format!("读邮件失败: {e}"))?;
        let Some(message) = messages.first() else {
            continue;
        };
        let ok = xremail_sync::fetch_body(
            &pool,
            &mut session,
            "smoke",
            &report.folder_id,
            &report.remote_name,
            message.uid,
            message.uidvalidity,
        )
        .await
        .map_err(|e| format!("拉正文失败: {e}"))?;

        if ok {
            let body = message_dao::get_message_body(&pool, &message.id)
                .await
                .map_err(|e| format!("读正文失败: {e}"))?;
            if let Some(body) = body {
                *charsets
                    .entry(body.charset.clone().unwrap_or_else(|| "(无)".into()))
                    .or_insert(0) += 1;
                let plain = body
                    .text_plain
                    .as_ref()
                    .map(|t| t.chars().count())
                    .unwrap_or(0);
                let html = body
                    .html_raw
                    .as_ref()
                    .map(|t| t.chars().count())
                    .unwrap_or(0);
                let replacement = body
                    .text_plain
                    .as_ref()
                    .map(|t| t.contains('\u{FFFD}'))
                    .unwrap_or(false)
                    || body
                        .html_raw
                        .as_ref()
                        .map(|t| t.contains('\u{FFFD}'))
                        .unwrap_or(false);
                println!(
                    "正文抽样：字符集 {}，纯文本 {} 字，HTML {} 字，含替换字符 {}",
                    body.charset.as_deref().unwrap_or("(无)"),
                    plain,
                    html,
                    replacement
                );
            }
        }
        break;
    }

    println!("邮件共 {total} 封，有主题 {with_subject} 封，有发件人 {with_sender} 封");
    if !charsets.is_empty() {
        let list: Vec<String> = charsets
            .iter()
            .map(|(name, count)| format!("{name}×{count}"))
            .collect();
        println!("正文字符集：{}", list.join("、"));
    }

    Ok(())
}
