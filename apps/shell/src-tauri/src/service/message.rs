//! 邮件应用服务。
//!
//! 把「库里的邮件」变成「前端要的 DTO」，并按需拉取正文。
//! 中间不出现 Tauri 类型。

use serde::Serialize;
use sqlx::SqlitePool;
use xremail_core::AppResult;
use xremail_core::model::{Folder, Message, MessageBody};
use xremail_store::dao::folder as folder_dao;
use xremail_store::dao::message as message_dao;

/// 一个文件夹（给前端看的形状）。
///
/// **不要**直接返回领域模型 [`Folder`]：它的 `remote_name` 是
/// **线路上的原样**（modified UTF-7，如 `&XfJT0ZAB-`），
/// 那是发命令时要用的，不是给人看的。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderDto {
    /// 本地 ID。
    pub id: String,
    /// 显示名（已解码的 modified UTF-7，如「草稿箱」）。
    pub name: String,
    /// 线路上的原样（发命令时用）。
    pub raw_name: String,
    /// 层级分隔符。
    pub delimiter: Option<String>,
    /// 服务器当前的 epoch（`None` = 还没同步过）。
    pub uidvalidity: Option<i64>,
    /// 已同步到的最大 UID。
    pub last_uid: Option<i64>,
    /// 上次同步时间（epoch 毫秒）。
    pub last_sync_at: Option<i64>,
}

impl FolderDto {
    /// 把领域模型变成 DTO：显示名解码，`local_name` 优先。
    pub fn from_domain(folder: &Folder) -> Self {
        let decoded = xremail_protocol_imap::decode_utf7(&folder.remote_name)
            .unwrap_or_else(|_| folder.remote_name.clone());
        Self {
            id: folder.id.clone(),
            name: folder.local_name.clone().unwrap_or(decoded),
            raw_name: folder.remote_name.clone(),
            delimiter: folder.delimiter.clone(),
            uidvalidity: folder.uidvalidity,
            last_uid: folder.last_uid,
            last_sync_at: folder.last_sync_at,
        }
    }
}

/// 邮件服务。
#[derive(Debug, Clone)]
pub struct MessageService {
    pool: SqlitePool,
}

impl MessageService {
    /// 用连接池构造。
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// 列出某账户的全部文件夹（**显示名已解码**）。
    pub async fn list_folders(&self, account_id: &str) -> AppResult<Vec<FolderDto>> {
        let folders = folder_dao::list_folders(&self.pool, account_id).await?;
        Ok(folders.iter().map(FolderDto::from_domain).collect())
    }

    /// 列出某文件夹里的邮件（按 UID 升序）。
    pub async fn list_messages(
        &self,
        account_id: &str,
        folder_id: &str,
    ) -> AppResult<Vec<Message>> {
        message_dao::list_messages(&self.pool, account_id, folder_id).await
    }

    /// 读一封邮件的正文（**已缓存**的）。
    ///
    /// 返回 `None` 表示本地还没有正文——需要按需去服务器拉。
    pub async fn get_body(
        &self,
        account_id: &str,
        folder_id: &str,
        uid: i64,
    ) -> AppResult<Option<MessageBody>> {
        let messages = message_dao::list_messages(&self.pool, account_id, folder_id).await?;
        let Some(message) = messages.into_iter().find(|message| message.uid == uid) else {
            return Ok(None);
        };
        message_dao::get_message_body(&self.pool, &message.id).await
    }

    /// 按远端身份取一封邮件。
    pub async fn get_message(
        &self,
        account_id: &str,
        folder_id: &str,
        uid: i64,
    ) -> AppResult<Option<Message>> {
        let messages = message_dao::list_messages(&self.pool, account_id, folder_id).await?;
        Ok(messages.into_iter().find(|message| message.uid == uid))
    }
}
