//! 密钥派生。
//!
//! 只做成熟库的封装，**不自行实现密码算法**
//! （见 [ARCHITECTURE §2](../../../docs/ARCHITECTURE.md) 对 `xremail-crypto` 的边界约定）。
//!
//! 算法固定为 Argon2id（argon2 的推荐变体，兼具抗 GPU 与抗侧信道能力）。

use argon2::{Algorithm, Argon2, Params, Version};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::error::CryptoError;

/// 盐长度。argon2 建议 16 字节。
pub const SALT_LEN: usize = 16;

/// 派生密钥长度。ChaCha20-Poly1305 的密钥固定 32 字节。
pub const KEY_LEN: usize = 32;

/// KDF 算法标识。
///
/// 进版本化封装格式的字节流，因此**不能**依赖 `#[repr]` 的隐式布局，
/// 必须显式指定线上的数值。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum KdfId {
    /// Argon2id，argon2 的推荐变体。
    #[serde(rename = "argon2id")]
    Argon2id,
}

impl KdfId {
    /// 线上字节表示。
    pub const fn as_u8(self) -> u8 {
        match self {
            KdfId::Argon2id => 1,
        }
    }

    /// 由线上字节还原。
    pub const fn from_u8(value: u8) -> Option<Self> {
        match value {
            1 => Some(KdfId::Argon2id),
            _ => None,
        }
    }
}

/// Argon2id 参数。
///
/// 这些参数**必须持久化**在密文封装里：日后提高 KDF 成本时，
/// 仍要用旧参数才能解开旧数据（见 [ARCHITECTURE §8](../../../docs/ARCHITECTURE.md)）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct KdfParams {
    /// 内存代价，单位 KiB。
    pub m_cost: u32,
    /// 迭代次数。
    pub t_cost: u32,
    /// 并行度。
    pub p_cost: u32,
    /// 派生密钥长度（字节）。
    pub output_len: u32,
}

impl KdfParams {
    /// 推荐默认值：19 MiB 内存、2 轮迭代、并行度 1、32 字节输出。
    ///
    /// 与 argon2 crate 的推荐值一致。
    pub const DEFAULT: Self = Self {
        m_cost: 19 * 1024,
        t_cost: 2,
        p_cost: 1,
        output_len: KEY_LEN as u32,
    };

    /// 测试用的低代价参数。
    ///
    /// **不要**在生产路径使用：代价过低会让口令被快速穷举。
    /// 仅供本 crate 的测试与下游 mock 使用。
    pub const FAST: Self = Self {
        m_cost: 8,
        t_cost: 1,
        p_cost: 1,
        output_len: KEY_LEN as u32,
    };

    /// 校验参数是否可用。
    pub fn validate(&self) -> Result<(), CryptoError> {
        if self.output_len as usize != KEY_LEN {
            return Err(CryptoError::InvalidParams(format!(
                "output_len 必须是 {KEY_LEN}，得到 {}",
                self.output_len
            )));
        }

        // argon2 的下界由 crate 自己检查，这里先挡一层，给出更清楚的错误。
        if self.m_cost < 8 * self.p_cost {
            return Err(CryptoError::InvalidParams(format!(
                "m_cost {} 过小（至少为 8 * p_cost = {}）",
                self.m_cost,
                8 * self.p_cost
            )));
        }
        if self.t_cost == 0 {
            return Err(CryptoError::InvalidParams("t_cost 不能为 0".into()));
        }
        if self.p_cost == 0 {
            return Err(CryptoError::InvalidParams("p_cost 不能为 0".into()));
        }

        Ok(())
    }

    /// 是否比 `other` 弱。
    ///
    /// 用于判断存量数据是否需要用更强的参数重新封装。
    pub fn is_weaker_than(&self, other: &KdfParams) -> bool {
        self.m_cost < other.m_cost || self.t_cost < other.t_cost
    }
}

/// 用 Argon2id 派生密钥。
///
/// 返回的密钥在离开作用域时会被清零（`Zeroizing`）。
pub fn derive_key(
    passphrase: &[u8],
    salt: &[u8],
    params: KdfParams,
) -> Result<Zeroizing<Vec<u8>>, CryptoError> {
    params.validate()?;

    if salt.len() < argon2::MIN_SALT_LEN {
        return Err(CryptoError::InvalidParams(format!(
            "盐至少 {SALT_LEN} 字节，收到 {}",
            salt.len()
        )));
    }

    let argon_params = Params::new(
        params.m_cost,
        params.t_cost,
        params.p_cost,
        Some(params.output_len as usize),
    )
    .map_err(|e| CryptoError::InvalidParams(e.to_string()))?;

    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, argon_params);

    let mut key = Zeroizing::new(vec![0u8; params.output_len as usize]);
    argon2
        .hash_password_into(passphrase, salt, key.as_mut())
        .map_err(|e| CryptoError::Kdf(e.to_string()))?;

    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_params_are_valid() {
        assert!(KdfParams::DEFAULT.validate().is_ok());
    }

    #[test]
    fn fast_params_are_valid() {
        assert!(KdfParams::FAST.validate().is_ok());
    }

    #[test]
    fn wrong_output_len_is_rejected() {
        let params = KdfParams {
            output_len: 16,
            ..KdfParams::FAST
        };
        assert!(matches!(
            params.validate(),
            Err(CryptoError::InvalidParams(_))
        ));
    }

    #[test]
    fn too_small_memory_cost_is_rejected() {
        let params = KdfParams {
            m_cost: 1,
            p_cost: 4,
            ..KdfParams::FAST
        };
        assert!(matches!(
            params.validate(),
            Err(CryptoError::InvalidParams(_))
        ));
    }

    #[test]
    fn kdf_is_deterministic_for_same_inputs() {
        let salt = [7u8; SALT_LEN];
        let a = derive_key(b"passphrase", &salt, KdfParams::FAST).unwrap();
        let b = derive_key(b"passphrase", &salt, KdfParams::FAST).unwrap();

        assert_eq!(a.as_slice(), b.as_slice());
    }

    #[test]
    fn different_salt_yields_different_key() {
        let a = derive_key(b"passphrase", &[1u8; SALT_LEN], KdfParams::FAST).unwrap();
        let b = derive_key(b"passphrase", &[2u8; SALT_LEN], KdfParams::FAST).unwrap();

        assert_ne!(a.as_slice(), b.as_slice());
    }

    #[test]
    fn different_passphrase_yields_different_key() {
        let salt = [3u8; SALT_LEN];
        let a = derive_key(b"passphrase-a", &salt, KdfParams::FAST).unwrap();
        let b = derive_key(b"passphrase-b", &salt, KdfParams::FAST).unwrap();

        assert_ne!(a.as_slice(), b.as_slice());
    }

    #[test]
    fn kdf_id_round_trips_through_byte() {
        assert_eq!(
            KdfId::from_u8(KdfId::Argon2id.as_u8()),
            Some(KdfId::Argon2id)
        );
        assert_eq!(KdfId::from_u8(0), None);
        assert_eq!(KdfId::from_u8(2), None);
    }

    #[test]
    fn weaker_params_are_detected() {
        let weak = KdfParams {
            m_cost: 8,
            t_cost: 1,
            ..KdfParams::FAST
        };
        assert!(weak.is_weaker_than(&KdfParams::DEFAULT));
        assert!(!KdfParams::DEFAULT.is_weaker_than(&KdfParams::DEFAULT));
    }
}
