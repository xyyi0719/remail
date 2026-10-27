//! 应用共享状态。
//!
//! 由 Tauri `manage` 注入。持有应用数据目录与各领域服务。

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use sqlx::SqlitePool;
use xremail_api::SecretValue;

use crate::service::{AccountService, MessageService, SyncService};

/// 应用共享状态。
#[derive(Debug)]
pub struct AppState {
    /// 数据目录（数据库、附件、日志的根目录）。
    pub data_dir: PathBuf,
    /// 账户服务。
    pub accounts: AccountService,
    /// 邮件服务。
    pub messages: MessageService,
    /// 同步服务。
    pub sync: SyncService,
    /// 数据库连接池。
    pool: SqlitePool,
    /// 本次会话的账户凭据（**只在内存里**，不落盘）。
    ///
    /// 平台钥匙串适配属 P4；在此之前，用户每次同步时输入授权码，
    /// 它只驻留到这次会话结束。
    ///
    /// 用 [`SecretValue`]：丢弃时清零，`Debug` 只打印长度不打印内容。
    credentials: Mutex<HashMap<String, SecretValue>>,
}

impl AppState {
    /// 构造状态。
    pub fn new(
        data_dir: PathBuf,
        accounts: AccountService,
        messages: MessageService,
        sync: SyncService,
        pool: SqlitePool,
    ) -> Self {
        Self {
            data_dir,
            accounts,
            messages,
            sync,
            pool,
            credentials: Mutex::new(HashMap::new()),
        }
    }

    /// 数据库文件路径。
    pub fn database_path(&self) -> PathBuf {
        self.data_dir.join(crate::DB_FILE_NAME)
    }

    /// 数据库连接池。
    ///
    /// Command 需要它做「按需拉正文」这种跨服务的操作。
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    /// 记下某个账户的凭据（覆盖旧的）。
    pub fn remember_credential(&self, account_id: &str, credential: SecretValue) {
        if let Ok(mut map) = self.credentials.lock() {
            map.insert(account_id.to_string(), credential);
        }
    }

    /// 取某个账户的凭据。
    pub fn credential(&self, account_id: &str) -> Option<SecretValue> {
        let map = self.credentials.lock().ok()?;
        map.get(account_id).cloned()
    }

    /// 忘掉某个账户的凭据。
    pub fn forget_credential(&self, account_id: &str) {
        if let Ok(mut map) = self.credentials.lock() {
            map.remove(account_id);
        }
    }
}
