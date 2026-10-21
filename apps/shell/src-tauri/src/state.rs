//! 应用共享状态。
//!
//! 目前只承载启动期信息；数据库句柄、账户 Actor 注册表、事件发送器
//! 会在 P1/P2 按需加入（见 [ARCHITECTURE §3](../../../../docs/ARCHITECTURE.md)）。

/// 由 Tauri `manage` 注入的全局状态。
#[derive(Debug, Default)]
pub struct AppState {
    /// 数据目录（数据库、附件、日志的根目录）。
    ///
    /// P0 阶段先留空，P1 接入 `xremail-store` 时按平台解析：
    /// Windows → `%APPDATA%/Xremail`，Android → app 内部存储。
    pub data_dir: Option<std::path::PathBuf>,
}
