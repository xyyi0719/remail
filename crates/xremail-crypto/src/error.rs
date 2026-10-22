//! 错误类型。
//!
//! 分类原则（见 [ARCHITECTURE §9](../../../docs/ARCHITECTURE.md)）：
//! 错误按**可恢复原因**分类，并且**不泄露可用于区分的信息**。
//!
//! 最关键的一条：`Decrypt` 是**单一**错误，口令错误与密文被篡改都归到它。
//! 若把两者分开，攻击者就能用"错误类型"当 oracle，
//! 逐字节试探密文的结构——认证标签的存在意义会被削弱。

use thiserror::Error;

/// 加密层错误。
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum CryptoError {
    /// KDF 参数非法（例如内存代价低于下限）。
    #[error("invalid kdf parameters: {0}")]
    InvalidParams(String),

    /// 密钥派生失败。
    #[error("key derivation failed: {0}")]
    Kdf(String),

    /// 随机数生成失败。**不应**被忽略：盐或 nonce 不可预测是安全前提。
    #[error("random source failed: {0}")]
    Random(String),

    /// 加密失败。
    #[error("encryption failed: {0}")]
    Encrypt(String),

    /// 解密或完整性校验失败。
    ///
    /// 口令错误与密文被篡改**共用**这个错误，不加区分。
    #[error("decryption failed")]
    Decrypt,

    /// 数据格式损坏（长度不对、魔数不匹配等）。
    #[error("malformed data: {0}")]
    Malformed(String),

    /// 密文封装版本不认识。
    ///
    /// 单独成一类是安全的：它只说明"这份数据来自另一个版本"，
    /// 不泄露任何与密钥或明文有关的信息。
    #[error("unsupported envelope version: {0}")]
    UnsupportedVersion(u8),
}

impl CryptoError {
    /// 是否值得重试。
    ///
    /// 加密层几乎所有失败都是确定性的：同样的输入重试还是同样的结果。
    /// 唯一可能重试的是随机数源故障。
    pub fn is_retryable(&self) -> bool {
        matches!(self, CryptoError::Random(_))
    }
}
