//! Tauri Command 层。
//!
//! 约定：
//! * 只做 **参数校验 → 调用领域服务 → 返回 DTO**；
//! * 不写业务逻辑（业务逻辑属于 `src/service/`）；
//! * **状态提交后才发事件**（见 [ARCHITECTURE §1.6](../../../../docs/ARCHITECTURE.md)）。

use serde::Serialize;
use tauri::{AppHandle, Emitter, Runtime, State};
use xremail_api::NewAccountRequest;
use xremail_api::SecretValue;
use xremail_core::AppEvent;
use xremail_core::model::{Account, Message, MessageBody, SyncState};

use crate::service::message::FolderDto;
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

/// 同步结果（给前端看的）。
///
/// Command 层定义自己的响应形状，不让 `xremail-sync` 依赖 serde。
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncResult {
    /// 同步了几个文件夹。
    pub folders: usize,
    /// 新拉到多少封。
    pub added: usize,
    /// 服务器已删、清掉多少封。
    pub removed: usize,
}

/// 同步一个账户的全部文件夹。
///
/// `credential` 是 IMAP 授权码：它只驻留在**内存**里（本次会话），
/// **不落盘**——平台钥匙串适配属 P4（见 [SECURITY §2.1](../../../../docs/SECURITY.md)）。
#[tauri::command]
pub async fn sync_account<R: Runtime>(
    state: State<'_, AppState>,
    app: AppHandle<R>,
    account_id: String,
    credential: String,
) -> Result<SyncResult, String> {
    let account = state
        .accounts
        .get(&account_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("账户 {account_id} 不存在"))?;

    // 凭据存进内存，供后续的「按需拉正文」用。
    state.remember_credential(&account_id, SecretValue::new(credential.into_bytes()));

    // 先报「连接中」。
    let event = AppEvent::SyncStateChanged {
        account_id: account_id.clone(),
        folder_id: None,
        state: SyncState::Connecting,
    };
    let _ = app.emit(event.name(), &event);

    let credential = state
        .credential(&account_id)
        .ok_or_else(|| "凭据不在内存里".to_string())?;

    let result = state
        .sync
        .sync_account(&account, &credential, current_time_ms())
        .await;

    let event = AppEvent::SyncStateChanged {
        account_id: account_id.clone(),
        folder_id: None,
        state: match &result {
            Ok(_) => SyncState::Idle,
            Err(_) => SyncState::Failed,
        },
    };
    let _ = app.emit(event.name(), &event);

    result
        .map(|summary| SyncResult {
            folders: summary.folders,
            added: summary.added,
            removed: summary.removed,
        })
        .map_err(|e| e.to_string())
}

/// 列出某账户的全部文件夹。
#[tauri::command]
pub async fn list_folders(
    state: State<'_, AppState>,
    account_id: String,
) -> Result<Vec<FolderDto>, String> {
    state
        .messages
        .list_folders(&account_id)
        .await
        .map_err(|e| e.to_string())
}

/// 列出某文件夹里的邮件。
#[tauri::command]
pub async fn list_messages(
    state: State<'_, AppState>,
    account_id: String,
    folder_id: String,
) -> Result<Vec<Message>, String> {
    state
        .messages
        .list_messages(&account_id, &folder_id)
        .await
        .map_err(|e| e.to_string())
}

/// 打开一封邮件：本地有正文就直接返回，没有就**按需**去服务器拉。
///
/// 拉正文用 `BODY.PEEK[]`——**不置** `\Seen`
/// （「看正文」不等于「标记已读」）。
#[tauri::command]
pub async fn open_message(
    state: State<'_, AppState>,
    account_id: String,
    folder_id: String,
    uid: i64,
) -> Result<Option<MessageBody>, String> {
    // 先在本地找这封邮件。
    //
    // 本地**根本没有**它 → 返回 None（「没这封邮件」与「拉失败」是两回事）。
    let Some(_message) = state
        .messages
        .get_message(&account_id, &folder_id, uid)
        .await
        .map_err(|e| e.to_string())?
    else {
        return Ok(None);
    };

    // 本地有正文 → 直接返回。
    if let Some(body) = state
        .messages
        .get_body(&account_id, &folder_id, uid)
        .await
        .map_err(|e| e.to_string())?
    {
        return Ok(Some(body));
    }

    // 有邮件但没正文 → **按需**去服务器拉。需要凭据与连接。
    let credential = state
        .credential(&account_id)
        .ok_or_else(|| "需要先同步一次（凭据只在内存里）".to_string())?;

    let account = state
        .accounts
        .get(&account_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("账户 {account_id} 不存在"))?;

    let password = credential
        .as_str()
        .ok_or_else(|| "授权码不是合法的 UTF-8 文本".to_string())?
        .to_string();

    let mut config = xremail_protocol_imap::ImapConfig::imaps(&account.server.imap_host);
    config.port = account.server.imap_port;

    let (mut session, _caps, _tls) = xremail_protocol_imap::connect_and_login_with(
        &config,
        &xremail_protocol_imap::TlsTrust::SystemRoots,
        &account.username,
        &password,
    )
    .await
    .map_err(|e| format!("连接或登录失败: {e}"))?;

    // 需要 epoch 才能定位邮件。
    let folder = state
        .messages
        .list_folders(&account_id)
        .await
        .map_err(|e| e.to_string())?
        .into_iter()
        .find(|folder| folder.id == folder_id)
        .ok_or_else(|| "文件夹不存在".to_string())?;
    let uidvalidity = folder
        .uidvalidity
        .ok_or_else(|| "该文件夹还没同步过，不知道 epoch".to_string())?;

    xremail_sync::fetch_body(
        state.pool(),
        &mut session,
        &account_id,
        &folder_id,
        &folder.raw_name,
        uid,
        uidvalidity,
    )
    .await
    .map_err(|e| e.to_string())?;

    state
        .messages
        .get_body(&account_id, &folder_id, uid)
        .await
        .map_err(|e| e.to_string())
}

/// 当前时间（epoch 毫秒）。
fn current_time_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};

    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
