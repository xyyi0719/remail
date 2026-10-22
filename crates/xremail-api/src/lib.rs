//! Xremail 应用服务契约。
//!
//! 本 crate 只放**trait 与 DTO**，不放实现
//! （见 [ARCHITECTURE §2](../../../docs/ARCHITECTURE.md) 的模块职责表）。
//!
//! ## 三个契约
//!
//! | 契约 | 职责 | 不是 |
//! |---|---|---|
//! | [`SecretStore`] | 平台钥匙串里的凭据 | 不是 SQLite 存储，不是密文封装 |
//! | [`VaultStore`] | 本地版本化密文封装与元数据 | 不是操作系统钥匙串 |
//! | [`SyncBackend`] | 远端密文与修订号的原子条件提交 | 不是"先查后写" |
//!
//! ## 依赖方向
//!
//! `xremail-api` ← 一切。本 crate 只依赖 `xremail-core`，
//! 不依赖存储、网络或 UI crate，因此不会与任何 crate 形成环。
//!
//! ## 范围声明
//!
//! P0 只做**接口预留与可运行 mock**，**不建设远端服务**
//! （见 [ARCHITECTURE §8](../../../docs/ARCHITECTURE.md)）。
//! 接口预留降低耦合，**不保证零返工**——P6 之前要重新审查
//! 身份认证、密钥恢复、轮换、回滚防护与元数据泄露。

pub mod account;
pub mod error;
pub mod mock;
pub mod secret;
pub mod sync;
pub mod vault;

pub use account::NewAccountRequest;
pub use error::{SecretStoreError, SyncError, VaultError};
pub use secret::{SecretStore, SecretValue};
pub use sync::{RemoteEntry, Revision, SyncBackend};
pub use vault::{VaultEntry, VaultStore};
