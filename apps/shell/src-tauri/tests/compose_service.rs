//! 发信服务层的集成测试。
//!
//! 测的是「落草稿 → 冻结字节 → 提交 → 记结局 → 回读」这条链路，
//! 重点锁住**三种结局必须分清**这件事：
//!
//! * `sent`——服务器回了 250，信确实出去了；
//! * `uncertain`——命令发出去了、没拿到应答，**不知道**出去没有；
//! * `failed`——明确没发出去。
//!
//! 这三种揉成一团的后果很具体：`uncertain` 被当成 `failed` 就会重复投递，
//! `failed` 被当成 `sent` 就是把没发出去的信报成发出去了。
//!
//! 不依赖真实邮箱、凭据、网络或本机绝对路径
//! （见 [TESTING_STRATEGY §5.2](../../../../docs/TESTING_STRATEGY.md)）：
//! 服务器是 `xremail-testkit` 里的假 SMTP 服务器。

use sqlx::SqlitePool;
use xremail_api::{NewAccountRequest, SecretValue};
use xremail_core::model::{ProviderKind, SendState, SmtpAuth, SyncMode, TlsMode};
use xremail_protocol_smtp::TlsTrust;
use xremail_shell_lib::service::account::AccountService;
use xremail_shell_lib::service::compose::{ComposeService, DraftInput, SendResultState};
use xremail_store::dao::send_queue as send_queue_dao;
use xremail_testkit::smtp::{FakeSmtpServer, SmtpServerConfig};

/// 假服务器的登录凭据。
const USER: &str = "user@163.com";
const PASSWORD: &str = "smtp-auth-code";

/// 一套「内存库 + 服务 + 账户」。
struct Fixture {
    service: ComposeService,
    pool: SqlitePool,
    account_id: String,
}

impl Fixture {
    /// 造一个指向假 SMTP 服务器的套件（信任它的自签 CA）。
    async fn with_server(config: SmtpServerConfig) -> (Self, FakeSmtpServer) {
        let server = FakeSmtpServer::start(config)
            .await
            .expect("假 SMTP 服务器启动失败");
        let pool = xremail_store::open_pool_in_memory(&xremail_store::PoolConfig::default())
            .await
            .expect("打开内存库失败");
        xremail_store::migrate(&pool).await.expect("应用迁移失败");

        let accounts = AccountService::new(pool.clone());
        let account_id = accounts
            .create(NewAccountRequest {
                email: USER.to_string(),
                provider: ProviderKind::Netease163,
                display_name: None,
                imap_host: server.host().to_string(),
                imap_port: server.port(),
                imap_tls: TlsMode::Implicit,
                smtp_host: server.host().to_string(),
                smtp_port: server.port(),
                smtp_tls: TlsMode::Implicit,
                smtp_auth: SmtpAuth::Login,
                username: USER.to_string(),
                sync_mode: SyncMode::Poll,
                poll_interval_secs: 300,
            })
            .await
            .expect("建账户失败")
            .id;

        let service = ComposeService::with_trust(
            pool.clone(),
            accounts,
            TlsTrust::Roots(server.root_store()),
        );
        (
            Self {
                service,
                pool,
                account_id,
            },
            server,
        )
    }

    /// 造一套不连任何服务器的套件（只测存草稿）。
    async fn offline() -> Self {
        let pool = xremail_store::open_pool_in_memory(&xremail_store::PoolConfig::default())
            .await
            .expect("打开内存库失败");
        xremail_store::migrate(&pool).await.expect("应用迁移失败");
        let accounts = AccountService::new(pool.clone());
        let account_id = accounts
            .create(NewAccountRequest {
                email: USER.to_string(),
                provider: ProviderKind::Netease163,
                display_name: None,
                imap_host: "imap.163.com".to_string(),
                imap_port: 993,
                imap_tls: TlsMode::Implicit,
                smtp_host: "smtp.163.com".to_string(),
                smtp_port: 465,
                smtp_tls: TlsMode::Implicit,
                smtp_auth: SmtpAuth::Login,
                username: USER.to_string(),
                sync_mode: SyncMode::Poll,
                poll_interval_secs: 300,
            })
            .await
            .expect("建账户失败")
            .id;
        let service = ComposeService::new(pool.clone(), accounts);
        Self {
            service,
            pool,
            account_id,
        }
    }

    /// 这套件里的那封信在库里的样子。
    async fn stored(&self) -> xremail_core::model::SendQueueEntry {
        let rows = send_queue_dao::list_entries(&self.pool, &self.account_id)
            .await
            .unwrap();
        assert_eq!(rows.len(), 1, "应当只有一封信");
        rows.into_iter().next().unwrap()
    }
}

/// 一份草稿。
fn draft(account_id: &str) -> DraftInput {
    DraftInput {
        account_id: account_id.to_string(),
        id: None,
        to: vec!["friend@example.com".to_string()],
        cc: vec![],
        bcc: vec![],
        subject: "你好".to_string(),
        body_text: "AUTOTEST-BODY-MARKER 这封信是自动测试发出来的。".to_string(),
        body_html: None,
        attachments: vec![],
    }
}

fn credential() -> SecretValue {
    SecretValue::from_str_lossless(PASSWORD)
}

/// 隐式 TLS（465 那种）+ PLAIN/LOGIN 认证。
fn secure_config() -> SmtpServerConfig {
    SmtpServerConfig::implicit_tls().with_auth(&["PLAIN", "LOGIN"], USER, PASSWORD)
}

#[tokio::test]
async fn draft_is_saved_and_read_back_from_the_database() {
    let fixture = Fixture::offline().await;

    let saved = fixture
        .service
        .save_draft(&draft(&fixture.account_id))
        .await
        .unwrap();
    assert_eq!(saved.state, "draft");
    assert_eq!(saved.subject, "你好");
    assert_eq!(saved.to, vec!["friend@example.com".to_string()]);

    let row = fixture.stored().await;
    assert_eq!(row.state, SendState::Draft);
    assert!(
        row.message_bytes.is_none(),
        "草稿还没发送，不该有冻结的字节"
    );
}

#[tokio::test]
async fn accepted_message_is_reported_as_sent_and_reaches_the_server() {
    let (fixture, server) = Fixture::with_server(secure_config()).await;

    let result = fixture
        .service
        .send(&draft(&fixture.account_id), &credential())
        .await
        .unwrap();

    assert_eq!(result.state, SendResultState::Sent);
    assert!(!result.message_id.is_empty());

    // 服务器确实收到了这一封。
    let received = server.last_message().expect("服务器没收到信");
    let text = String::from_utf8_lossy(&received);
    assert!(text.contains("friend@example.com"), "收件人没进 RCPT TO");
    assert!(text.contains("AUTOTEST-BODY-MARKER"), "正文没送到");
    assert!(
        text.to_lowercase()
            .contains("content-transfer-encoding: quoted-printable"),
        "正文按 quoted-printable 编码（服务器支不支持 8BITMIME 要等 EHLO 后才知道，\
         而字节必须在发送前就冻结，所以选到处都能用的那一种）"
    );

    // 库里记成 succeeded，字节与服务器收到的一致（重发发的就是同一份）。
    let row = fixture.stored().await;
    assert_eq!(row.state, SendState::Succeeded);
    assert_eq!(row.message_bytes.as_deref(), Some(received.as_slice()));

    server.shutdown().await;
}

#[tokio::test]
async fn missing_server_reply_is_uncertain_not_retryable() {
    // 收完 DATA 就不回应答 → 服务器手里可能已经有这封信了。
    let (fixture, server) = Fixture::with_server(secure_config().dropping_data_response()).await;

    let result = fixture
        .service
        .send(&draft(&fixture.account_id), &credential())
        .await
        .unwrap();

    assert_eq!(
        result.state,
        SendResultState::Uncertain,
        "命令已经发出去了却没拿到应答，只能是「不确定」"
    );

    let row = fixture.stored().await;
    assert_eq!(row.state, SendState::Uncertain);
    assert!(
        !row.state.is_auto_retryable(),
        "不确定的信**不能**自动重试——服务器可能已经收下了它"
    );

    server.shutdown().await;
}

#[tokio::test]
async fn permanent_rejection_is_failed_and_not_retried_automatically() {
    let (fixture, server) =
        Fixture::with_server(secure_config().rejecting_recipient("friend@example.com", 550)).await;

    let result = fixture
        .service
        .send(&draft(&fixture.account_id), &credential())
        .await
        .unwrap();

    assert_eq!(result.state, SendResultState::Failed);

    let row = fixture.stored().await;
    assert_eq!(row.state, SendState::Failed);
    assert!(
        !row.state.is_auto_retryable(),
        "5xx 是永久拒收，重发多少次都一样"
    );

    server.shutdown().await;
}

#[tokio::test]
async fn temporary_rejection_is_recorded_as_retryable() {
    let (fixture, server) =
        Fixture::with_server(secure_config().rejecting_recipient("friend@example.com", 450)).await;

    let result = fixture
        .service
        .send(&draft(&fixture.account_id), &credential())
        .await
        .unwrap();

    assert_eq!(result.state, SendResultState::Failed);

    let row = fixture.stored().await;
    assert_eq!(
        row.state,
        SendState::Retryable,
        "4xx 表示服务器这次没收下这封信，重发是安全的"
    );

    server.shutdown().await;
}

#[tokio::test]
async fn unreachable_server_does_not_leave_the_row_running() {
    // 起一个服务器再关掉，拿到一个没人监听的端口。
    let server = FakeSmtpServer::start(secure_config())
        .await
        .expect("假 SMTP 服务器启动失败");
    let port = server.port();
    let host = server.host().to_string();
    server.shutdown().await;

    let pool = xremail_store::open_pool_in_memory(&xremail_store::PoolConfig::default())
        .await
        .unwrap();
    xremail_store::migrate(&pool).await.unwrap();
    let accounts = AccountService::new(pool.clone());
    let account_id = accounts
        .create(NewAccountRequest {
            email: USER.to_string(),
            provider: ProviderKind::Netease163,
            display_name: None,
            imap_host: host.clone(),
            imap_port: port,
            imap_tls: TlsMode::Implicit,
            smtp_host: host,
            smtp_port: port,
            smtp_tls: TlsMode::Implicit,
            smtp_auth: SmtpAuth::Login,
            username: USER.to_string(),
            sync_mode: SyncMode::Poll,
            poll_interval_secs: 300,
        })
        .await
        .unwrap()
        .id;

    let service = ComposeService::new(pool.clone(), accounts);
    let result = service
        .send(&draft(&account_id), &credential())
        .await
        .unwrap();

    assert_eq!(result.state, SendResultState::Failed);

    let rows = send_queue_dao::list_entries(&pool, &account_id)
        .await
        .unwrap();
    assert_eq!(
        rows[0].state,
        SendState::Retryable,
        "连都没连上 → 这封信肯定没发出去 → 记成可重试，不能卡在 running"
    );
}

#[tokio::test]
async fn bcc_recipient_is_only_in_the_envelope() {
    let (fixture, server) = Fixture::with_server(secure_config()).await;

    let mut input = draft(&fixture.account_id);
    input.bcc = vec!["secret@example.com".to_string()];

    fixture.service.send(&input, &credential()).await.unwrap();

    let commands = server.command_lines().join("\n");
    assert!(
        commands.contains("secret@example.com"),
        "密送收件人必须进信封（RCPT TO）"
    );

    let received = String::from_utf8_lossy(&server.last_message().unwrap()).to_string();
    assert!(
        !received.to_lowercase().contains("bcc:"),
        "密送不能出现在邮件头里，否则收件人互相看得见"
    );

    server.shutdown().await;
}

/// 发送结果的线格式：字段名 camelCase，`state` 是三个小写单词之一。
///
/// 前端 `types.ts` 写死了 `"sent" | "uncertain" | "failed"`。
/// Rust 侧改一下枚举的序列化名，前端就会在**运行时**才发现不对——
/// 而 `state` 是前端决定「能不能重发」的依据，错了就是重复发信。
#[test]
fn send_result_serializes_the_way_the_frontend_expects() {
    use xremail_shell_lib::service::compose::SendResult;

    for (state, expected) in [
        (SendResultState::Sent, "sent"),
        (SendResultState::Uncertain, "uncertain"),
        (SendResultState::Failed, "failed"),
    ] {
        let value = serde_json::to_value(SendResult {
            message_id: "<m1@example.com>".to_string(),
            state,
            detail: "说明".to_string(),
        })
        .expect("序列化失败");

        assert_eq!(value["state"], expected);
        assert_eq!(value["messageId"], "<m1@example.com>");
        assert_eq!(value["detail"], "说明");
        assert!(
            !value.as_object().unwrap().contains_key("message_id"),
            "出现了 snake_case 字段名，前端读不到"
        );
    }
}
