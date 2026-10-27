//! 应用服务层。
//!
//! 职责边界（见 [ARCHITECTURE §2](../../../../../docs/ARCHITECTURE.md)）：
//! * Command 只做 **参数校验 → 调用领域服务 → 返回 DTO**，不写业务逻辑。
//! * 业务逻辑在服务层。
//!
//! ## 为什么服务层暂时住在 shell crate 里
//!
//! P0 的账户服务只是「store 的 DAO + 入参校验」的组合，还没有任何独立的行为。
//! 为它单开一个 crate 属于过早抽象。等 P2 的同步、收发服务落地、
//! 服务层有了真正的业务规则，再拆成独立的领域 crate 或 `xremail-app`。
//!
//! 服务层**不认识 Tauri**：发事件是 Command 的事（状态提交后发，见
//! [ARCHITECTURE §1.6](../../../../../docs/ARCHITECTURE.md)）。

pub mod account;
pub mod message;
pub mod sync;

pub use account::AccountService;
pub use message::MessageService;
pub use sync::SyncService;
