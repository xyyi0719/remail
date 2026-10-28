//! 平台凭据存储（[ARCHITECTURE §8](../../../docs/ARCHITECTURE.md)）。
//!
//! ## 这层只管一件事
//!
//! 把一条机密安全地放进**操作系统的凭据保管设施**，并能取回来。
//!
//! **不是** SQLite 里的 `local_secret` 表——那是 `xremail-api` 的 `vault` 的事。
//! **不是** `xremail-crypto` 的密文封装——那是加解密的事。
//!
//! ## 库里只存不透明引用
//!
//! 库里 [`xremail_core::model::Account::secret_ref`] 存的是**引用**
//! （如 `xremail:account:<account_id>`），不是凭据本身。
//! 凭据只待在平台钥匙串里。这样做的收益：
//!
//! * 数据库文件即使被拷走，也**不含**可直接使用的口令；
//! * 换机器 / 删账户时，可以精确地删掉那一条钥匙串记录；
//! * 同一套代码在 Windows / Android / macOS 上只是换掉本模块的实现。
//!
//! ## 平台差异
//!
//! * Windows：凭据管理器（Credential Manager）——见 [`windows_store`]。
//! * 其它平台：**暂不支持**（见 [`unsupported`])——存取都明确报错，
//!   **不静默降级**成"内存里存着"或"存到文件里"。
//!
//! 静默降级会让人以为凭据已经落盘，实际却没有——那正是本模块要避免的事。

use std::sync::Arc;

use xremail_api::SecretStore;

pub mod unsupported;
#[cfg(windows)]
pub mod windows_store;

/// 平台凭据存储。
///
/// 返回平台对应的实现。Windows 上是凭据管理器，其它平台是
/// [`unsupported::UnsupportedSecretStore`]（存取都报"不可用"）。
pub fn platform_store() -> Arc<dyn SecretStore> {
    #[cfg(windows)]
    {
        Arc::new(windows_store::WindowsSecretStore)
    }
    #[cfg(not(windows))]
    {
        Arc::new(unsupported::UnsupportedSecretStore)
    }
}

/// 账户凭据在平台钥匙串里的条目名。
///
/// 格式固定为 `xremail:account:<account_id>`：
///
/// * 前缀 `xremail:` 让本应用的条目在系统钥匙串里**可辨认、可批量清理**；
/// * 用账户 id（不是邮箱地址）做后缀，邮箱会变，id 不会。
///
/// 这是一个**不透明引用**：应用只拿它去 [`platform_store`] 取值，
/// 不解析它。格式若要改，这里是唯一的地方。
pub fn secret_ref_for(account_id: &str) -> String {
    format!("xremail:account:{account_id}")
}
