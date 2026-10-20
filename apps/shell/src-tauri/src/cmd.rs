//! Tauri Command 层。
//!
//! 约定：
//! * 只做 **参数校验 → 调用领域服务 → 返回 DTO**；
//! * 不写业务逻辑（业务逻辑属于 `crates/`）；
//! * 需要跨线程的活，投递给对应 Actor 的 mailbox。

use remail_core::Account;
use serde::Serialize;

/// 应用基本信息，供前端做环境探测与"关于"页。
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppInfo {
    /// 产品名。
    pub name: &'static str,
    /// 版本号。
    pub version: &'static str,
    /// 运行平台。
    pub platform: &'static str,
    /// 数据目录（前端只用于展示，不允许写入）。
    pub data_dir: Option<String>,
}

/// 返回应用基本信息。
#[tauri::command]
pub fn app_info() -> AppInfo {
    AppInfo {
        name: "ReMail",
        version: env!("CARGO_PKG_VERSION"),
        platform: std::env::consts::OS,
        data_dir: None,
    }
}

/// 列出已配置的账户。
///
/// P0 阶段尚无持久化实现，返回空列表；P1 接入 `remail-store` 后改为真实查询。
#[tauri::command]
pub async fn list_accounts() -> Result<Vec<Account>, String> {
    Ok(Vec::new())
}
