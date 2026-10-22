//! 账户的应用服务 DTO。
//!
//! 这些是**前端与服务层之间的契约**，不是 UI 的类型：
//! UI 不直接构造领域对象，领域对象也不直接暴露给 UI。
//!
//! 参见 [ARCHITECTURE §2](../../../docs/ARCHITECTURE.md) 的模块职责表：
//! `xremail-api` 放「应用服务 DTO」，**实现**放各领域 crate。
//!
//! ## 当前范围的限制
//!
//! 本 DTO **不含凭据**。P0 还没有平台钥匙串适配器
//! （那属于 P4），所以这里刻意不接收密码或授权码：
//! 一个无处安全存放的凭据，比没有凭据更危险。
//! 凭据字段在 P1 接入时补，届时走 [`crate::secret::SecretStore`]。

use serde::Deserialize;
use xremail_core::AppError;
use xremail_core::model::{
    Account, AuthKind, ProviderKind, ServerConfig, SmtpAuth, SyncMode, TlsMode,
};

/// 新增账户的请求。
///
/// 字段名用 snake_case：Rust 侧的 `Account` 直接按默认命名序列化，
/// 这里保持一致，避免前端做两套映射。
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewAccountRequest {
    /// 邮箱地址。
    pub email: String,
    /// 服务商标记。
    ///
    /// **只是标记**：每个字段都可覆盖。服务端地址不按品牌推定——
    /// 各服务商的参数要实测确认（见 [ISSUES_AND_RISKS R01/R02](../../../docs/ISSUES_AND_RISKS.md)）。
    pub provider: ProviderKind,
    /// 显示名。
    #[serde(default)]
    pub display_name: Option<String>,
    /// IMAP 服务器地址。
    pub imap_host: String,
    /// IMAP 端口。
    pub imap_port: u16,
    /// IMAP 传输安全方式。
    pub imap_tls: TlsMode,
    /// SMTP 服务器地址。
    pub smtp_host: String,
    /// SMTP 端口。
    pub smtp_port: u16,
    /// SMTP 传输安全方式。
    pub smtp_tls: TlsMode,
    /// SMTP 认证方式。
    pub smtp_auth: SmtpAuth,
    /// 提交给服务器的用户名（通常是完整邮箱地址）。
    pub username: String,
    /// 同步方式。
    pub sync_mode: SyncMode,
    /// 轮询间隔（秒）。`SyncMode::Idle` 时不使用。
    pub poll_interval_secs: i64,
}

impl NewAccountRequest {
    /// 校验入参。
    ///
    /// 失败一律返回 [`AppError::Invalid`]：用户输入的问题需要用户改，
    /// 重试没有意义。
    pub fn validate(&self) -> Result<(), AppError> {
        if self.email.trim().is_empty() {
            return Err(AppError::Invalid("邮箱不能为空".into()));
        }
        if !self.email.contains('@') {
            return Err(AppError::Invalid(format!("邮箱格式不对：{:?}", self.email)));
        }
        if self.imap_host.trim().is_empty() {
            return Err(AppError::Invalid("IMAP 服务器地址不能为空".into()));
        }
        if self.imap_port == 0 {
            return Err(AppError::Invalid("IMAP 端口不能为 0".into()));
        }
        if self.smtp_host.trim().is_empty() {
            return Err(AppError::Invalid("SMTP 服务器地址不能为空".into()));
        }
        if self.smtp_port == 0 {
            return Err(AppError::Invalid("SMTP 端口不能为 0".into()));
        }
        if self.username.trim().is_empty() {
            return Err(AppError::Invalid("用户名不能为空".into()));
        }
        if self.poll_interval_secs <= 0 {
            return Err(AppError::Invalid("轮询间隔必须为正数".into()));
        }

        Ok(())
    }

    /// 校验并转换为领域对象。
    ///
    /// `now_ms` 是当前时间（epoch 毫秒），由调用方注入以便测试。
    pub fn into_account(self, now_ms: i64) -> Result<Account, AppError> {
        self.validate()?;

        Ok(Account {
            id: uuid::Uuid::new_v4().to_string(),
            email: self.email.trim().to_string(),
            provider: self.provider,
            display_name: self
                .display_name
                .map(|name| name.trim().to_string())
                .filter(|name| !name.is_empty()),
            color: None,
            // P0 只支持口令/授权码；OAuth2 随 P1 的服务商接入一起加。
            auth_kind: AuthKind::Password,
            server: ServerConfig {
                imap_host: self.imap_host.trim().to_string(),
                imap_port: self.imap_port,
                imap_tls: self.imap_tls,
                smtp_host: self.smtp_host.trim().to_string(),
                smtp_port: self.smtp_port,
                smtp_tls: self.smtp_tls,
                smtp_auth: self.smtp_auth,
            },
            username: self.username.trim().to_string(),
            // 凭据尚未接入平台钥匙串，这里保持为空而不是存明文。
            secret_ref: None,
            oauth_client_id: None,
            sync_mode: self.sync_mode,
            poll_interval_secs: self.poll_interval_secs,
            signature_html: None,
            created_at: now_ms,
            updated_at: now_ms,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_request() -> NewAccountRequest {
        NewAccountRequest {
            email: "user@163.com".into(),
            provider: ProviderKind::Netease163,
            display_name: Some("我的邮箱".into()),
            imap_host: "imap.163.com".into(),
            imap_port: 993,
            imap_tls: TlsMode::Implicit,
            smtp_host: "smtp.163.com".into(),
            smtp_port: 465,
            smtp_tls: TlsMode::Implicit,
            smtp_auth: SmtpAuth::Login,
            username: "user@163.com".into(),
            sync_mode: SyncMode::Poll,
            poll_interval_secs: 300,
        }
    }

    #[test]
    fn valid_request_converts_to_account() {
        let account = valid_request().into_account(1_700_000_000_000).unwrap();

        assert_eq!(account.email, "user@163.com");
        assert_eq!(account.provider, ProviderKind::Netease163);
        assert_eq!(account.server.imap_port, 993);
        assert_eq!(account.server.smtp_auth, SmtpAuth::Login);
        assert_eq!(account.created_at, 1_700_000_000_000);
        assert_eq!(account.updated_at, 1_700_000_000_000);
        assert!(!account.id.is_empty(), "id 应自动生成");
    }

    #[test]
    fn generated_ids_are_unique() {
        let a = valid_request().into_account(0).unwrap();
        let b = valid_request().into_account(0).unwrap();

        assert_ne!(a.id, b.id);
    }

    #[test]
    fn secret_is_never_stored_in_the_account() {
        let account = valid_request().into_account(0).unwrap();

        assert!(
            account.secret_ref.is_none(),
            "P0 没有平台钥匙串，凭据不应被保存"
        );
    }

    #[test]
    fn blank_display_name_becomes_none() {
        let mut request = valid_request();
        request.display_name = Some("   ".into());

        let account = request.into_account(0).unwrap();

        assert!(account.display_name.is_none());
    }

    #[test]
    fn email_is_trimmed() {
        let mut request = valid_request();
        request.email = "  user@163.com  ".into();

        let account = request.into_account(0).unwrap();

        assert_eq!(account.email, "user@163.com");
    }

    // ---------------- 失败路径 ----------------

    #[test]
    fn empty_email_is_rejected() {
        let mut request = valid_request();
        request.email = "   ".into();

        let err = request.into_account(0).unwrap_err();

        assert!(!err.is_retryable());
        assert!(err.requires_user_action());
    }

    #[test]
    fn email_without_at_is_rejected() {
        let mut request = valid_request();
        request.email = "not-an-email".into();

        assert!(request.into_account(0).is_err());
    }

    #[test]
    fn empty_imap_host_is_rejected() {
        let mut request = valid_request();
        request.imap_host = " ".into();

        assert!(request.into_account(0).is_err());
    }

    #[test]
    fn zero_imap_port_is_rejected() {
        let mut request = valid_request();
        request.imap_port = 0;

        assert!(request.into_account(0).is_err());
    }

    #[test]
    fn empty_smtp_host_is_rejected() {
        let mut request = valid_request();
        request.smtp_host = "".into();

        assert!(request.into_account(0).is_err());
    }

    #[test]
    fn zero_smtp_port_is_rejected() {
        let mut request = valid_request();
        request.smtp_port = 0;

        assert!(request.into_account(0).is_err());
    }

    #[test]
    fn empty_username_is_rejected() {
        let mut request = valid_request();
        request.username = " ".into();

        assert!(request.into_account(0).is_err());
    }

    #[test]
    fn non_positive_poll_interval_is_rejected() {
        let mut request = valid_request();
        request.poll_interval_secs = 0;

        assert!(request.into_account(0).is_err());
    }
}
