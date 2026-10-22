//! Xremail 加密层：版本化密文封装。
//!
//! 职责边界（见 [ARCHITECTURE §2](../../../docs/ARCHITECTURE.md)）：
//! * 封装成熟密码库（argon2id + ChaCha20-Poly1305），**不自行实现密码算法**。
//! * 提供**版本化**的密文封装格式，使日后提高 KDF 成本时仍能解开旧数据。
//! * **不承担**所有平台的钥匙串职责：操作系统钥匙串是 `SecretStore` 的事
//!   （见 `xremail-api`），两者不是同一个接口。
//!
//! ## 封装格式
//!
//! 一次封装由三部分组成：
//!
//! | 部分 | 内容 | 作用 |
//! |---|---|---|
//! | 头 | 版本、KDF 标识、KDF 参数、盐 | 记录"怎么解" |
//! | nonce | 12 字节 | AEAD 随机数 |
//! | 密文 | AEAD 密文 + 认证标签 | 载荷与完整性 |
//!
//! 头与 nonce 一起作为 AEAD 的 **AAD（附加认证数据）**：
//! 它们不被加密，但**被认证**。因此改动版本号、KDF 参数、盐或 nonce
//! 都会导致解密失败，而不是被悄悄接受。
//!
//! ## 口令错误与篡改不区分
//!
//! [`SecretEnvelope::open`] 对"口令不对"和"密文被改过"返回**同一个**错误
//! [`CryptoError::Decrypt`]。若分开，攻击者就能用错误类型当 oracle，
//! 逐字节试探密文结构。
//!
//! ## nonce 不会重复
//!
//! 每次封装都用**全新的随机盐**派生密钥，而 nonce 在同一密钥下唯一才安全。
//! 盐每次都不同 ⇒ 密钥每次都不同 ⇒ nonce 不会跨封装重复。

use chacha20poly1305::KeyInit;
use chacha20poly1305::aead::Aead;

pub mod error;
pub mod kdf;

pub use error::CryptoError;
pub use kdf::{KEY_LEN, KdfId, KdfParams, SALT_LEN};

use crate::kdf::derive_key;

/// nonce 长度。ChaCha20-Poly1305 规定为 12 字节。
pub const NONCE_LEN: usize = 12;

/// 自包含字节格式的魔数。
///
/// 用来在解析时**快速排除**根本不是本格式的数据。
pub const MAGIC: [u8; 4] = *b"XRS1";

/// 自包含字节格式的头部长度。
///
/// `MAGIC(4) + version(1) + kdf(1) + 参数(16) + salt(16) + nonce(12) + 密文长度(4)`
const HEADER_LEN: usize = 4 + 1 + 1 + 4 * 4 + SALT_LEN + NONCE_LEN + 4;

/// 当前密文封装版本。
pub const ENVELOPE_VERSION: u8 = 1;

/// 认证标签长度。ChaCha20-Poly1305 规定为 16 字节。
const TAG_LEN: usize = 16;

/// 一个版本化的密文封装。
///
/// 字段与数据库 `local_secret` 表的列一一对应
/// （`version` / `kdf_params` / `salt` / `nonce` / `ciphertext`），
/// 见 [`SecretEnvelope::from_parts`] 与 [`SecretEnvelope::to_parts`]。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SecretEnvelope {
    version: u8,
    kdf: KdfId,
    kdf_params: KdfParams,
    salt: [u8; SALT_LEN],
    nonce: [u8; NONCE_LEN],
    /// AEAD 密文，末尾附带认证标签。
    ciphertext: Vec<u8>,
}

impl SecretEnvelope {
    /// 用口令封装一段明文。
    ///
    /// 每次调用都会生成全新的随机盐与 nonce。
    pub fn seal(
        plaintext: &[u8],
        passphrase: &[u8],
        params: KdfParams,
    ) -> Result<Self, CryptoError> {
        params.validate()?;

        let mut salt = [0u8; SALT_LEN];
        fill_random(&mut salt)?;

        let mut nonce = [0u8; NONCE_LEN];
        fill_random(&mut nonce)?;

        let key = derive_key(passphrase, &salt, params)?;

        let aad = build_aad(ENVELOPE_VERSION, KdfId::Argon2id, &params, &salt, &nonce);

        let cipher = chacha20poly1305::ChaCha20Poly1305::new_from_slice(key.as_slice())
            .map_err(|_| CryptoError::Kdf("派生密钥长度不合法".into()))?;

        let ciphertext = cipher
            .encrypt(
                &chacha20poly1305::Nonce::from(nonce),
                chacha20poly1305::aead::Payload {
                    msg: plaintext,
                    aad: &aad,
                },
            )
            .map_err(|e| CryptoError::Encrypt(e.to_string()))?;

        Ok(Self {
            version: ENVELOPE_VERSION,
            kdf: KdfId::Argon2id,
            kdf_params: params,
            salt,
            nonce,
            ciphertext,
        })
    }

    /// 解开封装，取回明文。
    ///
    /// 口令错误与密文被篡改都返回 [`CryptoError::Decrypt`]，不区分。
    pub fn open(&self, passphrase: &[u8]) -> Result<Vec<u8>, CryptoError> {
        if self.version != ENVELOPE_VERSION {
            return Err(CryptoError::UnsupportedVersion(self.version));
        }

        let kdf = KdfId::from_u8(self.kdf.as_u8())
            .ok_or(CryptoError::UnsupportedVersion(self.version))?;

        // 密文必须至少包含认证标签，否则不可能是合法的 AEAD 输出。
        if self.ciphertext.len() < TAG_LEN {
            return Err(CryptoError::Decrypt);
        }

        let key = derive_key(passphrase, &self.salt, self.kdf_params)?;

        let aad = build_aad(self.version, kdf, &self.kdf_params, &self.salt, &self.nonce);

        let cipher = chacha20poly1305::ChaCha20Poly1305::new_from_slice(key.as_slice())
            .map_err(|_| CryptoError::Decrypt)?;

        // 解密失败（认证标签不匹配）与口令错误在此汇合成同一个错误。
        cipher
            .decrypt(
                &chacha20poly1305::Nonce::from(self.nonce),
                chacha20poly1305::aead::Payload {
                    msg: &self.ciphertext,
                    aad: &aad,
                },
            )
            .map_err(|_| CryptoError::Decrypt)
    }

    /// 用新的 KDF 参数重新封装。
    ///
    /// 用于提高 KDF 成本：旧封装仍可用旧参数解开，新封装用新参数。
    pub fn rekey(&self, passphrase: &[u8], new_params: KdfParams) -> Result<Self, CryptoError> {
        let plaintext = self.open(passphrase)?;
        Self::seal(&plaintext, passphrase, new_params)
    }

    /// 封装的版本号。
    pub fn version(&self) -> u8 {
        self.version
    }

    /// KDF 算法标识。
    pub fn kdf(&self) -> KdfId {
        self.kdf
    }

    /// KDF 参数。
    pub fn kdf_params(&self) -> KdfParams {
        self.kdf_params
    }

    /// 盐。
    pub fn salt(&self) -> &[u8; SALT_LEN] {
        &self.salt
    }

    /// nonce。
    pub fn nonce(&self) -> &[u8; NONCE_LEN] {
        &self.nonce
    }

    /// 密文（含认证标签）。
    pub fn ciphertext(&self) -> &[u8] {
        &self.ciphertext
    }

    /// 这份封装的 KDF 参数是否比 `desired` 弱，因而应当重新封装。
    pub fn needs_rekey(&self, desired: &KdfParams) -> bool {
        self.kdf_params.is_weaker_than(desired)
    }

    /// 从数据库各列还原封装。
    ///
    /// 供 `xremail-store` 的 `local_secret` 表使用。
    pub fn from_parts(
        version: u8,
        kdf: KdfId,
        kdf_params: KdfParams,
        salt: &[u8],
        nonce: &[u8],
        ciphertext: Vec<u8>,
    ) -> Result<Self, CryptoError> {
        let salt: [u8; SALT_LEN] = salt
            .try_into()
            .map_err(|_| CryptoError::InvalidParams(format!("盐必须是 {SALT_LEN} 字节")))?;
        let nonce: [u8; NONCE_LEN] = nonce
            .try_into()
            .map_err(|_| CryptoError::InvalidParams(format!("nonce 必须是 {NONCE_LEN} 字节")))?;

        Ok(Self {
            version,
            kdf,
            kdf_params,
            salt,
            nonce,
            ciphertext,
        })
    }

    /// 拆成数据库各列。
    pub fn to_parts(&self) -> (u8, KdfId, KdfParams, [u8; SALT_LEN], [u8; NONCE_LEN], &[u8]) {
        (
            self.version,
            self.kdf,
            self.kdf_params,
            self.salt,
            self.nonce,
            &self.ciphertext,
        )
    }

    /// 序列化为**自包含**的字节。
    ///
    /// 与 [`SecretEnvelope::to_parts`] 的区别：这里把头与密文打包成一段连续字节，
    /// 可以作为**不透明载荷**在设备之间搬运（L2 同步要用），
    /// 接收方不需要知道内部结构。
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(HEADER_LEN + self.ciphertext.len());
        out.extend_from_slice(&MAGIC);
        out.push(self.version);
        out.push(self.kdf.as_u8());
        out.extend_from_slice(&self.kdf_params.m_cost.to_be_bytes());
        out.extend_from_slice(&self.kdf_params.t_cost.to_be_bytes());
        out.extend_from_slice(&self.kdf_params.p_cost.to_be_bytes());
        out.extend_from_slice(&self.kdf_params.output_len.to_be_bytes());
        out.extend_from_slice(&self.salt);
        out.extend_from_slice(&self.nonce);
        out.extend_from_slice(&(self.ciphertext.len() as u32).to_be_bytes());
        out.extend_from_slice(&self.ciphertext);
        out
    }

    /// 解析自包含字节格式。
    ///
    /// 解析只做**结构**校验，不涉及密钥：它不解密任何东西。
    /// 长度不对、魔数不匹配都会返回 [`CryptoError::Malformed`]。
    pub fn parse(bytes: &[u8]) -> Result<Self, CryptoError> {
        if bytes.len() < HEADER_LEN {
            return Err(CryptoError::Malformed(format!(
                "数据长度 {} 小于头部长度 {HEADER_LEN}",
                bytes.len()
            )));
        }
        if bytes[..4] != MAGIC {
            return Err(CryptoError::Malformed("魔数不匹配".into()));
        }

        let version = bytes[4];
        if version != ENVELOPE_VERSION {
            return Err(CryptoError::UnsupportedVersion(version));
        }

        let kdf = KdfId::from_u8(bytes[5]).ok_or(CryptoError::Malformed(format!(
            "未知 KDF 标识 {}",
            bytes[5]
        )))?;

        let u32_at = |offset: usize| -> u32 {
            u32::from_be_bytes([
                bytes[offset],
                bytes[offset + 1],
                bytes[offset + 2],
                bytes[offset + 3],
            ])
        };

        let params = KdfParams {
            m_cost: u32_at(6),
            t_cost: u32_at(10),
            p_cost: u32_at(14),
            output_len: u32_at(18),
        };

        let salt: [u8; SALT_LEN] = bytes[22..22 + SALT_LEN]
            .try_into()
            .map_err(|_| CryptoError::Malformed("盐长度不对".into()))?;
        let nonce: [u8; NONCE_LEN] = bytes[22 + SALT_LEN..22 + SALT_LEN + NONCE_LEN]
            .try_into()
            .map_err(|_| CryptoError::Malformed("nonce 长度不对".into()))?;

        let ct_len_offset = 22 + SALT_LEN + NONCE_LEN;
        let ct_len = u32_at(ct_len_offset) as usize;
        let ct_start = ct_len_offset + 4;

        if bytes.len() != ct_start + ct_len {
            return Err(CryptoError::Malformed(format!(
                "密文长度字段为 {ct_len}，但实际剩余 {} 字节",
                bytes.len() - ct_start
            )));
        }

        Ok(Self {
            version,
            kdf,
            kdf_params: params,
            salt,
            nonce,
            ciphertext: bytes[ct_start..].to_vec(),
        })
    }
}

/// 构造 AAD：头 + nonce。
///
/// 被认证但不被加密。改动其中任何一个字节都会让解密失败。
fn build_aad(
    version: u8,
    kdf: KdfId,
    params: &KdfParams,
    salt: &[u8; SALT_LEN],
    nonce: &[u8; NONCE_LEN],
) -> Vec<u8> {
    let mut aad = Vec::with_capacity(2 + 16 + SALT_LEN + NONCE_LEN);
    aad.push(version);
    aad.push(kdf.as_u8());
    aad.extend_from_slice(&params.m_cost.to_be_bytes());
    aad.extend_from_slice(&params.t_cost.to_be_bytes());
    aad.extend_from_slice(&params.p_cost.to_be_bytes());
    aad.extend_from_slice(&params.output_len.to_be_bytes());
    aad.extend_from_slice(salt);
    aad.extend_from_slice(nonce);
    aad
}

/// 从操作系统 CSPRNG 取随机字节。
fn fill_random(buf: &mut [u8]) -> Result<(), CryptoError> {
    use rand::TryRngCore;

    rand::rngs::OsRng
        .try_fill_bytes(buf)
        .map_err(|e| CryptoError::Random(e.to_string()))
}
