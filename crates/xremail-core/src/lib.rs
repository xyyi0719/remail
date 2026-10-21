//! Xremail 领域模型。
//!
//! 本 crate 是整个 workspace 的公共底座，**不得**依赖任何存储、网络或 UI crate
//! （不得出现 `sqlx` / `tokio` / `tauri` / `reqwest` 等引用）。
//! 其余所有 crate 都依赖它，从而保证依赖方向单一、无环。

pub mod error;
pub mod event;
pub mod model;

pub use error::{AppError, AppResult};
pub use event::AppEvent;
pub use model::{
    Account, Address, Attachment, AuthKind, Folder, Message, MessageBody, MessageFlags,
    ProviderKind, ServerConfig, SmtpAuth, SyncMode, SyncState, TlsMode,
};
