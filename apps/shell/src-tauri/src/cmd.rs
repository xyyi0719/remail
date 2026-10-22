//! Tauri Command 层。
//!
//! 约定：
//! * 只做 **参数校验 → 调用领域服务 → 返回 DTO**；
//! * 不写业务逻辑（业务逻辑属于 `src/service/`）；
//! * **状态提交后才发事件**（见 [ARCHITECTURE §1.6](../../../../docs/ARCHITECTURE.md)）。

use serde::Serialize;
use tauri::{AppHandle, Emitter, Runtime, State};
use xremail_api::NewAccountRequest;
use xremail_core::AppEvent;
use xremail_core::model::Account;

use crate::state::AppState;

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
pub fn app_info(state: State<'_, AppState>) -> AppInfo {
    AppInfo {
        name: "Xremail",
        version: env!("CARGO_PKG_VERSION"),
        platform: std::env::consts::OS,
        data_dir: Some(state.data_dir.display().to_string()),
    }
}

/// 列出已配置的账户。
///
/// 数据来自 SQLite，不是硬编码的空列表。
#[tauri::command]
pub async fn list_accounts(state: State<'_, AppState>) -> Result<Vec<Account>, String> {
    state.accounts.list().await.map_err(|e| e.to_string())
}

/// 新增一个账户。
///
/// 凭据（P0 尚无平台钥匙串）不在这条链路上，详见
/// [`xremail_api::NewAccountRequest`]。
#[tauri::command]
pub async fn create_account<R: Runtime>(
    state: State<'_, AppState>,
    app: AppHandle<R>,
    request: NewAccountRequest,
) -> Result<Account, String> {
    // 服务层先校验再落库：校验失败时库里不会留下半条记录。
    let account = state
        .accounts
        .create(request)
        .await
        .map_err(|e| e.to_string())?;

    // 提交成功后才发事件。
    let event = AppEvent::AccountAdded {
        account: account.clone(),
    };
    app.emit(event.name(), &event).map_err(|e| e.to_string())?;

    Ok(account)
}

/// 删除一个账户。返回是否真的删掉了。
#[tauri::command]
pub async fn delete_account<R: Runtime>(
    state: State<'_, AppState>,
    app: AppHandle<R>,
    id: String,
) -> Result<bool, String> {
    let deleted = state
        .accounts
        .delete(&id)
        .await
        .map_err(|e| e.to_string())?;

    if deleted {
        let event = AppEvent::AccountRemoved { account_id: id };
        app.emit(event.name(), &event).map_err(|e| e.to_string())?;
    }

    Ok(deleted)
}
