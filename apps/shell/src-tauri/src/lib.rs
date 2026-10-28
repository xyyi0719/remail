//! Xremail 应用外壳（Tauri）。
//!
//! 职责边界（见 [ARCHITECTURE §9](../../../../docs/ARCHITECTURE.md)）：
//! * Command 只做 **参数校验 → 调用领域服务 → 返回 DTO**，不写业务逻辑。
//! * 跨线程的工作（IMAP 拉取、发信等）投递给 `xremail-sync` 的 Actor。
//! * 平台相关能力（托盘 / 通知 / 前台服务）收敛到 `platform` 模块。
//!
//! ## 启动顺序
//!
//! 1. 解析数据目录（桌面 `%APPDATA%/com.xremail.app`，Android app 内部存储）
//! 2. 打开 SQLite 连接池并**应用迁移**
//! 3. 注入 `AppState`，注册 Command
//!
//! 迁移失败会让应用**启动失败**并报错，而不是降级成一个空库或旧库：
//! 静默降级会让人以为数据还在，实际却是另一份（ARCHITECTURE §9）。

pub mod cmd;
pub mod secrets;
pub mod service;
pub mod state;

use tauri::Manager;

use crate::service::{AccountService, MessageService, SyncService};
use crate::state::AppState;

/// 数据库文件名（位于数据目录下）。
pub const DB_FILE_NAME: &str = "xremail.db";

/// 应用入口。桌面与移动共用。
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    init_tracing();

    let builder = tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init());

    // 单实例与开机自启只有桌面端实现，这两个插件在 Android 上不提供 API。
    #[cfg(desktop)]
    let builder = builder
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            // 第二次启动时聚焦已有窗口，避免多开导致服务端连接数超限。
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ));

    builder
        .setup(|app| {
            let state = init_storage(app.handle())?;
            app.manage(state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            cmd::app_info,
            cmd::list_accounts,
            cmd::create_account,
            cmd::delete_account,
            cmd::sync_account,
            cmd::list_folders,
            cmd::list_messages,
            cmd::open_message,
            cmd::save_credential,
            cmd::forget_credential,
            cmd::set_message_flag,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Xremail application");
}

/// 解析数据目录、打开数据库、应用迁移。
fn init_storage(app: &tauri::AppHandle) -> Result<AppState, Box<dyn std::error::Error>> {
    // `app_data_dir()` 在桌面端是 `%APPDATA%/<identifier>`，
    // 在 Android 上是 app 的内部存储目录。
    let data_dir = app.path().app_data_dir()?;
    std::fs::create_dir_all(&data_dir)?;

    let db_path = data_dir.join(DB_FILE_NAME);

    // 打开连接池并应用迁移。`block_on` 用的是 Tauri 自己的全局运行时；
    // 此时运行时已就绪，迁移是纯 IO，不会回调主线程。
    let pool = tauri::async_runtime::block_on(async {
        let pool =
            xremail_store::open_pool_path(&db_path, &xremail_store::PoolConfig::default()).await?;
        xremail_store::migrate(&pool).await?;
        Ok::<_, xremail_core::AppError>(pool)
    })?;

    Ok(AppState::new(
        data_dir,
        AccountService::new(pool.clone()),
        MessageService::new(pool.clone()),
        SyncService::new(pool.clone()),
        pool,
    ))
}

/// 初始化结构化日志。
///
/// 默认级别 `info`；可用 `RUST_LOG=xremail_shell=debug,xremail_core=debug` 覆盖。
fn init_tracing() {
    use tracing_subscriber::EnvFilter;

    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(true)
        .init();
}
