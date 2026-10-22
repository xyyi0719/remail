//! 连接池与迁移。
//!
//! 运行时参数（见 [ARCHITECTURE §3](../../../docs/ARCHITECTURE.md)）：
//! * **WAL**：读写不互相阻塞，是 SQLite 上多读者场景的默认选择。
//! * **busy_timeout**：写锁竞争时等待而不是立刻失败。
//! * **foreign_keys = ON**：SQLite 默认**不**启用外键约束，
//!   不显式打开的话 schema 里的级联与约束全是摆设。
//! * 有界连接池 + acquire_timeout：避免无界地等下去。

use std::str::FromStr;
use std::time::Duration;

use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous};

use crate::error::{StoreResult, map_sqlx};

/// 连接池配置。
#[derive(Debug, Clone)]
pub struct PoolConfig {
    /// 连接池上限。SQLite 写入是串行的，过大的池并不会更快。
    pub max_connections: u32,
    /// 遇到写锁时的等待时长。
    pub busy_timeout: Duration,
    /// 从池中取连接的最长等待。
    pub acquire_timeout: Duration,
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            max_connections: 4,
            busy_timeout: Duration::from_secs(5),
            acquire_timeout: Duration::from_secs(10),
        }
    }
}

/// 内嵌的迁移集合。编译期读入 `migrations/` 目录，运行时不依赖外部文件。
static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

/// 打开一个数据库连接池。
///
/// `url` 形如 `sqlite://data/xremail.db`。
/// 文件不存在时会创建；目录不存在则报错（不做隐式建目录）。
pub async fn open_pool(url: &str, config: &PoolConfig) -> StoreResult<SqlitePool> {
    let options = SqliteConnectOptions::from_str(url)
        .map_err(|e| map_sqlx(sqlx::Error::Configuration(Box::new(e)), "open_pool: url"))?
        .create_if_missing(true)
        // 外键约束默认关闭，必须显式打开。
        .foreign_keys(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .busy_timeout(config.busy_timeout);

    SqlitePoolOptions::new()
        .max_connections(config.max_connections)
        .acquire_timeout(config.acquire_timeout)
        .connect_with(options)
        .await
        .map_err(|e| map_sqlx(e, "open_pool"))
}

/// 按文件路径打开一个数据库连接池。
///
/// 桌面端的真实用法：数据目录是平台相关路径（如 Windows 的
/// `%APPDATA%/Xremail`），塞进 URL 再解析容易出问题，
/// 因此直接用路径拼 `SqliteConnectOptions`。
///
/// 父目录**不会**被隐式创建：目录不存在时直接报错，
/// 避免把「路径写错」变成「静默建了一个新库」。
pub async fn open_pool_path(
    path: &std::path::Path,
    config: &PoolConfig,
) -> StoreResult<SqlitePool> {
    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .foreign_keys(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .busy_timeout(config.busy_timeout);

    SqlitePoolOptions::new()
        .max_connections(config.max_connections)
        .acquire_timeout(config.acquire_timeout)
        .connect_with(options)
        .await
        .map_err(|e| map_sqlx(e, "open_pool_path"))
}

/// 打开一个内存库连接池，供测试使用。
///
/// 内存库每个连接独立，因此连接池被限制为**单连接**，
/// 否则第二个连接会看到一个空库。
pub async fn open_pool_in_memory(config: &PoolConfig) -> StoreResult<SqlitePool> {
    let options = SqliteConnectOptions::from_str("sqlite::memory:")
        .map_err(|e| {
            map_sqlx(
                sqlx::Error::Configuration(Box::new(e)),
                "open_pool_in_memory",
            )
        })?
        .foreign_keys(true)
        .busy_timeout(config.busy_timeout);

    SqlitePoolOptions::new()
        .max_connections(1)
        .acquire_timeout(config.acquire_timeout)
        .connect_with(options)
        .await
        .map_err(|e| map_sqlx(e, "open_pool_in_memory"))
}

/// 应用全部未执行的迁移。
///
/// 迁移是幂等的：已应用的版本会被跳过。
pub async fn migrate(pool: &SqlitePool) -> StoreResult<()> {
    MIGRATOR
        .run(pool)
        .await
        .map_err(|e| map_sqlx(sqlx::Error::Migrate(Box::new(e)), "migrate"))
}
