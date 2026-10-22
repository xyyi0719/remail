//! 文件夹 DAO。
//!
//! 对应 `folder` 表。文件夹同时承载**同步游标**与 **epoch**
//! （`uidvalidity`），与 `xremail-core` 的 `Folder` 结构一一对应，
//! 不另建 `sync_state` 表，避免出现同义重复字段
//! （见 [ARCHITECTURE §4.3](../../../../docs/ARCHITECTURE.md)）。

use sqlx::FromRow;
use sqlx::SqlitePool;
use xremail_core::AppResult;
use xremail_core::model::Folder;

use crate::error::map_sqlx;

/// `folder` 表的一行。
#[derive(Debug, Clone, FromRow)]
#[sqlx(rename_all = "snake_case")]
struct FolderRow {
    id: String,
    account_id: String,
    remote_name: String,
    remote_id: Option<String>,
    local_name: Option<String>,
    delimiter: Option<String>,
    subscribed: bool,
    selectable: bool,
    last_sync_at: Option<i64>,
    last_uid: Option<i64>,
    highest_modseq: Option<i64>,
    uidvalidity: Option<i64>,
}

/// 插入或更新一个文件夹。
///
/// 冲突键是 `(account_id, remote_name)`：同一账户下服务器侧的文件夹名唯一。
/// `id` 是本地 ID，冲突时**不**覆盖——因为 `id` 已被 `message` 的外键引用，
/// 换 id 会让已缓存的邮件失去归属。
pub async fn upsert_folder(pool: &SqlitePool, folder: &Folder) -> AppResult<()> {
    sqlx::query!(
        r#"
        INSERT INTO folder (
            id, account_id, remote_name, remote_id, local_name, delimiter,
            subscribed, selectable, last_sync_at, last_uid, highest_modseq, uidvalidity
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT (account_id, remote_name) DO UPDATE SET
            remote_id      = excluded.remote_id,
            local_name     = excluded.local_name,
            delimiter      = excluded.delimiter,
            subscribed     = excluded.subscribed,
            selectable     = excluded.selectable,
            last_sync_at   = excluded.last_sync_at,
            last_uid       = excluded.last_uid,
            highest_modseq = excluded.highest_modseq,
            uidvalidity    = excluded.uidvalidity
        "#,
        folder.id,
        folder.account_id,
        folder.remote_name,
        folder.remote_id,
        folder.local_name,
        folder.delimiter,
        folder.subscribed,
        folder.selectable,
        folder.last_sync_at,
        folder.last_uid,
        folder.highest_modseq,
        folder.uidvalidity,
    )
    .execute(pool)
    .await
    .map_err(|e| map_sqlx(e, "upsert_folder"))?;

    Ok(())
}

/// 列出某账户的全部文件夹。
pub async fn list_folders(pool: &SqlitePool, account_id: &str) -> AppResult<Vec<Folder>> {
    let rows = sqlx::query_as!(
        FolderRow,
        r#"
        SELECT id, account_id, remote_name, remote_id, local_name, delimiter,
               subscribed, selectable, last_sync_at, last_uid, highest_modseq, uidvalidity
        FROM folder
        WHERE account_id = ?
        ORDER BY remote_name ASC
        "#,
        account_id
    )
    .fetch_all(pool)
    .await
    .map_err(|e| map_sqlx(e, "list_folders"))?;

    rows.iter().map(row_to_folder).collect()
}

/// 按 id 取单个文件夹。
pub async fn get_folder(pool: &SqlitePool, id: &str) -> AppResult<Option<Folder>> {
    let row = sqlx::query_as!(
        FolderRow,
        r#"
        SELECT id, account_id, remote_name, remote_id, local_name, delimiter,
               subscribed, selectable, last_sync_at, last_uid, highest_modseq, uidvalidity
        FROM folder
        WHERE id = ?
        "#,
        id
    )
    .fetch_optional(pool)
    .await
    .map_err(|e| map_sqlx(e, "get_folder"))?;

    row.as_ref().map(row_to_folder).transpose()
}

/// 删除一个文件夹。
///
/// 其下邮件、附件随之级联删除（缓存数据，无独立价值）。
/// 绑定到这些邮件的待提交操作**不**随之删除：它们只做逻辑引用，
/// 由同步层核对后置为 `uncertain` 或 `failed`（见 ARCHITECTURE §4.3）。
pub async fn delete_folder(pool: &SqlitePool, id: &str) -> AppResult<bool> {
    let result = sqlx::query!("DELETE FROM folder WHERE id = ?", id)
        .execute(pool)
        .await
        .map_err(|e| map_sqlx(e, "delete_folder"))?;

    Ok(result.rows_affected() > 0)
}

/// 行 → 领域对象。
fn row_to_folder(row: &FolderRow) -> AppResult<Folder> {
    Ok(Folder {
        id: row.id.clone(),
        account_id: row.account_id.clone(),
        remote_name: row.remote_name.clone(),
        remote_id: row.remote_id.clone(),
        local_name: row.local_name.clone(),
        delimiter: row.delimiter.clone(),
        subscribed: row.subscribed,
        selectable: row.selectable,
        last_sync_at: row.last_sync_at,
        last_uid: row.last_uid,
        highest_modseq: row.highest_modseq,
        uidvalidity: row.uidvalidity,
    })
}
