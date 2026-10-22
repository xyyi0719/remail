//! 账户 DAO。
//!
//! 对应 `account` 表。凭据本身**不落库**：`secret_ref` 只是操作系统钥匙串中
//! 一条机密的不透明引用（见 [ARCHITECTURE §2](../../../../docs/ARCHITECTURE.md)）。

use sqlx::FromRow;
use sqlx::SqlitePool;
use xremail_core::model::{
    Account, AuthKind, ProviderKind, ServerConfig, SmtpAuth, SyncMode, TlsMode,
};
use xremail_core::{AppError, AppResult};

use crate::codec::{enum_from_text, enum_to_text};
use crate::error::map_sqlx;

/// `account` 表的一行。
///
/// 字段名与列名一一对应；`sqlx` 的 `FromRow` 派生按列名取值。
/// 这里的类型必须与 schema 严格一致——`list` / `get` 共用同一份映射，
/// 避免两条路径长出不同行为。
#[derive(Debug, Clone, FromRow)]
#[sqlx(rename_all = "snake_case")]
struct AccountRow {
    id: String,
    email: String,
    provider: String,
    display_name: Option<String>,
    color: Option<String>,
    auth_kind: String,
    imap_host: String,
    imap_port: i64,
    imap_tls: String,
    smtp_host: String,
    smtp_port: i64,
    smtp_tls: String,
    smtp_auth: String,
    username: String,
    secret_ref: Option<String>,
    oauth_client_id: Option<String>,
    sync_mode: String,
    poll_interval_secs: i64,
    signature_html: Option<String>,
    created_at: i64,
    updated_at: i64,
}

/// 插入或更新一个账户。
///
/// 以 `id` 为主键冲突键：同 id 视为同一账户的更新，而不是新建。
/// `created_at` 保留首次写入的值。
pub async fn upsert_account(pool: &SqlitePool, account: &Account) -> AppResult<()> {
    let provider = enum_to_text(&account.provider, "provider")?;
    let auth_kind = enum_to_text(&account.auth_kind, "auth_kind")?;
    let imap_tls = enum_to_text(&account.server.imap_tls, "imap_tls")?;
    let smtp_tls = enum_to_text(&account.server.smtp_tls, "smtp_tls")?;
    let smtp_auth = enum_to_text(&account.server.smtp_auth, "smtp_auth")?;
    let sync_mode = enum_to_text(&account.sync_mode, "sync_mode")?;

    sqlx::query!(
        r#"
        INSERT INTO account (
            id, email, provider, display_name, color, auth_kind,
            imap_host, imap_port, imap_tls,
            smtp_host, smtp_port, smtp_tls, smtp_auth,
            username, secret_ref, oauth_client_id, sync_mode,
            poll_interval_secs, signature_html, created_at, updated_at
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT (id) DO UPDATE SET
            email              = excluded.email,
            provider           = excluded.provider,
            display_name       = excluded.display_name,
            color              = excluded.color,
            auth_kind          = excluded.auth_kind,
            imap_host          = excluded.imap_host,
            imap_port          = excluded.imap_port,
            imap_tls           = excluded.imap_tls,
            smtp_host          = excluded.smtp_host,
            smtp_port          = excluded.smtp_port,
            smtp_tls           = excluded.smtp_tls,
            smtp_auth          = excluded.smtp_auth,
            username           = excluded.username,
            secret_ref         = excluded.secret_ref,
            oauth_client_id    = excluded.oauth_client_id,
            sync_mode          = excluded.sync_mode,
            poll_interval_secs = excluded.poll_interval_secs,
            signature_html     = excluded.signature_html,
            updated_at         = excluded.updated_at
        "#,
        account.id,
        account.email,
        provider,
        account.display_name,
        account.color,
        auth_kind,
        account.server.imap_host,
        i64::from(account.server.imap_port),
        imap_tls,
        account.server.smtp_host,
        i64::from(account.server.smtp_port),
        smtp_tls,
        smtp_auth,
        account.username,
        account.secret_ref,
        account.oauth_client_id,
        sync_mode,
        account.poll_interval_secs,
        account.signature_html,
        account.created_at,
        account.updated_at,
    )
    .execute(pool)
    .await
    .map_err(|e| map_sqlx(e, "upsert_account"))?;

    Ok(())
}

/// 列出全部账户，按创建时间排序。
pub async fn list_accounts(pool: &SqlitePool) -> AppResult<Vec<Account>> {
    let rows = sqlx::query_as!(
        AccountRow,
        r#"
        SELECT id, email, provider, display_name, color, auth_kind,
               imap_host, imap_port, imap_tls,
               smtp_host, smtp_port, smtp_tls, smtp_auth,
               username, secret_ref, oauth_client_id, sync_mode,
               poll_interval_secs, signature_html, created_at, updated_at
        FROM account
        ORDER BY created_at ASC, id ASC
        "#
    )
    .fetch_all(pool)
    .await
    .map_err(|e| map_sqlx(e, "list_accounts"))?;

    rows.iter().map(row_to_account).collect()
}

/// 按 id 取单个账户。
pub async fn get_account(pool: &SqlitePool, id: &str) -> AppResult<Option<Account>> {
    let row = sqlx::query_as!(
        AccountRow,
        r#"
        SELECT id, email, provider, display_name, color, auth_kind,
               imap_host, imap_port, imap_tls,
               smtp_host, smtp_port, smtp_tls, smtp_auth,
               username, secret_ref, oauth_client_id, sync_mode,
               poll_interval_secs, signature_html, created_at, updated_at
        FROM account
        WHERE id = ?
        "#,
        id
    )
    .fetch_optional(pool)
    .await
    .map_err(|e| map_sqlx(e, "get_account"))?;

    row.as_ref().map(row_to_account).transpose()
}

/// 删除账户。
///
/// 返回是否真的删掉了行。账户的文件夹、邮件、附件随之级联删除；
/// 操作记录与发信记录也会级联，但它们对邮件只做逻辑引用。
pub async fn delete_account(pool: &SqlitePool, id: &str) -> AppResult<bool> {
    let result = sqlx::query!("DELETE FROM account WHERE id = ?", id)
        .execute(pool)
        .await
        .map_err(|e| map_sqlx(e, "delete_account"))?;

    Ok(result.rows_affected() > 0)
}

/// 行 → 领域对象。
///
/// 解析失败一律返回 `Invalid`：**不**静默回退到默认值。
/// 一个不认识的 provider 字符串说明数据与代码已经不一致。
fn row_to_account(row: &AccountRow) -> AppResult<Account> {
    let provider: ProviderKind = enum_from_text(&row.provider, "provider")?;
    let auth_kind: AuthKind = enum_from_text(&row.auth_kind, "auth_kind")?;
    let imap_tls: TlsMode = enum_from_text(&row.imap_tls, "imap_tls")?;
    let smtp_tls: TlsMode = enum_from_text(&row.smtp_tls, "smtp_tls")?;
    let smtp_auth: SmtpAuth = enum_from_text(&row.smtp_auth, "smtp_auth")?;
    let sync_mode: SyncMode = enum_from_text(&row.sync_mode, "sync_mode")?;

    Ok(Account {
        id: row.id.clone(),
        email: row.email.clone(),
        provider,
        display_name: row.display_name.clone(),
        color: row.color.clone(),
        auth_kind,
        server: ServerConfig {
            imap_host: row.imap_host.clone(),
            imap_port: to_port(row.imap_port, "imap_port")?,
            imap_tls,
            smtp_host: row.smtp_host.clone(),
            smtp_port: to_port(row.smtp_port, "smtp_port")?,
            smtp_tls,
            smtp_auth,
        },
        username: row.username.clone(),
        secret_ref: row.secret_ref.clone(),
        oauth_client_id: row.oauth_client_id.clone(),
        sync_mode,
        poll_interval_secs: row.poll_interval_secs,
        signature_html: row.signature_html.clone(),
        created_at: row.created_at,
        updated_at: row.updated_at,
    })
}

/// 端口号收敛回 `u16`。
///
/// schema 已有 `CHECK (port BETWEEN 1 AND 65535)`，但数据库可能被外部改写，
/// 因此读回时仍做一次边界检查，损坏的数据不进入领域模型。
fn to_port(value: i64, field: &str) -> AppResult<u16> {
    u16::try_from(value).map_err(|_| AppError::Invalid(format!("{field} 越界: {value}")))
}
