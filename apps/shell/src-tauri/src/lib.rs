//! Xremail 应用外壳（Tauri）。
//!
//! 职责边界（见 [ARCHITECTURE §9](../../../../docs/ARCHITECTURE.md)）：
//! * Command 只做 **参数校验 → 调用领域服务 → 返回 DTO**，不写业务逻辑。
//! * 跨线程的工作（IMAP 拉取、发信等）投递给 `xremail-sync` 的 Actor。
//! * 平台相关能力（托盘 / 通知 / 前台服务）收敛到 `platform` 模块。

pub mod cmd;
pub mod state;

use tauri::Manager;

/// 应用入口。桌面与移动共用。
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    init_tracing();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
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
        ))
        .manage(state::AppState::default())
        .invoke_handler(tauri::generate_handler![cmd::app_info, cmd::list_accounts])
        .run(tauri::generate_context!())
        .expect("error while running Xremail application");
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
