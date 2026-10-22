//! 应用共享状态。
//!
//! 由 Tauri `manage` 注入。持有应用数据目录与各领域服务。

use std::path::PathBuf;

use crate::service::AccountService;

/// 应用共享状态。
#[derive(Debug)]
pub struct AppState {
    /// 数据目录（数据库、附件、日志的根目录）。
    pub data_dir: PathBuf,
    /// 账户服务。
    pub accounts: AccountService,
}

impl AppState {
    /// 构造状态。
    pub fn new(data_dir: PathBuf, accounts: AccountService) -> Self {
        Self { data_dir, accounts }
    }

    /// 数据库文件路径。
    pub fn database_path(&self) -> PathBuf {
        self.data_dir.join(crate::DB_FILE_NAME)
    }
}
