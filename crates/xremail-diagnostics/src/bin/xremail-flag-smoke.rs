//! 真实邮箱的**写标记**冒烟测试（标记可读 / 可写）。
//!
//! 上一版 `xremail-sync-smoke` 是**只读**的：连接 → 登录 → 拉取 → 落库 → 读正文。
//! 这一版补上写的那一半——**`UID STORE` 在真实的国内邮箱服务器上到底成不成立**。
//!
//! ## 为什么单独写一个工具
//!
//! 回写链路的逻辑（意图 → 队列 → 服务器 → **回读确认**）在 crate 层有假服务器
//! 的端到端用例。但假服务器是**我们写的**，它对 `UID STORE` 的回答是我们
//! **假设**服务器会怎么回答。
//!
//! 真实服务器会做一些假服务器不会做的事，比如：
//!
//! * 对一个**已经有** `\Seen` 的邮件再发一次 `+FLAGS (\Seen)`，
//!   很多服务器回的是 `NO [MODIFIED]` 而不是 `OK`；
//! * `STORE` 回 `OK` 但标记**没真的改**（有的服务器在某些情况下会这样）。
//!
//! 这两种行为直接决定「重试 / 记账 / 报成功」的判断，所以必须对着真服务器验。
//!
//! ## 严格只输出统计
//!
//! **绝不打印**凭据、邮箱地址、主题、正文、发件人。只输出：邮件数、标记数、
//! 命令结果分类、是否恢复原状。
//!
//! ## 会改真实邮箱的状态
//!
//! 本工具会**真的**给一封邮件加 `\Seen`，验完**恢复原状**。
//! 它只动**一封**邮件的一个标记，且只动收件箱里的一封。
//!
//! ## 用法
//!
//! ```text
//! xremail-flag-smoke --provider 163 [--folder INBOX] [--keep]
//! ```

use std::time::Duration;

use xremail_protocol_imap::command::run_checked;
use xremail_protocol_imap::credential::{CredentialStore, default_accounts_path};
use xremail_protocol_imap::{
    CommandOutcome, ImapConfig, ProtocolError, TlsTrust, connect_and_login_with,
};

/// 一条命令的结果怎么分类——**分法要和回写链路的一致**。
#[derive(Debug, PartialEq, Eq)]
enum StoreResult {
    /// 服务器回了 `OK`。
    Ok,
    /// 服务器回了 `NO [MODIFIED]`：基线过期，**不是**拒绝。
    Modified,
    /// 服务器回了 `NO`（其它原因）：明确拒绝。
    Rejected(String),
}

impl StoreResult {
    /// 打印用的简短标签。
    fn label(&self) -> &str {
        match self {
            StoreResult::Ok => "OK",
            StoreResult::Modified => "NO [MODIFIED]",
            StoreResult::Rejected(_) => "NO",
        }
    }
}

#[tokio::main]
async fn main() {
    if let Err(message) = run().await {
        eprintln!("失败: {message}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), String> {
    let mut provider = "163".to_string();
    let mut folder = "INBOX".to_string();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--provider" | "-p" => provider = args.next().ok_or("--provider 需要一个值")?,
            "--folder" | "-f" => folder = args.next().ok_or("--folder 需要一个值")?,
            "--help" | "-h" => {
                println!("用法：xremail-flag-smoke --provider <段名> [--folder <邮箱名>]");
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

    let mut config = ImapConfig::imaps(host);
    config.port = 993;
    config.connect_timeout = Duration::from_secs(15);
    config.command_timeout = Duration::from_secs(60);

    let (mut session, _caps, tls) = connect_and_login_with(
        &config,
        &TlsTrust::SystemRoots,
        &account_credential.user,
        &password,
    )
    .await
    .map_err(|e| format!("连接或登录失败: {e}"))?;

    println!("== 标记回写冒烟（provider = {provider}，文件夹 = {folder}）==");
    println!("TLS: {} / {}", tls.protocol, tls.cipher);

    // 写标记必须以**可写**方式打开邮箱（只读的 EXAMINE 拒绝 STORE）。
    session
        .select(&folder)
        .await
        .map_err(|e| format!("以可写方式打开文件夹失败: {e}"))?;

    // 找一封邮件：优先挑**没被标记已读**的，这样「加 \Seen」是真变化，
    // 验完能干净地恢复原状。
    let uids = match run_checked(&mut session, "UID SEARCH ALL")
        .await
        .map_err(|e| format!("UID SEARCH 失败: {e}"))?
    {
        CommandOutcome::Search(ids) => ids,
        _ => return Err("UID SEARCH 没有返回 id 列表".into()),
    };
    if uids.is_empty() {
        return Err("收件箱里没有邮件，没什么可验的".into());
    }

    let mut picked: Option<(u32, bool)> = None; // (uid, 原本有没有 \Seen)
    for uid in uids.iter().copied() {
        let flags = fetch_flags(&mut session, uid).await?;
        if !flags.iter().any(|f| f.eq_ignore_ascii_case("\\Seen")) {
            picked = Some((uid, false));
            break;
        }
        if picked.is_none() {
            picked = Some((uid, true));
        }
    }
    let (uid, originally_seen) = picked.ok_or("没能挑出一封邮件")?;

    let before = fetch_flags(&mut session, uid).await?;
    println!(
        "收件箱 {} 封；选中 uid={uid}（原本已读 = {originally_seen}，原本标记 {} 个）",
        uids.len(),
        before.len()
    );

    // 1. 加 `\Seen`。
    let add = store_command(&mut session, uid, "+FLAGS", "\\Seen").await;
    println!("1) UID STORE {uid} +FLAGS (\\Seen) → {}", add.label());

    // 2. **回读确认**：服务器说 OK 不等于真改了——必须读回来看。
    let after = fetch_flags(&mut session, uid).await?;
    let now_seen = after.iter().any(|f| f.eq_ignore_ascii_case("\\Seen"));
    println!(
        "2) 回读确认：\\Seen = {now_seen}（标记 {} 个）",
        after.len()
    );
    if add == StoreResult::Ok && !now_seen {
        println!("   ⚠ 服务器回了 OK，但回读发现标记没变 —— 这是「看起来成功、其实没成」");
    }

    // 3. 再加一次同样的标记：服务器多半回 `NO [MODIFIED]`。
    //    这一步验证「基线过期」在真服务器上确实是一种**独立的**回答。
    let again = store_command(&mut session, uid, "+FLAGS", "\\Seen").await;
    println!(
        "3) 重复 UID STORE {uid} +FLAGS (\\Seen) → {}",
        again.label()
    );

    // 4. 恢复原状。
    let restore = if originally_seen {
        StoreResult::Ok
    } else {
        let result = store_command(&mut session, uid, "-FLAGS", "\\Seen").await;
        println!(
            "4) 恢复：UID STORE {uid} -FLAGS (\\Seen) → {}",
            result.label()
        );
        result
    };
    let restored = fetch_flags(&mut session, uid).await?;
    let restored_seen = restored.iter().any(|f| f.eq_ignore_ascii_case("\\Seen"));
    println!("5) 恢复后回读：\\Seen = {restored_seen}（应当 == 原本 {originally_seen}）");

    println!();
    println!("--- 结论 ---");
    println!("加标记: {}", add.label());
    println!("回读确认生效: {now_seen}");
    println!("重复加同一个标记: {}", again.label());
    println!(
        "状态已恢复: {}",
        if restored_seen == originally_seen {
            "是"
        } else {
            "**否 —— 需要人工确认**"
        }
    );
    if matches!(restore, StoreResult::Rejected(_)) {
        println!("恢复命令被拒绝: {}", restore.label());
    }

    Ok(())
}

/// 读一封邮件当前的标记。
async fn fetch_flags(
    session: &mut xremail_protocol_imap::Session,
    uid: u32,
) -> Result<Vec<String>, String> {
    let command = format!("UID FETCH {uid} (UID FLAGS)");
    match run_checked(session, &command)
        .await
        .map_err(|e| format!("{command} 失败: {e}"))?
    {
        CommandOutcome::Fetches(messages) => {
            let flags = messages
                .into_iter()
                .flat_map(|m| m.flags)
                .collect::<Vec<_>>();
            Ok(flags)
        }
        _ => Err(format!("{command} 没有返回 FETCH 结果")),
    }
}

/// 发一条 `UID STORE` 命令，并把结果按**回写链路的分法**分类。
///
/// 分法必须和 `xremail-sync` 里的一致：
/// * `OK` → 服务器接受；
/// * `NO [MODIFIED]` → 基线过期（**不是**拒绝，要重算后重试）；
/// * 其它 `NO` → 明确拒绝。
async fn store_command(
    session: &mut xremail_protocol_imap::Session,
    uid: u32,
    mode: &str,
    flags: &str,
) -> StoreResult {
    let command = format!("UID STORE {uid} {mode} ({flags})");
    match run_checked(session, &command).await {
        Ok(_) => StoreResult::Ok,
        Err(ProtocolError::ConditionFailed(info)) => {
            if info.to_lowercase().contains("modified") {
                StoreResult::Modified
            } else {
                StoreResult::Rejected(info)
            }
        }
        Err(error) => StoreResult::Rejected(error.to_string()),
    }
}
