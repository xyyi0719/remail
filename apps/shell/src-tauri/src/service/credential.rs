//! 凭据应用服务。
//!
//! 授权码存在**平台钥匙串**（Windows 凭据管理器 / macOS Keychain /
//! Android Keystore）里；数据库里只存一条**不透明引用**
//! （`xremail:account:<account_id>`），**不存凭据本身**。
//!
//! ## 为什么凭据不进数据库
//!
//! 数据库文件会被备份、被同步、被拷走。凭据一旦落进 SQLite，
//! 「文件泄露」就等于「口令泄露」。而钥匙串条目可以被**精确**删除：
//! 删账户时顺手删掉那一条，不留残骸。
//!
//! ## 平台不支持时**如实报错**
//!
//! 没适配钥匙串的平台，`set` 会返回
//! [`SecretStoreError::Unavailable`](xremail_api::SecretStoreError::Unavailable)。
//! 这里把它**原样透出来**，不降级成「存到内存里」或「存到文件里」——
//! 那样会让用户以为凭据已经落盘，实际却没有。
//!
//! ## 机密不进日志、不进错误信息
//!
//! 授权码全程只以 [`SecretValue`] 的形式在内存里流转；
//! 它的 `Debug` 只打印长度。错误信息里也永远不会带上它。

use xremail_api::{SecretStoreError, SecretValue};
use xremail_core::model::Account;
use xremail_core::{AppError, AppResult};

use crate::secrets;
use crate::service::AccountService;

/// 凭据服务。
///
/// 只做「钥匙串 ↔ 账户记录」这一件事；它**不认识** Tauri，
/// 也不碰内存里那份会话凭据（那由 [`crate::state::AppState`] 持有）。
#[derive(Debug, Clone)]
pub struct CredentialService {
    accounts: AccountService,
}

impl CredentialService {
    /// 用账户服务构造。
    pub fn new(accounts: AccountService) -> Self {
        Self { accounts }
    }

    /// 取某个账户的授权码。
    ///
    /// 只看 `account.secret_ref`：没有引用就**不去打扰**钥匙串。
    /// 钥匙串里没有这条 → `Ok(None)`（"没保存过"不是错误）。
    ///
    /// 钥匙串**本身**出故障（被系统锁定、权限不足）→ 返回错误：
    /// 那不是"没有凭据"，而是"取不到凭据"，两者必须分清。
    pub async fn resolve_credential(&self, account: &Account) -> AppResult<Option<SecretValue>> {
        let Some(secret_ref) = account.secret_ref.as_deref() else {
            return Ok(None);
        };

        let store = secrets::platform_store();
        store
            .get(secret_ref)
            .await
            .map_err(|error| AppError::Storage(read_error_message(&error)))
    }

    /// 把授权码存进平台钥匙串，并把账户的 `secret_ref` 更新成这条引用。
    ///
    /// 顺序很重要：**先写钥匙串，成功了再改数据库**。
    /// 反过来的话，一旦写钥匙串失败，数据库里会留下一条
    /// 指向空条目的引用——下次同步就会说"有凭据却连不上"。
    pub async fn save_credential(
        &self,
        account_id: &str,
        credential: SecretValue,
    ) -> AppResult<()> {
        let mut account = self
            .accounts
            .get(account_id)
            .await?
            .ok_or_else(|| AppError::NotFound(format!("账户 {account_id} 不存在")))?;

        let secret_ref = secrets::secret_ref_for(account_id);
        let store = secrets::platform_store();
        store
            .set(&secret_ref, &credential)
            .await
            .map_err(|error| AppError::Storage(save_error_message(&error)))?;

        account.secret_ref = Some(secret_ref);
        self.accounts.update(account).await?;
        Ok(())
    }

    /// 忘掉某个账户的授权码。
    ///
    /// 做两件事：删钥匙串条目 → 把账户的 `secret_ref` 清成 `None`。
    /// 内存里那份由调用方清（[`crate::state::AppState::forget_credential`]）——
    /// 内存凭据表属于应用状态，不属于本服务。
    ///
    /// 返回是否**真的**删掉了钥匙串里的那条。
    /// 「本来就没有」也是 `false`，但那仍然是一次成功的「忘掉」。
    pub async fn forget_credential(&self, account_id: &str) -> AppResult<bool> {
        let store = secrets::platform_store();

        // 优先删账户里记着的那条引用——那才是当初真正写进去的；
        // 再补删一次规范引用，清理掉「引用已被清掉、条目还在」的残骸。
        let stored_ref = self
            .accounts
            .get(account_id)
            .await?
            .and_then(|account| account.secret_ref);
        let canonical_ref = secrets::secret_ref_for(account_id);

        let mut deleted = false;
        let mut seen: Vec<String> = Vec::new();
        for candidate in [stored_ref, Some(canonical_ref)].into_iter().flatten() {
            if seen.contains(&candidate) {
                continue;
            }
            seen.push(candidate.clone());
            if store
                .delete(&candidate)
                .await
                .map_err(|error| AppError::Storage(delete_error_message(&error)))?
            {
                deleted = true;
            }
        }

        // 引用要一起清掉：留着它，下次同步会去找一条已经不存在的条目。
        if let Some(mut account) = self.accounts.get(account_id).await?
            && account.secret_ref.is_some()
        {
            account.secret_ref = None;
            self.accounts.update(account).await?;
        }

        Ok(deleted)
    }
}

/// 取出钥匙串错误里那句**说明**。
///
/// [`SecretStoreError`] 的 `Display` 带一段英文前缀
/// （`secret store unavailable: …`），那是内部术语；
/// 但说明本身**不能丢**——那句里才有"为什么存不进去"。
fn secret_detail(error: &SecretStoreError) -> String {
    match error {
        SecretStoreError::Unavailable(detail)
        | SecretStoreError::Denied(detail)
        | SecretStoreError::Invalid(detail) => detail.clone(),
        other => other.to_string(),
    }
}

/// 保存失败时给用户看的话。
fn save_error_message(error: &SecretStoreError) -> String {
    format!(
        "保存授权码失败：{}。授权码没有存进系统凭据保管设施；你仍然可以在同步时临时输入授权码。",
        secret_detail(error)
    )
}

/// 读取失败时给用户看的话。
fn read_error_message(error: &SecretStoreError) -> String {
    format!(
        "读取授权码失败：{}。请重新输入授权码并同步一次。",
        secret_detail(error)
    )
}

/// 删除失败时给用户看的话。
fn delete_error_message(error: &SecretStoreError) -> String {
    format!("删除授权码失败：{}。请稍后重试。", secret_detail(error))
}
