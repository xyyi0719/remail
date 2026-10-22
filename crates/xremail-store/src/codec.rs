//! 领域类型与数据库文本/JSON 列之间的编解码。
//!
//! 数据库里枚举以 snake_case 文本存储（与 `xremail-core` 的
//! `#[serde(rename_all = "snake_case")]` 一致），地址列表以 JSON 数组存储。
//!
//! 解析失败一律返回 `AppError::Invalid`：**不**静默回退到默认值。
//! 一个不认识的 provider 字符串说明数据与代码已经不一致，
//! 静默兜底会把这种不一致藏起来。

use serde::Serialize;
use serde::de::DeserializeOwned;
use xremail_core::AppError;

use crate::error::StoreResult;

/// 把领域枚举编码为数据库文本。
pub fn enum_to_text<T: Serialize>(value: &T, context: &str) -> StoreResult<String> {
    serde_json::to_string(value)
        .map(|s| s.trim_matches('"').to_string())
        .map_err(|e| AppError::Invalid(format!("{context}: 枚举编码失败: {e}")))
}

/// 从数据库文本解码领域枚举。
pub fn enum_from_text<T: DeserializeOwned>(text: &str, context: &str) -> StoreResult<T> {
    let quoted = format!("\"{text}\"");
    serde_json::from_str::<T>(&quoted)
        .map_err(|e| AppError::Invalid(format!("{context}: 未知枚举值 {text:?}: {e}")))
}

/// 把地址列表编码为 JSON 文本。
pub fn addresses_to_json<T: Serialize>(value: &T, context: &str) -> StoreResult<String> {
    serde_json::to_string(value)
        .map_err(|e| AppError::Invalid(format!("{context}: 地址列表编码失败: {e}")))
}

/// 从 JSON 文本解码地址列表。
pub fn addresses_from_json<T: DeserializeOwned>(text: &str, context: &str) -> StoreResult<T> {
    serde_json::from_str(text)
        .map_err(|e| AppError::Invalid(format!("{context}: 地址列表 JSON 解析失败: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use xremail_core::model::{ProviderKind, TlsMode};

    #[test]
    fn enum_round_trip_uses_snake_case() {
        let text = enum_to_text(&ProviderKind::Netease163, "provider").unwrap();
        // 显式 serde 名，不依赖 rename_all 自动推导：
        // 变体改名不应改变落库文本。
        assert_eq!(text, "netease_163");

        let back: ProviderKind = enum_from_text(&text, "provider").unwrap();
        assert_eq!(back, ProviderKind::Netease163);
    }

    #[test]
    fn every_provider_round_trips() {
        for provider in [
            ProviderKind::Netease163,
            ProviderKind::Netease126,
            ProviderKind::NeteaseEnterprise,
            ProviderKind::Tencent,
            ProviderKind::Wo,
            ProviderKind::Google,
            ProviderKind::Exchange,
            ProviderKind::Generic,
        ] {
            let text = enum_to_text(&provider, "provider").unwrap();
            let back: ProviderKind = enum_from_text(&text, "provider").unwrap();
            assert_eq!(back, provider, "provider {provider:?} 往返失败");
        }
    }

    #[test]
    fn unknown_enum_is_invalid_not_silently_defaulted() {
        let err = enum_from_text::<ProviderKind>("not_a_provider", "provider").unwrap_err();

        assert!(!err.is_retryable());
        assert!(err.requires_user_action());
    }

    #[test]
    fn tls_mode_round_trip() {
        let text = enum_to_text(&TlsMode::Implicit, "tls").unwrap();
        assert_eq!(text, "implicit");

        let back: TlsMode = enum_from_text(&text, "tls").unwrap();
        assert_eq!(back, TlsMode::Implicit);
    }
}
