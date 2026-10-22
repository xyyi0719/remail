//! 内存实现（测试替身）。
//!
//! **这些不是生产实现。** 真实实现放在各领域 crate
//! （平台钥匙串适配器、SQLite vault、远端同步服务），
//! 这里只提供契约的参考实现，供测试使用
//! （见 [ARCHITECTURE §8](../../../../docs/ARCHITECTURE.md) 对 mock 的要求）。
//!
//! 放在 `xremail-api` 而不是测试目录里，是为了让下游 crate 的测试
//! 也能用到同一份 mock——L2 路径的集成测试需要它。

pub mod secret;
pub mod sync;
pub mod vault;

pub use secret::InMemorySecretStore;
pub use sync::InMemorySyncBackend;
pub use vault::InMemoryVaultStore;
