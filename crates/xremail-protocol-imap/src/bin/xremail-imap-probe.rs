//! 只读 IMAP 探测工具。
//!
//! **严格只读**：只执行 LOGIN / CAPABILITY / ID / LIST / EXAMINE / STATUS / IDLE+DONE。
//! 绝不执行 STORE / MOVE / COPY / DELETE / EXPUNGE / APPEND，也不发 SMTP 邮件。
//!
//! ## 凭据处理
//!
//! 凭据从**本机文件**读取（`%USERPROFILE%/.xremail/accounts.txt`），
//! 按 `[provider]` 段落选择账号。
//!
//! * 凭据**不会**出现在命令行参数里（命令行参数会进 shell 历史）。
//! * 凭据**不会**出现在本程序的任何输出里。
//! * 本程序**不**写任何日志文件。
//!
//! 用法：
//!
//! ```text
//! cargo run -p xremail-protocol-imap --bin xremail-imap-probe -- --list
//! cargo run -p xremail-protocol-imap --bin xremail-imap-probe -- --provider 163
//! ```

use std::path::PathBuf;
use std::process::ExitCode;

use xremail_protocol_imap::{CredentialStore, ImapConfig, ProbeConfig};

struct Args {
    host: String,
    port: u16,
    mailbox: String,
    credentials: PathBuf,
    provider: String,
    try_idle: bool,
    list_only: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut host = "imap.163.com".to_string();
    let mut port = 993u16;
    let mut mailbox = "INBOX".to_string();
    let mut credentials = xremail_protocol_imap::default_accounts_path();
    let mut provider = "163".to_string();
    let mut try_idle = true;
    let mut list_only = false;

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--host" => {
                host = args.next().ok_or("--host 缺少取值")?;
            }
            "--port" => {
                let raw = args.next().ok_or("--port 缺少取值")?;
                port = raw.parse().map_err(|_| "--port 不是数字")?;
            }
            "--mailbox" => {
                mailbox = args.next().ok_or("--mailbox 缺少取值")?;
            }
            "--credentials" => {
                credentials = PathBuf::from(args.next().ok_or("--credentials 缺少取值")?);
            }
            "--provider" => {
                provider = args.next().ok_or("--provider 缺少取值")?;
            }
            "--no-idle" => {
                try_idle = false;
            }
            "--list" => {
                list_only = true;
            }
            "-h" | "--help" => {
                println!("{HELP}");
                std::process::exit(0);
            }
            other => return Err(format!("未知参数: {other}")),
        }
    }

    Ok(Args {
        host,
        port,
        mailbox,
        credentials,
        provider,
        try_idle,
        list_only,
    })
}

const HELP: &str = "\
用法：xremail-imap-probe [选项]

  --list                     只列出凭据文件里已配置的 provider
  --provider <标识>          用哪个账号（对应文件里的 [段落名]），默认 163
  --host <主机名>            默认 imap.163.com
  --port <端口>              默认 993（隐式 TLS）
  --mailbox <邮箱名>         默认 INBOX
  --credentials <文件>       默认 %USERPROFILE%/.xremail/accounts.txt
  --no-idle                  不试 IDLE

只读探测：只执行 LOGIN / CAPABILITY / ID / LIST / EXAMINE / STATUS / IDLE+DONE。
不会修改服务端任何状态，也不会发信。
";

#[tokio::main]
async fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(args) => args,
        Err(e) => {
            eprintln!("参数错误: {e}\n\n{HELP}");
            return ExitCode::from(2);
        }
    };

    let store = match CredentialStore::read_from_file(&args.credentials) {
        Ok(store) => store,
        Err(e) => {
            eprintln!("{e}");
            eprintln!("凭据文件位置: {}", args.credentials.display());
            eprintln!("模板见同目录的 accounts.template");
            return ExitCode::from(2);
        }
    };

    if args.list_only {
        println!("凭据文件: {}", args.credentials.display());
        if store.is_empty() {
            println!("（没有配置任何账号）");
        } else {
            for provider in store.providers() {
                let account = store.get(provider).unwrap();
                println!("- {provider:<8} {}", account.masked_user());
            }
        }
        return ExitCode::SUCCESS;
    }

    let credential = match store.get(&args.provider) {
        Ok(credential) => credential,
        Err(e) => {
            eprintln!("{e}");
            eprintln!("已配置的 provider: {:?}", store.providers());
            return ExitCode::from(2);
        }
    };

    let mut server = ImapConfig::imaps(args.host.clone());
    server.port = args.port;

    let config = ProbeConfig {
        server,
        mailbox: args.mailbox.clone(),
        try_idle: args.try_idle,
    };

    println!("# Xremail IMAP 只读探测报告");
    println!();
    println!("- 目标       : {}:{}", args.host, args.port);
    println!("- provider   : {}", args.provider);
    println!("- 账号       : {}", credential.masked_user());
    println!("- 凭据来源   : {}", args.credentials.display());
    println!("- 检查邮箱   : {}", args.mailbox);
    println!("- 客户端     : xremail-protocol-imap（rustls + async-imap）");
    println!();

    let password = match credential.password.as_str() {
        Some(password) => password,
        None => {
            eprintln!("授权码不是合法的 UTF-8 文本");
            return ExitCode::from(2);
        }
    };

    let result = xremail_protocol_imap::probe(&config, &credential.user, password).await;

    match result {
        Ok(report) => {
            print_report(&report);
            ExitCode::SUCCESS
        }
        Err(e) => {
            println!("## 结果");
            println!();
            println!("- **失败阶段**: {}", e);
            println!("- 可重试     : {}", e.is_retryable());
            println!("- 需用户处理 : {}", e.requires_user_action());
            eprintln!("探测失败: {e}");
            ExitCode::FAILURE
        }
    }
}

/// 打印脱敏报告。
fn print_report(report: &xremail_protocol_imap::ProbeReport) {
    println!("## TLS");
    println!();
    println!("| 项 | 值 |");
    println!("|---|---|");
    println!("| 协议版本 | {} |", report.tls.protocol);
    println!("| 加密套件 | {} |", report.tls.cipher);
    println!(
        "| 证书链与主机名校验 | {} |",
        if report.tls.verified {
            "✅ 通过（rustls 强制校验，系统根）"
        } else {
            "❌ 未通过"
        }
    );
    println!();

    println!("## 认证");
    println!();
    println!("- 结果: ✅ 登录成功");
    println!(
        "- 支持的认证机制: {}",
        if report.auth_mechanisms.is_empty() {
            "（未宣告）".to_string()
        } else {
            report.auth_mechanisms.join(", ")
        }
    );
    println!();

    println!("## 扩展支持度");
    println!();
    println!("| 扩展 | 支持 | 意义 |");
    println!("|---|---|---|");
    let e = &report.extensions;
    println!("| IDLE | {} | 实时推送的基础 |", yn(e.idle));
    println!(
        "| CONDSTORE | {} | 条件写（UNCHANGEDSINCE）的基础 |",
        yn(e.condstore)
    );
    println!("| QRESYNC | {} | CONDSTORE 的增量同步扩展 |", yn(e.qresync));
    println!(
        "| UIDPLUS | {} | MOVE/COPY 返回 UIDVALIDITY 与目标 UID |",
        yn(e.uidplus)
    );
    println!(
        "| MOVE | {} | 一步移动；无则需 COPY+\\Deleted+EXPUNGE |",
        yn(e.move_)
    );
    println!("| LIST-EXTENDED | {} | 特殊用途标记 |", yn(e.list_extended));
    println!("| NAMESPACE | {} | 文件夹层级与分隔符 |", yn(e.namespace));
    println!("| UTF8 | {} | UTF-8 支持 |", yn(e.utf8));
    println!("| ID | {} | 服务器标识 |", yn(e.id));
    println!("| QUOTA | {} | 配额查询 |", yn(e.quota));
    println!("| LITERAL+ | {} | 非同步字面量 |", yn(e.literal_plus));
    println!("| LITERAL- | {} | 旧式同步字面量 |", yn(e.literal_minus));
    println!();

    println!("## 文件夹（脱敏）");
    println!();
    println!("- 总数: {}", report.folders.total);
    println!(
        "- 标准文件夹: {}",
        if report.folders.standard.is_empty() {
            "（无）".to_string()
        } else {
            report.folders.standard.join(", ")
        }
    );
    println!(
        "- 非标准文件夹: {} 个（名字不记录）",
        report.folders.non_standard_count
    );
    println!(
        "- 层级分隔符: {}",
        report.folders.delimiter.as_deref().unwrap_or("未观察到")
    );
    println!();
    println!("| 角色 | 文件夹 |");
    println!("|---|---|");
    for folder in &report.folders.roles {
        println!("| `{}` | {} |", folder.role, folder.name);
    }
    println!();

    println!("## 邮箱概况（EXAMINE，只读）");
    println!();
    println!("| 项 | 值 |");
    println!("|---|---|");
    println!("| 邮件数 | {} |", report.mailbox.exists);
    println!(
        "| 未读数 | {} |",
        report
            .mailbox
            .unseen
            .map(|u| u.to_string())
            .unwrap_or_else(|| "—".into())
    );
    println!(
        "| UIDVALIDITY (epoch) | {} |",
        report
            .mailbox
            .uid_validity
            .map(|v| v.to_string())
            .unwrap_or_else(|| "—".into())
    );
    println!(
        "| UIDNEXT | {} |",
        report
            .mailbox
            .uid_next
            .map(|v| v.to_string())
            .unwrap_or_else(|| "—".into())
    );
    println!(
        "| 最高 MODSEQ | {} |",
        report
            .mailbox
            .highest_modseq
            .map(|v| v.to_string())
            .unwrap_or_else(|| "—".into())
    );
    println!(
        "| 永久标记 | {} |",
        if report.mailbox.permanent_flags.is_empty() {
            "（无）".to_string()
        } else {
            report.mailbox.permanent_flags.join(", ")
        }
    );
    if let Some(accepted) = report.mailbox.idle_accepted {
        println!(
            "| IDLE 实测 | {} |",
            if accepted {
                "✅ 服务器接受"
            } else {
                "❌ 服务器拒绝"
            }
        );
    }
    println!();

    println!("## 能力集（认证后）");
    println!();
    for cap in &report.capabilities {
        println!("- `{}`", cap);
    }
}

/// 是 / 否。
fn yn(value: bool) -> &'static str {
    if value { "✅" } else { "❌" }
}
