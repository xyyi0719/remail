//! `xremail-smtp-probe` 的端到端自检。
//!
//! 探测链路本身由 `crates/xremail-protocol-smtp/tests/probe_against_fake.rs` 覆盖；
//! 这里补的是**命令行这一层**：参数解析、凭据读取、markdown 报告渲染、
//! 退出码——也就是用户真正会用到的那部分。
//!
//! 只读红线也在这里被断言：假服务器记录客户端发过的每一条命令，
//! 报告里绝不能出现 `MAIL` / `RCPT` / `DATA` / `RSET`，输出里绝不能出现口令。

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use xremail_testkit::{FakeSmtpServer, SmtpServerConfig};

/// 被测的二进制（cargo 会自动构建它）。
const BIN: &str = env!("CARGO_BIN_EXE_xremail-smtp-probe");

/// 同一进程内的自增序号。
static SEQ: AtomicU64 = AtomicU64::new(0);

/// 临时文件路径。
///
/// **必须保证同进程内不重名**：这几个测试是**并行**跑的，
/// 而 Windows 的系统时钟分辨率约 15.6ms——光靠 `as_nanos()`
/// 不足以区分同一时刻的两次调用（实测踩过：
/// 两个测试的 CA 文件路径撞车，后写的覆盖先写的，
/// 于是客户端拿着**别的测试的证书**去验服务器证书，
/// 报 `invalid peer certificate: BadSignature`）。
///
/// 所以额外加一个进程内自增序号。
fn temp_path(name: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("时钟")
        .as_nanos();
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "xremail-smtp-probe-{}-{nanos}-{seq}.{name}",
        std::process::id()
    ))
}

/// 跑一次 CLI，拿 stdout。
fn run_cli(args: &[&str]) -> (bool, String, String) {
    let output = Command::new(BIN)
        .args(args)
        .output()
        .expect("运行 xremail-smtp-probe");
    (
        output.status.success(),
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn starttls_path_produces_a_markdown_report() {
    let server = FakeSmtpServer::start(
        SmtpServerConfig::new()
            .with_extensions(&["SIZE 52428800", "8BITMIME", "SMTPUTF8", "PIPELINING", "DSN"])
            .with_auth(&["PLAIN", "LOGIN"], "user@example.com", "secret"),
    )
    .await
    .expect("启动假 SMTP 服务器");

    let ca_path = temp_path("ca.pem");
    std::fs::write(&ca_path, server.ca_pem()).expect("写 CA");
    let cred_path = temp_path("accounts.txt");
    std::fs::write(
        &cred_path,
        "[test]\nuser = user@example.com\npassword = secret\n",
    )
    .expect("写凭据");

    let (ok, stdout, stderr) = run_cli(&[
        "--provider",
        "test",
        "--host",
        "localhost",
        "--port",
        &server.port().to_string(),
        "--tls-mode",
        "starttls",
        "--credentials",
        cred_path.to_str().expect("路径"),
        "--ca-file",
        ca_path.to_str().expect("路径"),
    ]);

    assert!(ok, "CLI 退出码不是 0\nstdout:\n{stdout}\nstderr:\n{stderr}");
    for expect in [
        "# Xremail SMTP 探测报告",
        "## TLS",
        "## EHLO 扩展",
        "`SIZE 52428800`",
        "`8BITMIME`",
        "`SMTPUTF8`",
        "## 认证",
        "认证结果：✅ 成功",
        "## 建议配置",
        "smtp_tls = \"starttls\"",
        "smtp_auth = \"login\"",
    ] {
        assert!(stdout.contains(expect), "报告里缺少: {expect}\n{stdout}");
    }

    // 凭据零泄漏：口令与完整账号都不许出现在报告里。
    assert!(!stdout.contains("secret"), "报告里出现了口令!\n{stdout}");
    assert!(
        !stdout.contains("user@example.com"),
        "报告里出现了完整账号!\n{stdout}"
    );
    assert!(stdout.contains("u***@example.com"), "账号应脱敏:\n{stdout}");

    // 只读红线：只允许 EHLO / STARTTLS / AUTH / QUIT。
    let commands = server.commands();
    for forbidden in ["MAIL", "RCPT", "DATA", "RSET"] {
        assert!(
            !commands.iter().any(|c| c == forbidden),
            "客户端发过了投递类命令: {forbidden} / {commands:?}"
        );
    }
    assert_eq!(commands, vec!["EHLO", "STARTTLS", "EHLO", "AUTH", "QUIT"]);

    let _ = std::fs::remove_file(&ca_path);
    let _ = std::fs::remove_file(&cred_path);
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn implicit_tls_with_a_wrong_password_reports_the_failure_without_the_password() {
    let server = FakeSmtpServer::start(SmtpServerConfig::implicit_tls().with_auth(
        &["PLAIN"],
        "user@example.com",
        "secret",
    ))
    .await
    .expect("启动假 SMTP 服务器");

    let ca_path = temp_path("ca.pem");
    std::fs::write(&ca_path, server.ca_pem()).expect("写 CA");
    let cred_path = temp_path("accounts.txt");
    std::fs::write(
        &cred_path,
        "[test]\nuser = user@example.com\npassword = wrong-password\n",
    )
    .expect("写凭据");

    let (ok, stdout, stderr) = run_cli(&[
        "--provider",
        "test",
        "--host",
        "localhost",
        "--port",
        &server.port().to_string(),
        "--tls-mode",
        "direct",
        "--credentials",
        cred_path.to_str().expect("路径"),
        "--ca-file",
        ca_path.to_str().expect("路径"),
    ]);

    assert!(ok, "CLI 退出码不是 0\nstdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(stdout.contains("认证结果：❌ 失败"), "{stdout}");
    // 失败时也要带上服务器原文（不含口令）。
    assert!(stdout.contains("535"), "应带服务器原文 535:\n{stdout}");
    assert!(
        !stdout.contains("wrong-password"),
        "报告里出现了口令!\n{stdout}"
    );

    let _ = std::fs::remove_file(&ca_path);
    let _ = std::fs::remove_file(&cred_path);
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn no_auth_mode_never_sends_credentials() {
    let server = FakeSmtpServer::start(SmtpServerConfig::new())
        .await
        .expect("启动假 SMTP 服务器");

    let ca_path = temp_path("ca.pem");
    std::fs::write(&ca_path, server.ca_pem()).expect("写 CA");

    // 故意给一个**不存在**的 provider：--no-auth 时不该因此失败。
    let (ok, stdout, stderr) = run_cli(&[
        "--provider",
        "not-configured",
        "--host",
        "localhost",
        "--port",
        &server.port().to_string(),
        "--tls-mode",
        "starttls",
        "--no-auth",
        "--ca-file",
        ca_path.to_str().expect("路径"),
    ]);

    assert!(ok, "CLI 退出码不是 0\nstdout:\n{stdout}\nstderr:\n{stderr}");
    assert!(stdout.contains("认证结果：⏭️ 未认证"), "{stdout}");
    let commands = server.commands();
    assert!(
        !commands.iter().any(|c| c == "AUTH"),
        "--no-auth 时不该发 AUTH: {commands:?}"
    );

    let _ = std::fs::remove_file(&ca_path);
    server.shutdown().await;
}
