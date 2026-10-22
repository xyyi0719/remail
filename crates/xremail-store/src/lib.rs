//! Xremail 持久化层。
//!
//! 职责边界（见 [ARCHITECTURE §2](../../../docs/ARCHITECTURE.md)）：
//! * 只做 SQL、迁移与 DAO，**不发网络请求**。
//! * 领域模型来自 `xremail-core`，本 crate 不重复定义实体。
//! * 错误统一映射到 `xremail_core::AppError`，保留可重试 / 需用户介入的分类，
//!   不把一切折叠成 `Other`。
//!
//! 依赖方向：`shell → … → store → core`，不产生反向依赖。

pub mod codec;
pub mod dao;
pub mod error;
pub mod pool;

pub use error::{StoreResult, map_sqlx};
pub use pool::{PoolConfig, migrate, open_pool, open_pool_in_memory, open_pool_path};
