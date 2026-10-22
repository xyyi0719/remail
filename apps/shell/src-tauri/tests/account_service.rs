//! 账户服务的集成测试。
//!
//! 覆盖 P0 的验收项「Command → 服务 → SQLite → React 真实列表」中
//! **服务 → SQLite** 这一段的真实行为：数据真的落库、真的读得回来。
//!
//! 不依赖真实邮箱、凭据、网络或本机绝对路径
//! （见 [TESTING_STRATEGY §5.2](../../../../docs/TESTING_STRATEGY.md)）。

use xremail_api::NewAccountRequest;
use xremail_core::model::{ProviderKind, SmtpAuth, SyncMode, TlsMode};
use xremail_shell_lib::service::AccountService;

/// 造一个已应用迁移的内存库与账户服务。
async fn service() -> AccountService {
    let pool = xremail_store::open_pool_in_memory(&xremail_store::PoolConfig::default())
        .await
        .expect("打开内存库失败");
    xremail_store::migrate(&pool).await.expect("应用迁移失败");
    AccountService::new(pool)
}

/// 一份合法的请求。
fn request(email: &str) -> NewAccountRequest {
    NewAccountRequest {
        email: email.to_string(),
        provider: ProviderKind::Netease163,
        display_name: Some("我的邮箱".to_string()),
        imap_host: "imap.163.com".to_string(),
        imap_port: 993,
        imap_tls: TlsMode::Implicit,
        smtp_host: "smtp.163.com".to_string(),
        smtp_port: 465,
        smtp_tls: TlsMode::Implicit,
        smtp_auth: SmtpAuth::Login,
        username: email.to_string(),
        sync_mode: SyncMode::Poll,
        poll_interval_secs: 300,
    }
}

#[tokio::test]
async fn empty_database_lists_no_accounts() {
    let service = service().await;

    let accounts = service.list().await.unwrap();

    assert!(
        accounts.is_empty(),
        "新建的库应当是空的——列表不是硬编码的空数组，而是真的查出来的"
    );
}

#[tokio::test]
async fn created_account_comes_back_from_the_database() {
    let service = service().await;

    let created = service.create(request("user@163.com")).await.unwrap();

    let listed = service.list().await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].id, created.id);
    assert_eq!(listed[0].email, "user@163.com");
    assert_eq!(listed[0].provider, ProviderKind::Netease163);
    assert_eq!(listed[0].server.imap_host, "imap.163.com");
    assert_eq!(listed[0].server.imap_port, 993);
    assert_eq!(listed[0].server.smtp_port, 465);
    assert_eq!(listed[0].server.smtp_auth, SmtpAuth::Login);
    assert_eq!(listed[0].display_name.as_deref(), Some("我的邮箱"));
}

#[tokio::test]
async fn created_account_can_be_fetched_by_id() {
    let service = service().await;

    let created = service.create(request("user@qq.com")).await.unwrap();

    let got = service.get(&created.id).await.unwrap();
    assert!(got.is_some());
    assert_eq!(got.unwrap().email, "user@qq.com");
}

#[tokio::test]
async fn missing_account_returns_none_not_error() {
    let service = service().await;

    let got = service.get("no-such-id").await.unwrap();

    assert!(got.is_none(), "「没有」不是错误");
}

#[tokio::test]
async fn multiple_accounts_are_all_listed() {
    let service = service().await;

    service.create(request("a@163.com")).await.unwrap();
    service.create(request("b@qq.com")).await.unwrap();
    service.create(request("c@wo.cn")).await.unwrap();

    let accounts = service.list().await.unwrap();

    assert_eq!(accounts.len(), 3);
}

#[tokio::test]
async fn deleting_an_account_removes_it_from_the_list() {
    let service = service().await;
    let created = service.create(request("user@163.com")).await.unwrap();

    let deleted = service.delete(&created.id).await.unwrap();

    assert!(deleted);
    assert!(service.list().await.unwrap().is_empty());
}

#[tokio::test]
async fn deleting_a_missing_account_reports_false() {
    let service = service().await;

    assert!(!service.delete("no-such-id").await.unwrap());
}

#[tokio::test]
async fn invalid_request_is_rejected_and_nothing_is_written() {
    let service = service().await;

    let mut bad = request("not-an-email");
    bad.email = "not-an-email".to_string();

    let err = service.create(bad).await.unwrap_err();

    assert!(!err.is_retryable());
    assert!(
        service.list().await.unwrap().is_empty(),
        "校验失败时库里不应留下半条记录"
    );
}

#[tokio::test]
async fn updating_an_account_keeps_its_identity() {
    let service = service().await;
    let mut created = service.create(request("user@163.com")).await.unwrap();

    created.display_name = Some("改过的名字".to_string());
    let updated = service.update(created.clone()).await.unwrap();

    assert_eq!(updated.id, created.id, "更新不应换 id");
    assert_eq!(updated.display_name.as_deref(), Some("改过的名字"));

    let listed = service.list().await.unwrap();
    assert_eq!(listed.len(), 1, "更新不应产生第二条");
    assert_eq!(listed[0].display_name.as_deref(), Some("改过的名字"));
}

#[tokio::test]
async fn account_survives_a_database_reopen() {
    // 这一条验证「真的落库」而不只是「在内存里存着」：
    // 关掉连接、重新打开同一个文件，数据还在。
    let dir = tempfile::tempdir().expect("创建临时目录失败");
    let path = dir.path().join("xremail-test.db");

    let id = {
        let pool = xremail_store::open_pool_path(&path, &xremail_store::PoolConfig::default())
            .await
            .expect("打开文件库失败");
        xremail_store::migrate(&pool).await.expect("应用迁移失败");
        let service = AccountService::new(pool);
        let created = service.create(request("user@163.com")).await.unwrap();
        created.id
    };

    let pool = xremail_store::open_pool_path(&path, &xremail_store::PoolConfig::default())
        .await
        .expect("重新打开文件库失败");
    let service = AccountService::new(pool);

    let accounts = service.list().await.unwrap();
    assert_eq!(accounts.len(), 1);
    assert_eq!(accounts[0].id, id);
}
