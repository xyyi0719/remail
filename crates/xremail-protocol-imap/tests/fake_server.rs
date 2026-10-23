//! 假 IMAP 服务器 × 真客户端的端到端测试。
//!
//! 这是 T7 的核心：**协议层的测试不再依赖真实邮箱**。
//! 真服务器只能复现一种情况；假服务器把各家服务商的怪癖做成开关，
//! 在 CI 里全部跑一遍。
//!
//! 重点复现的是 163 的两个实测行为：
//! * 认证前拒绝一切命令；
//! * **必须先发 `ID`**（RFC 2971）才能访问邮箱。

use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio_rustls::TlsConnector;
use xremail_protocol_imap::{ImapConfig, TlsTrust, connect_and_login_with};
use xremail_testkit::{FakeImapServer, Fault, MailboxFixture, MessageFixture, ServerConfig};

/// 取错误。
///
/// `Capabilities` 没实现 `Debug`，所以 `expect_err` 用不了。
fn expect_err<T>(
    result: Result<T, xremail_protocol_imap::ProtocolError>,
    message: &str,
) -> xremail_protocol_imap::ProtocolError {
    match result {
        Ok(_) => panic!("{message}"),
        Err(err) => err,
    }
}

/// 起一个假服务器并返回可直接用的客户端配置。
async fn start(config: ServerConfig) -> (FakeImapServer, ImapConfig, TlsTrust) {
    let server = FakeImapServer::start(config)
        .await
        .expect("启动假服务器失败");

    let mut imap = ImapConfig::imaps(server.host());
    imap.port = server.port();
    imap.connect_timeout = Duration::from_secs(5);
    imap.command_timeout = Duration::from_secs(10);

    let trust = TlsTrust::Roots(server.root_store());

    (server, imap, trust)
}

/// 默认配置 + 几封邮件。
fn config_with_mail() -> ServerConfig {
    ServerConfig {
        mailboxes: vec![
            MailboxFixture::new("INBOX")
                .with_message(MessageFixture::new(1_792_658_701).with_flag("\\Seen")),
            MailboxFixture::new("草稿箱").with_attribute("\\Drafts"),
            MailboxFixture::new("已发送").with_attribute("\\Sent"),
        ],
        ..ServerConfig::default()
    }
}

/// 用**裸连接**发一条命令，绕过协议层。
///
/// 用来验证假服务器**自身**的行为（比如「认证前拒绝一切命令」），
/// 而不是客户端的行为。
async fn raw_command(server: &FakeImapServer, command: &str) -> String {
    let connector = TlsConnector::from(std::sync::Arc::new(server.client_config()));
    let tcp = tokio::net::TcpStream::connect(server.addr())
        .await
        .expect("连假服务器失败");
    let name = rustls::pki_types::ServerName::try_from(server.host().to_string()).unwrap();
    let stream = connector.connect(name, tcp).await.expect("TLS 握手失败");

    // 先读掉问候行。
    let mut reader = BufReader::new(stream);
    let mut greeting = String::new();
    reader.read_line(&mut greeting).await.expect("读问候行失败");

    // 写命令。
    let mut stream = reader.into_inner();
    stream
        .write_all(format!("c1 {command}\r\n").as_bytes())
        .await
        .expect("写命令失败");
    stream.flush().await.ok();

    // 读一行响应。
    let mut reader = BufReader::new(stream);
    let mut response = String::new();
    reader.read_line(&mut response).await.expect("读响应失败");
    response.trim_end().to_string()
}

#[tokio::test]
async fn login_succeeds_and_returns_capabilities() {
    let (server, imap, trust) = start(config_with_mail()).await;

    let (_session, capabilities, tls) =
        connect_and_login_with(&imap, &trust, "tester@example.com", "test-password")
            .await
            .expect("登录应当成功");

    assert!(capabilities.has_str("IMAP4rev1"));
    assert!(capabilities.has_str("UIDPLUS"));
    assert!(tls.verified, "假服务器的证书应由测试显式信任");
    server.shutdown().await;
}

#[tokio::test]
async fn wrong_credentials_fail_and_need_user_action() {
    let (server, imap, trust) = start(config_with_mail()).await;

    let err = expect_err(
        connect_and_login_with(&imap, &trust, "tester@example.com", "wrong-password").await,
        "错误口令应当失败",
    );

    assert!(
        err.requires_user_action(),
        "服务器回了 NO，凭据被拒，需要用户处理"
    );
    server.shutdown().await;
}

#[tokio::test]
async fn commands_before_auth_are_rejected_like_163() {
    // 假服务器的行为本身：未登录时任何命令都被拒。
    let (server, _imap, _trust) = start(config_with_mail()).await;

    let response = raw_command(&server, "CAPABILITY").await;

    assert!(
        response.contains("BAD command not support"),
        "认证前应当拒绝一切命令，实际是：{response}"
    );
    server.shutdown().await;
}

#[tokio::test]
async fn even_logout_is_rejected_before_auth() {
    // 163 实测：LOGOUT 也回 BAD。
    let (server, _imap, _trust) = start(config_with_mail()).await;

    let response = raw_command(&server, "LOGOUT").await;

    assert!(
        response.contains("BAD command not support"),
        "认证前 LOGOUT 也应被拒，实际是：{response}"
    );
    server.shutdown().await;
}

#[tokio::test]
async fn client_logs_in_first_so_pre_auth_rejection_is_fine() {
    // 假服务器像 163 一样严，客户端照样能用——说明客户端没有在认证前发命令。
    let (server, imap, trust) = start(config_with_mail()).await;

    let (_session, _capabilities, _tls) =
        connect_and_login_with(&imap, &trust, "tester@example.com", "test-password")
            .await
            .expect("登录应当成功");

    server.shutdown().await;
}

#[tokio::test]
async fn requiring_id_works_because_the_client_sends_it() {
    let mut config = config_with_mail();
    config.require_id = true;
    let (server, imap, trust) = start(config).await;

    // connect_and_login_with 会在登录后自动发 ID（服务器宣告了 ID 能力），
    // 所以这里应当能连上——这正是实现要求的。
    let (_session, _capabilities, _tls) =
        connect_and_login_with(&imap, &trust, "tester@example.com", "test-password")
            .await
            .expect("发了 ID 就应当能进");

    server.shutdown().await;
}

#[tokio::test]
async fn server_without_id_capability_still_works() {
    // 不宣告 ID → 客户端不发 ID；服务器也不要求 ID → 一切正常。
    let mut config = config_with_mail();
    config.capabilities.retain(|c| c != "ID");
    config.require_id = false;
    let (server, imap, trust) = start(config).await;

    let (_session, capabilities, _tls) =
        connect_and_login_with(&imap, &trust, "tester@example.com", "test-password")
            .await
            .expect("不宣告 ID 也能连");

    assert!(!capabilities.has_str("ID"));
    server.shutdown().await;
}

#[tokio::test]
async fn examine_rejected_but_select_still_works() {
    let mut config = config_with_mail();
    config.reject_examine = true;
    let (server, imap, trust) = start(config).await;

    let (mut session, _caps, _tls) =
        connect_and_login_with(&imap, &trust, "tester@example.com", "test-password")
            .await
            .expect("登录应当成功");

    let err = session
        .examine("INBOX")
        .await
        .expect_err("EXAMINE 应被拒绝");

    assert!(err.to_string().contains("Unsafe Login"));

    // SELECT 仍然可用。
    session.select("INBOX").await.expect("SELECT 应当可用");
    server.shutdown().await;
}

#[tokio::test]
async fn disconnect_after_login_is_retryable() {
    let mut config = config_with_mail();
    config.faults.push(Fault::DropAfterLogin);
    let (server, imap, trust) = start(config).await;

    let err = expect_err(
        connect_and_login_with(&imap, &trust, "tester@example.com", "test-password").await,
        "断连应导致失败",
    );

    assert!(
        err.is_retryable(),
        "传输层断连值得重试（不是“密码错了”），实际错误是：{err}"
    );
    assert!(!err.requires_user_action(), "连接断了不该让用户去检查密码");
    server.shutdown().await;
}

#[tokio::test]
async fn server_actually_listens() {
    let (server, _imap, _trust) = start(config_with_mail()).await;

    assert!(server.port() > 0);
    assert_eq!(server.addr().ip().to_string(), "127.0.0.1");

    server.shutdown().await;
}
