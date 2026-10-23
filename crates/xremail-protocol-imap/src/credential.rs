//! 本机测试账号库。
//!
//! # 为什么放这里
//!
//! 凭据**只在用户机器上**存在，**不进仓库、不进日志、不进命令行参数**
//! （见 [AI 开发工作规范 §1.6](../../../docs/AI_DEVELOPMENT_GUIDE.md)）。
//! 默认位置：`%USERPROFILE%/.xremail/accounts.txt`（Windows），
//! 或 `~/.xremail/accounts.txt`。
//!
//! # 文件格式
//!
//! 段落名是**服务商标识**（`[163]` / `[qq]` / `[wo]` / 自定义也行），
//! 段内是 `key = value`：
//!
//! ```text
//! [163]
//! user = someone@163.com
//! password = 16位客户端授权码
//! ```
//!
//! `password` 填**客户端授权码**，不是登录密码。
//!
//! # 兼容
//!
//! 没有段落头的文件被当作**单个**账号，归到调用方指定的 provider 下
//! （老格式的 `probe-163.txt` 仍可直接用）。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use xremail_api::SecretValue;

/// 凭据库错误。
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CredentialError {
    /// 文件读不到。
    #[error("读取凭据文件失败 ({0}): {1}")]
    Io(String, String),

    /// 文件格式不对。
    #[error("凭据文件格式错误: {0}")]
    Format(String),

    /// 找不到指定服务商的账号。
    #[error("凭据文件里没有 provider = {0} 的账号")]
    NotFound(String),
}

/// 一条账号凭据。
#[derive(Debug, Clone)]
pub struct AccountCredential {
    /// 服务商标识（段落名）。
    pub provider: String,
    /// 邮箱地址。
    pub user: String,
    /// 客户端授权码。
    ///
    /// 用 [`SecretValue`] 包着：丢弃时清零，且 `Debug` 不打印内容。
    pub password: SecretValue,
}

impl AccountCredential {
    /// 脱敏后的账号标识：首字符 + 域名。
    ///
    /// 用于在报告里区分是哪个账号，**不暴露**完整地址。
    pub fn masked_user(&self) -> String {
        mask_user(&self.user)
    }
}

/// 凭据库：provider → 凭据。
#[derive(Debug, Clone, Default)]
pub struct CredentialStore {
    accounts: BTreeMap<String, AccountCredential>,
}

impl CredentialStore {
    /// 从文件读取。
    pub fn read_from_file(path: &Path) -> Result<Self, CredentialError> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| CredentialError::Io(path.display().to_string(), e.to_string()))?;
        Self::parse(&text)
    }

    /// 解析文本内容。
    pub fn parse(text: &str) -> Result<Self, CredentialError> {
        let mut accounts: BTreeMap<String, AccountCredential> = BTreeMap::new();
        let mut current_provider: Option<String> = None;
        let mut current_user: Option<String> = None;
        let mut current_password: Option<String> = None;

        let flush = |provider: &mut Option<String>,
                     user: &mut Option<String>,
                     password: &mut Option<String>,
                     accounts: &mut BTreeMap<String, AccountCredential>|
         -> Result<(), CredentialError> {
            let provider = provider.take();
            let user = user.take();
            let password = password.take();

            match (&provider, &user, &password) {
                // 整个文件都是空的。
                (None, None, None) => Ok(()),
                // 段落头在，但两个值都没有 → 这个服务商**没配**（模板里的空段落），跳过。
                (Some(_), None, None) => Ok(()),
                // 只填了一半 → 配置写错了，**报错**而不是静默跳过。
                (Some(p), Some(_), None) | (Some(p), None, Some(_)) => {
                    Err(CredentialError::Format(format!(
                        "provider = {p} 的段落只填了 user 或 password 之一"
                    )))
                }
                // 有值但没有段落头。
                (None, Some(_), _) | (None, _, Some(_)) => Err(CredentialError::Format(
                    "文件里有 user/password 但没有 [provider] 段落头".into(),
                )),
                (Some(p), Some(u), Some(pw)) => {
                    if u.trim().is_empty() || pw.trim().is_empty() {
                        return Err(CredentialError::Format(format!(
                            "provider = {p} 的段落里 user/password 为空"
                        )));
                    }
                    accounts.insert(
                        p.clone(),
                        AccountCredential {
                            provider: p.clone(),
                            user: u.trim().to_string(),
                            password: SecretValue::new(pw.trim().as_bytes().to_vec()),
                        },
                    );
                    Ok(())
                }
            }
        };

        for raw_line in text.lines() {
            let line = raw_line.trim();

            if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
                continue;
            }

            if let Some(rest) = line.strip_prefix('[') {
                // 新段落开始：先把上一段落结算掉。
                flush(
                    &mut current_provider,
                    &mut current_user,
                    &mut current_password,
                    &mut accounts,
                )?;

                let name = rest
                    .strip_suffix(']')
                    .ok_or_else(|| CredentialError::Format(format!("段落头缺右括号: {line}")))?
                    .trim()
                    .to_string();
                if name.is_empty() {
                    return Err(CredentialError::Format("段落名为空".into()));
                }
                current_provider = Some(name);
                continue;
            }

            let Some((key, value)) = line.split_once('=') else {
                return Err(CredentialError::Format(format!(
                    "不是 key = value 形式: {line}"
                )));
            };

            match key.trim().to_ascii_lowercase().as_str() {
                // 空值当作「没填」——模板里的空段落就是这么写的。
                "user" => current_user = non_empty(value),
                "password" => current_password = non_empty(value),
                // 未知键忽略（前向兼容）。
                _ => {}
            }
        }

        flush(
            &mut current_provider,
            &mut current_user,
            &mut current_password,
            &mut accounts,
        )?;

        // 没有任何段落头的老格式：整份文件算一条账号。
        if accounts.is_empty() && (current_user.is_some() || current_password.is_some()) {
            return Err(CredentialError::Format(
                "文件里没有 [provider] 段落头".into(),
            ));
        }

        Ok(Self { accounts })
    }

    /// 取某个服务商的凭据。
    pub fn get(&self, provider: &str) -> Result<&AccountCredential, CredentialError> {
        self.accounts
            .get(provider)
            .ok_or_else(|| CredentialError::NotFound(provider.to_string()))
    }

    /// 已配置的 provider 列表。
    pub fn providers(&self) -> Vec<&str> {
        self.accounts.keys().map(String::as_str).collect()
    }

    /// 是否为空。
    pub fn is_empty(&self) -> bool {
        self.accounts.is_empty()
    }
}

/// 脱敏后的账号标识：首字符 + 域名。
pub fn mask_user(user: &str) -> String {
    match user.split_once('@') {
        Some((local, domain)) => {
            let head: String = local.chars().take(1).collect();
            format!("{head}***@{domain}")
        }
        None => "***".to_string(),
    }
}

/// 空值当作 `None`。
fn non_empty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// 默认凭据文件路径。
pub fn default_accounts_path() -> PathBuf {
    let home = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    home.join(".xremail").join("accounts.txt")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_multiple_providers() {
        let store = CredentialStore::parse(
            "[163]\nuser = a@163.com\npassword = pwd163\n\n[qq]\nuser = b@qq.com\npassword = pwdqq\n",
        )
        .unwrap();

        assert_eq!(store.providers(), vec!["163", "qq"]);
        assert_eq!(store.get("163").unwrap().user, "a@163.com");
        assert_eq!(store.get("qq").unwrap().password.as_bytes(), b"pwdqq");
    }

    #[test]
    fn ignores_comments_and_blank_lines() {
        let store = CredentialStore::parse(
            "# 注释\n\n[163]\n; 另一种注释\nuser = a@163.com\npassword = pwd\n",
        )
        .unwrap();

        assert_eq!(store.get("163").unwrap().user, "a@163.com");
    }

    #[test]
    fn missing_provider_is_an_error() {
        let store = CredentialStore::parse("[163]\nuser = a@163.com\npassword = pwd\n").unwrap();

        assert!(matches!(
            store.get("qq").unwrap_err(),
            CredentialError::NotFound(_)
        ));
    }

    #[test]
    fn section_without_password_is_an_error() {
        let err = CredentialStore::parse("[163]\nuser = a@163.com\n").unwrap_err();

        assert!(matches!(err, CredentialError::Format(_)));
    }

    #[test]
    fn empty_section_means_not_configured() {
        // 模板里的空段落：既没有 user 也没有 password → 跳过，不报错。
        let store =
            CredentialStore::parse("[163]\nuser = a@163.com\npassword = pwd\n\n[qq]\n\n[wo]\n")
                .unwrap();

        assert_eq!(store.providers(), vec!["163"]);
    }

    #[test]
    fn empty_values_count_as_not_configured() {
        // 模板里的写法是 `user =`（有键、值为空），也算「没配」。
        let store = CredentialStore::parse(
            "[163]\nuser = a@163.com\npassword = pwd\n\n[qq]\nuser =\npassword =\n",
        )
        .unwrap();

        assert_eq!(store.providers(), vec!["163"]);
    }

    #[test]
    fn one_empty_one_filled_is_an_error() {
        let err = CredentialStore::parse("[163]\nuser = a@163.com\npassword =\n").unwrap_err();

        assert!(matches!(err, CredentialError::Format(_)));
    }

    #[test]
    fn empty_value_is_an_error() {
        let err = CredentialStore::parse("[163]\nuser = a@163.com\npassword =\n").unwrap_err();

        assert!(matches!(err, CredentialError::Format(_)));
    }

    #[test]
    fn unterminated_section_header_is_an_error() {
        let err = CredentialStore::parse("[163\nuser = a@163.com\npassword = p\n").unwrap_err();

        assert!(matches!(err, CredentialError::Format(_)));
    }

    #[test]
    fn line_without_equals_is_an_error() {
        let err = CredentialStore::parse("[163]\ngarbage\n").unwrap_err();

        assert!(matches!(err, CredentialError::Format(_)));
    }

    #[test]
    fn mask_hides_the_local_part() {
        assert_eq!(mask_user("someone@163.com"), "s***@163.com");
        assert_eq!(mask_user("garbage"), "***");
    }

    #[test]
    fn credential_debug_does_not_leak_the_password() {
        let store =
            CredentialStore::parse("[163]\nuser = a@163.com\npassword = secret123\n").unwrap();
        let printed = format!("{:?}", store.get("163").unwrap());

        assert!(!printed.contains("secret123"), "授权码泄漏到了 Debug 输出");
    }
}
