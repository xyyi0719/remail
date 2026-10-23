//! 假 IMAP 服务器 × 真客户端：UID FETCH / UID SEARCH / UID STORE。
//!
//! 这三个命令是同步引擎的**核心依赖**：
//! * `UID FETCH` 拉元数据与邮件内容；
//! * `UID SEARCH` 做按条件查询（增量同步的起点）；
//! * `UID STORE` 回写标记。
//!
//! 有了假服务器，这些路径**不再需要真邮箱**就能测。
//!
//! ## 重要：为什么用 `run_checked` 而不是 async-imap 的同名方法
//!
//! async-imap 0.12 会**吞掉**服务器回的 `NO` / `BAD`
//! （详见 `xremail_protocol_imap::command` 的文档）。
//! 本文件里 [`async_imap_swallows_server_rejection`] 专门把这个行为钉住，
//! 其余用例一律走 `run_checked`——那是同步引擎**必须**用的入口。

use std::time::Duration;

use xremail_protocol_imap::{
    CommandOutcome, FetchedMessage, ImapConfig, ProtocolError, Session, TlsTrust,
    connect_and_login_with, run_checked,
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

/// 一封带真实内容的邮件。
fn message(uid: u32, flags: &[&str], subject: &str, body: &str) -> MessageFixture {
    let mut fixture = MessageFixture::new(uid)
        .with_subject(subject)
        .with_body(body);
    for flag in flags {
        fixture = fixture.with_flag(flag);
    }
    fixture
}

/// 三封邮件的 INBOX：UID **不连续**（像 163 那样），这样序号与 UID 一旦混淆就会出错。
fn config_with_mail() -> ServerConfig {
    ServerConfig {
        mailboxes: vec![
            MailboxFixture::new("INBOX")
                .with_message(message(1_792_658_701, &["\\Seen"], "已读的一封", "正文一"))
                .with_message(message(1_792_658_702, &[], "未读的一封", "正文二"))
                .with_message(message(
                    1_792_658_703,
                    &["\\Flagged"],
                    "加星的第三封",
                    "正文三",
                )),
        ],
        ..ServerConfig::default()
    }
}

/// 登录并**选中** INBOX（`SELECT` = 可读写）。
///
/// 注意：`EXAMINE` 打开的是只读邮箱，服务器会拒绝 `STORE`——
/// 这不是 bug，是 IMAP 语义。
async fn login_and_select(imap: &ImapConfig, trust: &TlsTrust) -> Session {
    let (mut session, _caps, _tls) =
        connect_and_login_with(imap, trust, "tester@example.com", "test-password")
            .await
            .expect("登录应当成功");
    session.select("INBOX").await.expect("SELECT 应当成功");
    session
}

/// 登录并以**只读**方式打开 INBOX（`EXAMINE`）。
async fn login_and_examine(imap: &ImapConfig, trust: &TlsTrust) -> Session {
    let (mut session, _caps, _tls) =
        connect_and_login_with(imap, trust, "tester@example.com", "test-password")
            .await
            .expect("登录应当成功");
    session.examine("INBOX").await.expect("EXAMINE 应当成功");
    session
}

/// 取错误。
///
/// 各种返回类型里有些没实现 `Debug`，所以不能用 `expect_err`。
fn expect_err<T>(result: Result<T, ProtocolError>, message: &str) -> ProtocolError {
    match result {
        Ok(_) => panic!("{message}"),
        Err(err) => err,
    }
}

/// `UID FETCH`（走安全命令层）。
async fn uid_fetch(
    session: &mut Session,
    set: &str,
    items: &str,
) -> Result<Vec<FetchedMessage>, ProtocolError> {
    let outcome = run_checked(session, &format!("UID FETCH {set} {items}")).await?;
    Ok(outcome.fetches().to_vec())
}

/// `UID SEARCH`（走安全命令层）。
async fn uid_search(session: &mut Session, query: &str) -> Result<Vec<u32>, ProtocolError> {
    let outcome = run_checked(session, &format!("UID SEARCH {query}")).await?;
    Ok(outcome.search().to_vec())
}

/// `UID STORE`（走安全命令层）。
async fn uid_store(
    session: &mut Session,
    set: &str,
    query: &str,
) -> Result<Vec<FetchedMessage>, ProtocolError> {
    let outcome = run_checked(session, &format!("UID STORE {set} {query}")).await?;
    Ok(outcome.fetches().to_vec())
}

// ---------------------------------------------------------------- UID FETCH

#[tokio::test]
async fn uid_fetch_returns_uid_flags_and_size() {
    let (server, imap, trust) = start(config_with_mail()).await;
    let mut session = login_and_select(&imap, &trust).await;

    let fetched = uid_fetch(&mut session, "1:*", "(UID FLAGS RFC822.SIZE)")
        .await
        .expect("UID FETCH 应当成功");

    assert_eq!(fetched.len(), 3);
    // 序号是 1/2/3，UID 却是那三个大数——两者绝不能混。
    assert_eq!(fetched[0].seq, 1);
    assert_eq!(fetched[0].uid, Some(1_792_658_701));
    assert_eq!(fetched[1].seq, 2);
    assert_eq!(fetched[1].uid, Some(1_792_658_702));
    assert_eq!(fetched[2].uid, Some(1_792_658_703));

    assert!(fetched[0].has_flag("\\Seen"));
    assert!(fetched[0].size.unwrap() > 0);

    server.shutdown().await;
}

#[tokio::test]
async fn uid_fetch_with_uid_range() {
    let (server, imap, trust) = start(config_with_mail()).await;
    let mut session = login_and_select(&imap, &trust).await;

    // `UID FETCH` 的 sequence set 是 **UID** 范围（不是序号）。
    let fetched = uid_fetch(&mut session, "1792658701:1792658702", "(UID)")
        .await
        .expect("UID FETCH 应当成功");

    assert_eq!(fetched.len(), 2);
    assert_eq!(fetched[0].uid, Some(1_792_658_701));
    assert_eq!(fetched[1].uid, Some(1_792_658_702));

    server.shutdown().await;
}

#[tokio::test]
async fn plain_fetch_uses_sequence_numbers() {
    let (server, imap, trust) = start(config_with_mail()).await;
    let mut session = login_and_select(&imap, &trust).await;

    // 不带 `UID` 的 `FETCH` 用的是**序号**（第几封）。
    let outcome = run_checked(&mut session, "FETCH 1:2 (UID)")
        .await
        .expect("FETCH 应当成功");
    let fetched = outcome.fetches();

    assert_eq!(fetched.len(), 2);
    assert_eq!(fetched[0].seq, 1);
    assert_eq!(fetched[0].uid, Some(1_792_658_701));
    assert_eq!(fetched[1].uid, Some(1_792_658_702));

    server.shutdown().await;
}

#[tokio::test]
async fn uid_fetch_single_uid() {
    let (server, imap, trust) = start(config_with_mail()).await;
    let mut session = login_and_select(&imap, &trust).await;

    let fetched = uid_fetch(&mut session, "1792658702", "(UID FLAGS)")
        .await
        .expect("UID FETCH 应当成功");

    assert_eq!(fetched.len(), 1);
    assert_eq!(fetched[0].uid, Some(1_792_658_702));

    server.shutdown().await;
}

#[tokio::test]
async fn uid_fetch_body_peek_does_not_set_seen() {
    let (server, imap, trust) = start(config_with_mail()).await;
    let mut session = login_and_select(&imap, &trust).await;

    // 拉第二封（本来没有 \Seen）的头。
    let fetched = uid_fetch(&mut session, "1792658702", "(UID BODY.PEEK[HEADER])")
        .await
        .expect("UID FETCH 应当成功");

    let header = fetched[0].header.as_ref().expect("应当拿到头");
    assert!(String::from_utf8_lossy(header).contains("Subject: 未读的一封"));

    // PEEK 不得置 \Seen：再查一次标记。
    let again = uid_fetch(&mut session, "1792658702", "(UID FLAGS)")
        .await
        .expect("UID FETCH 应当成功");

    assert!(
        !again[0].has_flag("\\Seen"),
        "BODY.PEEK 不应置 \\Seen，实际标记：{:?}",
        again[0].flags
    );

    server.shutdown().await;
}

#[tokio::test]
async fn uid_fetch_body_without_peek_sets_seen() {
    let (server, imap, trust) = start(config_with_mail()).await;
    let mut session = login_and_select(&imap, &trust).await;

    let fetched = uid_fetch(&mut session, "1792658702", "(UID BODY[])")
        .await
        .expect("UID FETCH 应当成功");

    assert!(fetched[0].body.is_some(), "应当拿到整封正文");

    let again = uid_fetch(&mut session, "1792658702", "(UID FLAGS)")
        .await
        .expect("UID FETCH 应当成功");

    assert!(
        again[0].has_flag("\\Seen"),
        "不带 PEEK 的 BODY 应当置上 \\Seen，实际标记：{:?}",
        again[0].flags
    );

    server.shutdown().await;
}

#[tokio::test]
async fn uid_fetch_header_fields() {
    let (server, imap, trust) = start(config_with_mail()).await;
    let mut session = login_and_select(&imap, &trust).await;

    let fetched = uid_fetch(
        &mut session,
        "1792658701",
        "(UID BODY.PEEK[HEADER.FIELDS (SUBJECT FROM)])",
    )
    .await
    .expect("UID FETCH 应当成功");

    let header =
        String::from_utf8_lossy(fetched[0].header.as_ref().expect("应当拿到头")).to_lowercase();
    assert!(header.contains("subject: 已读的一封"), "实际头是：{header}");
    assert!(
        header.contains("from: sender@example.com"),
        "实际头是：{header}"
    );
    assert!(
        !header.contains("message-id"),
        "只取指定字段，不该带别的头，实际头是：{header}"
    );

    server.shutdown().await;
}

// --------------------------------------------------------------- UID SEARCH

#[tokio::test]
async fn uid_search_all_returns_every_uid() {
    let (server, imap, trust) = start(config_with_mail()).await;
    let mut session = login_and_select(&imap, &trust).await;

    let uids = uid_search(&mut session, "ALL")
        .await
        .expect("UID SEARCH 应当成功");

    assert_eq!(uids.len(), 3);
    assert!(uids.contains(&1_792_658_701));
    assert!(uids.contains(&1_792_658_703));

    server.shutdown().await;
}

#[tokio::test]
async fn uid_search_unseen_filters_by_flag() {
    let (server, imap, trust) = start(config_with_mail()).await;
    let mut session = login_and_select(&imap, &trust).await;

    let uids = uid_search(&mut session, "UNSEEN")
        .await
        .expect("UID SEARCH 应当成功");

    assert_eq!(uids.len(), 2, "第一封有 \\Seen，不该被选中");
    assert!(!uids.contains(&1_792_658_701));
    assert!(uids.contains(&1_792_658_702));

    server.shutdown().await;
}

#[tokio::test]
async fn uid_search_by_uid_range() {
    let (server, imap, trust) = start(config_with_mail()).await;
    let mut session = login_and_select(&imap, &trust).await;

    let uids = uid_search(&mut session, "UID 1792658702:1792658703")
        .await
        .expect("UID SEARCH 应当成功");

    assert_eq!(uids.len(), 2);
    assert!(!uids.contains(&1_792_658_701));

    server.shutdown().await;
}

#[tokio::test]
async fn uid_search_by_header() {
    let (server, imap, trust) = start(config_with_mail()).await;
    let mut session = login_and_select(&imap, &trust).await;

    let uids = uid_search(&mut session, r#"SUBJECT "加星""#)
        .await
        .expect("UID SEARCH 应当成功");

    assert_eq!(uids.len(), 1);
    assert!(uids.contains(&1_792_658_703));

    server.shutdown().await;
}

#[tokio::test]
async fn uid_search_without_select_is_a_state_error() {
    let (server, imap, trust) = start(config_with_mail()).await;
    let (mut session, _caps, _tls) =
        connect_and_login_with(&imap, &trust, "tester@example.com", "test-password")
            .await
            .expect("登录应当成功");

    // IMAP 状态机：没选中邮箱就不能 SEARCH。
    let err = expect_err(
        uid_search(&mut session, "ALL").await,
        "未选中邮箱时 SEARCH 应当失败",
    );

    assert!(err.to_string().contains("State error"), "实际错误是：{err}");

    server.shutdown().await;
}

// ---------------------------------------------------------------- UID STORE

#[tokio::test]
async fn uid_store_adds_a_flag() {
    let (server, imap, trust) = start(config_with_mail()).await;
    let mut session = login_and_select(&imap, &trust).await;

    let updated = uid_store(&mut session, "1792658702", "+FLAGS (\\Flagged)")
        .await
        .expect("UID STORE 应当成功");

    assert_eq!(updated.len(), 1);
    assert_eq!(updated[0].uid, Some(1_792_658_702));
    assert!(updated[0].has_flag("\\Flagged"));

    // 状态要真的变：再查一次。
    let again = uid_fetch(&mut session, "1792658702", "(UID FLAGS)")
        .await
        .expect("UID FETCH 应当成功");
    assert!(
        again[0].has_flag("\\Flagged"),
        "状态应当真的变了，实际标记：{:?}",
        again[0].flags
    );

    server.shutdown().await;
}

#[tokio::test]
async fn uid_store_removes_a_flag() {
    let (server, imap, trust) = start(config_with_mail()).await;
    let mut session = login_and_select(&imap, &trust).await;

    uid_store(&mut session, "1792658701", "-FLAGS (\\Seen)")
        .await
        .expect("UID STORE 应当成功");

    let again = uid_fetch(&mut session, "1792658701", "(UID FLAGS)")
        .await
        .expect("UID FETCH 应当成功");
    assert!(!again[0].has_flag("\\Seen"), "\\Seen 应当被去掉");

    server.shutdown().await;
}

#[tokio::test]
async fn uid_store_in_read_only_mailbox_is_rejected() {
    let (server, imap, trust) = start(config_with_mail()).await;
    // `EXAMINE` 打开的是只读邮箱，不能改标记。
    let mut session = login_and_examine(&imap, &trust).await;

    // EXAMINE 打开的是只读邮箱，不能改标记。
    let err = expect_err(
        uid_store(&mut session, "1792658702", "+FLAGS (\\Flagged)").await,
        "只读邮箱里的 STORE 应当失败",
    );

    assert!(
        err.to_string().to_lowercase().contains("read-only"),
        "实际错误是：{err}"
    );

    server.shutdown().await;
}

#[tokio::test]
async fn uid_store_unchangedsince_rejects_stale_write() {
    let mut config = config_with_mail();
    // 宣告 CONDSTORE → 服务器才会处理 UNCHANGEDSINCE。
    config.capabilities.push("CONDSTORE".to_string());
    let (server, imap, trust) = start(config).await;
    let mut session = login_and_select(&imap, &trust).await;

    // 先改一次，modseq 就变了。
    uid_store(&mut session, "1792658702", "+FLAGS (\\Flagged)")
        .await
        .expect("UID STORE 应当成功");

    // 用过期的 modseq 做条件写 → 必须失败，且**不能**改动。
    let err = expect_err(
        uid_store(
            &mut session,
            "1792658702",
            "+FLAGS (\\Deleted) UNCHANGEDSINCE (1)",
        )
        .await,
        "过期的条件写应当失败",
    );

    assert!(
        err.to_string().to_lowercase().contains("modified"),
        "实际错误是：{err}"
    );

    // 确认没被改坏。
    let again = uid_fetch(&mut session, "1792658702", "(UID FLAGS)")
        .await
        .expect("UID FETCH 应当成功");
    assert!(
        !again[0].has_flag("\\Deleted"),
        "条件写失败就不该改动，实际标记：{:?}",
        again[0].flags
    );

    server.shutdown().await;
}

#[tokio::test]
async fn uid_store_on_missing_uid_changes_nothing() {
    let (server, imap, trust) = start(config_with_mail()).await;
    let mut session = login_and_select(&imap, &trust).await;

    let updated = uid_store(&mut session, "999999", "+FLAGS (\\Flagged)")
        .await
        .expect("UID STORE 不该报错");

    assert!(updated.is_empty(), "不存在的 UID 不该有变更响应");

    server.shutdown().await;
}

// ------------------------------------------------------- async-imap 的陷阱

#[tokio::test]
async fn async_imap_swallows_server_rejection() {
    // 这条用例把 async-imap 0.12 的行为**钉住**：
    // 服务器回 `NO` 时，`uid_store` 返回 **Ok**，只是没有变更响应。
    //
    // 后果：同步引擎会把「服务器拒绝了」当成「写成功了」。
    // 所以本项目一律用 `run_checked`（见 `xremail_protocol_imap::command`）。
    let (server, imap, trust) = start(config_with_mail()).await;
    let mut session = login_and_examine(&imap, &trust).await;

    // 注意：返回的流借用了 session，所以要在块里用掉它。
    let is_ok = {
        let result = session.uid_store("1792658702", "+FLAGS (\\Flagged)").await;
        result.is_ok()
    };

    assert!(
        is_ok,
        "async-imap 把 NO 吞掉了——这正是要用 run_checked 的理由"
    );

    // 同一件事，安全命令层会报错。
    let err = expect_err(
        uid_store(&mut session, "1792658702", "+FLAGS (\\Flagged)").await,
        "run_checked 应当把 NO 报出来",
    );
    assert!(
        err.to_string().to_lowercase().contains("read-only"),
        "实际错误是：{err}"
    );

    server.shutdown().await;
}

#[tokio::test]
async fn run_checked_returns_empty_outcome_for_successful_but_empty_command() {
    let (server, imap, trust) = start(config_with_mail()).await;
    let mut session = login_and_select(&imap, &trust).await;

    let outcome = run_checked(&mut session, "NOOP")
        .await
        .expect("NOOP 应当成功");

    assert_eq!(outcome, CommandOutcome::Empty);
    server.shutdown().await;
}
