//! Command 层的 IPC 测试。
//!
//! 这一层测的是**整条链路的最后一段**：请求 JSON → 参数反序列化 →
//! 服务 → SQLite → 响应 JSON。
//!
//! 特别要锁住的是**线格式**：字段名是 camelCase（`displayName`、`imapPort`）。
//! 手写的 TypeScript 类型一旦和 Rust 侧漂移，前端就会静默拿到 `undefined`——
//! 用例在这里把格式钉死。
//!
//! 参考 Tauri 官方的 `tauri::test` 用法。

use serde_json::json;
use tauri::WebviewWindowBuilder;
use tauri::ipc::{CallbackFn, InvokeBody};
use tauri::test::{
    INVOKE_KEY, MockRuntime, get_ipc_response, mock_builder, mock_context, noop_assets,
};
use tauri::webview::InvokeRequest;
use xremail_shell_lib::cmd;
use xremail_shell_lib::service::{AccountService, MessageService, SyncService};
use xremail_shell_lib::state::AppState;

/// 造一个带内存库的应用与 webview。
///
/// **连接池必须在 Tauri 的运行时上创建**：sqlx 的连接池与创建它的运行时绑定，
/// 若在别的运行时（如 #[tokio::test] 的 tokio 运行时）上建池，
/// Command 在 Tauri 运行时上取连接时就会等到超时。
fn app() -> (tauri::App<MockRuntime>, tauri::WebviewWindow<MockRuntime>) {
    let pool = tauri::async_runtime::block_on(async {
        let pool = xremail_store::open_pool_in_memory(&xremail_store::PoolConfig::default())
            .await
            .expect("打开内存库失败");
        xremail_store::migrate(&pool).await.expect("应用迁移失败");
        pool
    });

    let state = AppState::new(
        std::path::PathBuf::from("/tmp/xremail-test"),
        AccountService::new(pool.clone()),
        MessageService::new(pool.clone()),
        SyncService::new(pool.clone()),
        pool,
    );

    let app = mock_builder()
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            cmd::list_accounts,
            cmd::create_account,
            cmd::delete_account,
            cmd::sync_account,
            cmd::list_folders,
            cmd::list_messages,
            cmd::open_message,
        ])
        .build(mock_context(noop_assets()))
        .expect("构建 mock 应用失败");

    let webview = WebviewWindowBuilder::new(&app, "main", Default::default())
        .build()
        .expect("创建 mock webview 失败");

    (app, webview)
}

/// 本地 origin。
///
/// Tauri 的自定义协议 origin 是**平台相关**的
/// （见 Tauri 的 `protocol::origin`）：
///
/// * Windows / Android：`http://tauri.localhost`
///   （wry 在这两个平台上用 `http(s)://{scheme}.localhost` 绕行）
/// * 其它平台：`tauri://localhost`
///
/// **写错的后果**：请求会被当成**远程来源**（`is_local = false`），
/// 于是走 ACL 检查——而 mock 应用的 ACL 是**空的**，
/// 命令会被一律拒绝：
/// `create_account not allowed. Plugin not found`。
///
/// 这个坑**在 Windows 上看不见**（那里恰好硬编码对了），
/// 只有 Linux CI 才暴露——2026-10-26 首次推送时就是这么发现的。
fn local_origin() -> &'static str {
    if cfg!(any(windows, target_os = "android")) {
        "http://tauri.localhost"
    } else {
        "tauri://localhost"
    }
}

/// 构造一个 IPC 请求。
fn request(cmd: &str, body: serde_json::Value) -> InvokeRequest {
    InvokeRequest {
        cmd: cmd.to_string(),
        callback: CallbackFn(0),
        error: CallbackFn(1),
        url: local_origin().parse().expect("URL 非法"),
        body: InvokeBody::Json(body),
        headers: Default::default(),
        invoke_key: INVOKE_KEY.to_string(),
    }
}

/// 一份合法的 `create_account` 请求体（camelCase）。
fn valid_body(email: &str) -> serde_json::Value {
    json!({
        "email": email,
        "provider": "netease_163",
        "displayName": "我的邮箱",
        "imapHost": "imap.163.com",
        "imapPort": 993,
        "imapTls": "implicit",
        "smtpHost": "smtp.163.com",
        "smtpPort": 465,
        "smtpTls": "implicit",
        "smtpAuth": "login",
        "username": email,
        "syncMode": "poll",
        "pollIntervalSecs": 300
    })
}

#[test]
fn list_accounts_returns_empty_array_on_a_fresh_database() {
    let (_app, webview) = app();

    let body =
        get_ipc_response(&webview, request("list_accounts", json!({}))).expect("命令应当成功");
    let value: serde_json::Value = body.deserialize().expect("反序列化失败");

    assert_eq!(value, json!([]));
}

#[test]
fn create_account_persists_and_appears_in_the_list() {
    let (_app, webview) = app();

    get_ipc_response(
        &webview,
        request(
            "create_account",
            json!({ "request": valid_body("user@163.com") }),
        ),
    )
    .expect("创建应当成功");

    let body =
        get_ipc_response(&webview, request("list_accounts", json!({}))).expect("命令应当成功");
    let value: serde_json::Value = body.deserialize().expect("反序列化失败");

    assert_eq!(value.as_array().expect("应为数组").len(), 1);
    assert_eq!(value[0]["email"], "user@163.com");
}

#[test]
fn account_uses_camel_case_field_names_on_the_wire() {
    // 这一条锁死线格式：前端手写的 TS 类型依赖这些名字。
    let (_app, webview) = app();

    get_ipc_response(
        &webview,
        request(
            "create_account",
            json!({ "request": valid_body("user@163.com") }),
        ),
    )
    .expect("创建应当成功");

    let body =
        get_ipc_response(&webview, request("list_accounts", json!({}))).expect("命令应当成功");
    let value: serde_json::Value = body.deserialize().expect("反序列化失败");
    let account = &value[0];

    // camelCase，不是 snake_case。
    assert!(account.get("displayName").is_some(), "缺少 displayName");
    assert!(
        account.get("pollIntervalSecs").is_some(),
        "缺少 pollIntervalSecs"
    );
    assert!(account.get("secretRef").is_some(), "缺少 secretRef");
    assert!(account.get("syncMode").is_some(), "缺少 syncMode");
    assert!(account.get("authKind").is_some(), "缺少 authKind");
    // 不应出现 snake_case 版本。
    assert!(
        account.get("display_name").is_none(),
        "出现了 snake_case 字段"
    );
    assert!(account.get("poll_interval_secs").is_none());

    // 嵌套对象同样是 camelCase。
    assert_eq!(account["server"]["imapHost"], "imap.163.com");
    assert_eq!(account["server"]["imapPort"], 993);
    assert_eq!(account["server"]["smtpAuth"], "login");
    assert!(account["server"].get("imap_host").is_none());
}

#[test]
fn create_account_returns_the_created_account() {
    let (_app, webview) = app();

    let body = get_ipc_response(
        &webview,
        request(
            "create_account",
            json!({ "request": valid_body("user@qq.com") }),
        ),
    )
    .expect("创建应当成功");
    let value: serde_json::Value = body.deserialize().expect("反序列化失败");

    assert_eq!(value["email"], "user@qq.com");
    assert_eq!(value["provider"], "netease_163");
    assert_eq!(value["server"]["imapPort"], 993);
    assert!(!value["id"].as_str().expect("应有 id").is_empty());
    assert!(
        value["secretRef"].is_null(),
        "P0 没有平台钥匙串，凭据引用应为空"
    );
}

#[test]
fn create_account_rejects_an_invalid_request() {
    let (_app, webview) = app();

    let mut bad = valid_body("user@163.com");
    bad["email"] = json!("not-an-email");

    let err = get_ipc_response(
        &webview,
        request("create_account", json!({ "request": bad })),
    )
    .expect_err("非法邮箱应被拒绝");

    assert!(
        err.to_string().contains("邮箱"),
        "错误信息应说明是邮箱的问题，实际是 {err}"
    );
}

#[test]
fn create_account_rejects_a_missing_required_field() {
    let (_app, webview) = app();

    let mut incomplete = valid_body("user@163.com");
    incomplete
        .as_object_mut()
        .expect("应为对象")
        .remove("imapHost");

    let err = get_ipc_response(
        &webview,
        request("create_account", json!({ "request": incomplete })),
    )
    .expect_err("缺字段应被拒绝");

    assert!(!err.to_string().is_empty());
}

#[test]
fn delete_account_removes_it() {
    let (_app, webview) = app();

    let body = get_ipc_response(
        &webview,
        request(
            "create_account",
            json!({ "request": valid_body("user@163.com") }),
        ),
    )
    .expect("创建应当成功");
    let value: serde_json::Value = body.deserialize().expect("反序列化失败");
    let id = value["id"].as_str().expect("应有 id").to_string();

    let deleted = get_ipc_response(&webview, request("delete_account", json!({ "id": id })))
        .expect("删除命令应当成功");
    let deleted: serde_json::Value = deleted.deserialize().expect("反序列化失败");
    assert_eq!(deleted, json!(true));

    let body =
        get_ipc_response(&webview, request("list_accounts", json!({}))).expect("命令应当成功");
    let value: serde_json::Value = body.deserialize().expect("反序列化失败");
    assert_eq!(value, json!([]));
}

#[test]
fn deleting_a_missing_account_reports_false() {
    let (_app, webview) = app();

    let body = get_ipc_response(
        &webview,
        request("delete_account", json!({ "id": "no-such-id" })),
    )
    .expect("命令应当成功");
    let value: serde_json::Value = body.deserialize().expect("反序列化失败");

    assert_eq!(value, json!(false));
}

// ---------------------------------------------------------------- 邮件查询

#[test]
fn list_folders_returns_empty_array_before_any_sync() {
    let (_app, webview) = app();

    let body = get_ipc_response(
        &webview,
        request("list_folders", json!({ "accountId": "acct-1" })),
    )
    .expect("命令应当成功");
    let value: serde_json::Value = body.deserialize().expect("反序列化失败");

    assert_eq!(value, json!([]));
}

#[test]
fn list_messages_returns_empty_array_for_unknown_folder() {
    let (_app, webview) = app();

    let body = get_ipc_response(
        &webview,
        request(
            "list_messages",
            json!({ "accountId": "acct-1", "folderId": "没有这个文件夹" }),
        ),
    )
    .expect("命令应当成功");
    let value: serde_json::Value = body.deserialize().expect("反序列化失败");

    assert_eq!(value, json!([]));
}

#[test]
fn open_message_returns_null_when_body_is_not_cached() {
    // 本地没有正文时返回 null（前端据此按需去拉），
    // **不是**报错——「还没拉」和「拉失败」是两回事。
    let (_app, webview) = app();

    let body = get_ipc_response(
        &webview,
        request(
            "open_message",
            json!({ "accountId": "acct-1", "folderId": "f-1", "uid": 1 }),
        ),
    )
    .expect("命令应当成功");
    let value: serde_json::Value = body.deserialize().expect("反序列化失败");

    assert_eq!(value, json!(null));
}

#[test]
fn sync_account_rejects_an_unknown_account() {
    let (_app, webview) = app();

    let err = get_ipc_response(
        &webview,
        request(
            "sync_account",
            json!({ "accountId": "不存在的账户", "credential": "x" }),
        ),
    )
    .expect_err("账户不存在应当报错");

    assert!(err.to_string().contains("不存在"), "实际错误是：{err}");
}
