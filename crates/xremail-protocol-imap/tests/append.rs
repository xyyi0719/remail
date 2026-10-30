//! APPEND 的端到端测试：真客户端 × 假服务器。
//!
//! 重点验证三件事：
//! * 字面量按**声明的长度**原样送达服务器（多一个字节少一个字节都不行）；
//! * 目标文件夹要传**线路上的原名**（不是解码后的名字）；
//! * 服务器回 `NO` 时客户端**一定**报错——不能把「没存进去」当成「存好了」。

use std::time::Duration;

use xremail_protocol_imap::{
    AppendOptions, ImapConfig, Session, TlsTrust, append_message, connect_and_login_with,
    run_checked,
};
use xremail_testkit::{FakeImapServer, MailboxFixture, MessageFixture, ServerConfig};

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

fn config_with_sent() -> ServerConfig {
    ServerConfig {
        mailboxes: vec![
            MailboxFixture::new("INBOX").with_message(MessageFixture::new(1)),
            MailboxFixture::new("已发送").with_attribute("\\Sent"),
        ],
        ..ServerConfig::default()
    }
}

/// 列出文件夹，返回「已发送」在**线路上**的名字。
async fn sent_raw_name(session: &mut Session) -> String {
    let outcome = run_checked(session, "LIST \"\" \"*\"")
        .await
        .expect("LIST 应当成功");
    outcome
        .list()
        .iter()
        .find(|folder| folder.role.as_deref() == Some("sent"))
        .expect("应当有「已发送」文件夹")
        .raw_name
        .clone()
}

#[tokio::test]
async fn append_stores_the_message_in_the_target_folder() {
    let (server, imap, trust) = start(config_with_sent()).await;
    let (mut session, _caps, _tls) =
        connect_and_login_with(&imap, &trust, "tester@example.com", "test-password")
            .await
            .expect("登录应当成功");

    let raw_name = sent_raw_name(&mut session).await;
    let body = "Subject: 已发送测试\r\n\r\n正文里有中文\r\n";

    let outcome = append_message(
        &mut session,
        &raw_name,
        body.as_bytes(),
        &AppendOptions {
            flags: vec!["\\Seen".to_string()],
            internal_date: Some("26-Oct-2026 12:00:00 +0800".to_string()),
        },
    )
    .await
    .expect("APPEND 应当成功");

    assert_eq!(outcome.uid, Some(1), "空文件夹里 append 的新 UID 应当是 1");

    let mailboxes = server.mailboxes();
    let sent = mailboxes
        .iter()
        .find(|mailbox| mailbox.name == "已发送")
        .expect("应当有「已发送」文件夹");
    assert_eq!(sent.messages.len(), 1, "服务器上应当多了一封信");
    assert_eq!(
        sent.messages[0].content, body,
        "正文必须原样送达，一个字节都不能少"
    );
    assert!(
        sent.messages[0].has_flag("\\Seen"),
        "APPEND 带的标记应当被存下"
    );

    server.shutdown().await;
}

#[tokio::test]
async fn append_reports_the_uidvalidity_too() {
    let (server, imap, trust) = start(config_with_sent()).await;
    let (mut session, _caps, _tls) =
        connect_and_login_with(&imap, &trust, "tester@example.com", "test-password")
            .await
            .expect("登录应当成功");

    let raw_name = sent_raw_name(&mut session).await;
    let outcome = append_message(
        &mut session,
        &raw_name,
        b"Subject: x\r\n\r\n",
        &AppendOptions::default(),
    )
    .await
    .expect("APPEND 应当成功");

    assert_eq!(outcome.uid_validity, Some(server.uid_validity()));
    server.shutdown().await;
}

#[tokio::test]
async fn append_to_a_missing_folder_is_reported_as_an_error() {
    let (server, imap, trust) = start(config_with_sent()).await;
    let (mut session, _caps, _tls) =
        connect_and_login_with(&imap, &trust, "tester@example.com", "test-password")
            .await
            .expect("登录应当成功");

    let error = append_message(
        &mut session,
        "不存在的文件夹",
        b"Subject: x\r\n\r\n",
        &AppendOptions::default(),
    )
    .await
    .expect_err("存到不存在的文件夹必须报错");

    let text = error.to_string();
    assert!(
        text.contains("NO") || text.contains("Mailbox does not exist"),
        "错误里应当说清是服务器拒绝了，实际是：{text}"
    );

    let sent = server
        .mailboxes()
        .into_iter()
        .find(|mailbox| mailbox.name == "已发送")
        .expect("应当有「已发送」文件夹");
    assert!(sent.messages.is_empty(), "没存成就不该有多出来的信");

    server.shutdown().await;
}

#[tokio::test]
async fn empty_content_is_refused_without_touching_the_server() {
    let (server, imap, trust) = start(config_with_sent()).await;
    let (mut session, _caps, _tls) =
        connect_and_login_with(&imap, &trust, "tester@example.com", "test-password")
            .await
            .expect("登录应当成功");

    let raw_name = sent_raw_name(&mut session).await;
    let error = append_message(&mut session, &raw_name, b"", &AppendOptions::default())
        .await
        .expect_err("空正文必须报错");

    assert!(error.to_string().contains("空"));

    let appended = server
        .command_log()
        .into_iter()
        .filter(|line| line.starts_with("APPEND"))
        .count();
    assert_eq!(appended, 0, "本地就该拦下来，不该往服务器发命令");

    server.shutdown().await;
}
