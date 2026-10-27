//! 同步应用服务。
//!
//! 把「前端说：把这个账户同步一下」变成「连上服务器、同步、落库」。
//!
//! ## 凭据只驻留内存
//!
//! 授权码由前端在同步时传入，存在 [`crate::state::AppState`] 的
//! **内存**里（[`xremail_api::SecretValue`]，丢弃时清零、Debug 不打印内容），
//! **不落盘**。
//!
//! 落盘要等平台钥匙串（P4）或主密码方案——那时
//! `account.secret_ref` 才是有意义的不透明引用。
//!
//! ## 连接参数来自账户配置
//!
//! 服务器地址、端口、用户名都取自库里那条账户记录，
//! **不按品牌推定**（163/QQ/沃 的参数虽已实测，但用户可以改）。

use sqlx::SqlitePool;
use xremail_api::SecretValue;
use xremail_core::model::Account;
use xremail_core::{AppError, AppResult};
use xremail_protocol_imap::{ImapConfig, TlsTrust, connect_and_login_with};
use xremail_sync::{SyncSummary, sync_account};

/// 同步服务。
#[derive(Debug, Clone)]
pub struct SyncService {
    pool: SqlitePool,
}

impl SyncService {
    /// 用连接池构造。
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// 同步一个账户的全部文件夹。
    ///
    /// `credential` 是 IMAP 授权码（明文口令对多数服务商无效）。
    /// **用完即弃**——不回显、不落盘、不进日志。
    pub async fn sync_account(
        &self,
        account: &Account,
        credential: &SecretValue,
        now_ms: i64,
    ) -> AppResult<SyncSummary> {
        let password = credential
            .as_str()
            .ok_or_else(|| AppError::Invalid("授权码不是合法的 UTF-8 文本".into()))?;

        // 连接参数来自账户配置。
        let mut config = ImapConfig::imaps(&account.server.imap_host);
        config.port = account.server.imap_port;

        // 生产路径：系统根证书 + 强制校验，**没有**跳过校验的开关。
        let (mut session, _caps, _tls) =
            connect_and_login_with(&config, &TlsTrust::SystemRoots, &account.username, password)
                .await
                .map_err(|e| AppError::Protocol(format!("连接或登录失败: {e}")))?;

        sync_account(&self.pool, &mut session, account, now_ms).await
    }
}
