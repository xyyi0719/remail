//! 账户应用服务。
//!
//! 把「前端来的请求」变成「落库的账户」，并把「库里的账户」变成「前端要的 DTO」。
//! 中间不出现 Tauri 类型。

use sqlx::SqlitePool;
use xremail_api::NewAccountRequest;
use xremail_core::AppResult;
use xremail_core::model::Account;
use xremail_store::dao::account as account_dao;

/// 账户服务。
#[derive(Debug, Clone)]
pub struct AccountService {
    pool: SqlitePool,
}

impl AccountService {
    /// 用连接池构造。
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// 列出全部账户。
    pub async fn list(&self) -> AppResult<Vec<Account>> {
        account_dao::list_accounts(&self.pool).await
    }

    /// 按 id 取账户。
    pub async fn get(&self, id: &str) -> AppResult<Option<Account>> {
        account_dao::get_account(&self.pool, id).await
    }

    /// 新增账户。
    ///
    /// 顺序很重要：**先校验，再落库**。校验失败时库里不会留下半条记录。
    pub async fn create(&self, request: NewAccountRequest) -> AppResult<Account> {
        let account = request.into_account(current_time_ms())?;
        account_dao::upsert_account(&self.pool, &account).await?;
        Ok(account)
    }

    /// 更新账户。
    pub async fn update(&self, account: Account) -> AppResult<Account> {
        let mut account = account;
        account.updated_at = current_time_ms();
        account_dao::upsert_account(&self.pool, &account).await?;
        Ok(account)
    }

    /// 删除账户。返回是否真的删掉了。
    pub async fn delete(&self, id: &str) -> AppResult<bool> {
        account_dao::delete_account(&self.pool, id).await
    }
}

/// 当前时间（epoch 毫秒）。
fn current_time_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};

    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
