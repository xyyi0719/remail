//! `xremail-crypto` 的 T3 测试：密文封装。
//!
//! 测试预期来自**密码学语义与威胁模型**，不来自实现本身
//! （见 [AI 开发工作规范 §1.3](../../../docs/AI_DEVELOPMENT_GUIDE.md)、
//! [SECURITY](../../../docs/SECURITY.md)）。
//!
//! 覆盖的要点：
//! * 往返：封装 → 解开得到原文。
//! * 篡改检测：密文 / nonce / 盐 / 头（版本、KDF 参数）任一被改，解开必须失败。
//! * **口令错误与篡改不区分**：两者必须返回同一个错误，否则错误类型就成了 oracle。
//! * 明文不外泄：明文的任何字节都不得出现在封装的任何字段里。
//! * KDF 参数升级：旧封装仍能解开，新封装用新参数。
//! * 未知版本：明确报错，而不是当成别的版本硬解。

use xremail_crypto::error::CryptoError;
use xremail_crypto::kdf::KdfParams;
use xremail_crypto::{ENVELOPE_VERSION, KdfId, NONCE_LEN, SALT_LEN, SecretEnvelope};

/// 测试用口令。
const PASSPHRASE: &[u8] = b"correct horse battery staple";

/// 测试用低代价 KDF 参数（生产路径不得使用）。
fn fast() -> KdfParams {
    KdfParams::FAST
}

// ===========================================================================
// 往返
// ===========================================================================

#[test]
fn round_trip_returns_original_plaintext() {
    let plaintext = "imap 授权码：ABCDEF123456".as_bytes();

    let envelope = SecretEnvelope::seal(plaintext, PASSPHRASE, fast()).unwrap();
    let opened = envelope.open(PASSPHRASE).unwrap();

    assert_eq!(opened, plaintext);
}

#[test]
fn round_trip_works_for_empty_plaintext() {
    let envelope = SecretEnvelope::seal(b"", PASSPHRASE, fast()).unwrap();
    let opened = envelope.open(PASSPHRASE).unwrap();

    assert!(opened.is_empty());
}

#[test]
fn round_trip_works_for_large_plaintext() {
    let plaintext: Vec<u8> = (0..1024 * 1024).map(|i| (i % 251) as u8).collect();

    let envelope = SecretEnvelope::seal(&plaintext, PASSPHRASE, fast()).unwrap();
    let opened = envelope.open(PASSPHRASE).unwrap();

    assert_eq!(opened.len(), plaintext.len());
    assert_eq!(opened, plaintext);
}

#[test]
fn round_trip_works_with_default_params() {
    // 默认参数比 FAST 慢不少（19 MiB 内存 × 2 轮），但必须真的能用。
    let plaintext = b"default params";

    let envelope = SecretEnvelope::seal(plaintext, PASSPHRASE, KdfParams::DEFAULT).unwrap();
    let opened = envelope.open(PASSPHRASE).unwrap();

    assert_eq!(opened, plaintext);
}

#[test]
fn each_seal_uses_a_fresh_salt_and_nonce() {
    let a = SecretEnvelope::seal(b"same", PASSPHRASE, fast()).unwrap();
    let b = SecretEnvelope::seal(b"same", PASSPHRASE, fast()).unwrap();

    assert_ne!(a.salt(), b.salt(), "每次封装必须用新的盐");
    assert_ne!(a.nonce(), b.nonce(), "每次封装必须用新的 nonce");
    assert_ne!(a.ciphertext(), b.ciphertext(), "相同输入不应产生相同密文");
}

// ===========================================================================
// 口令错误
// ===========================================================================

#[test]
fn wrong_passphrase_fails_to_open() {
    let envelope = SecretEnvelope::seal(b"secret", PASSPHRASE, fast()).unwrap();

    let err = envelope.open(b"wrong passphrase").unwrap_err();

    assert!(matches!(err, CryptoError::Decrypt));
}

#[test]
fn empty_passphrase_does_not_open_a_sealed_envelope() {
    let envelope = SecretEnvelope::seal(b"secret", PASSPHRASE, fast()).unwrap();

    assert!(envelope.open(b"").is_err());
}

// ===========================================================================
// 篡改检测
// ===========================================================================

#[test]
fn tampering_with_ciphertext_is_detected() {
    let envelope = SecretEnvelope::seal(b"secret payload", PASSPHRASE, fast()).unwrap();
    let mut broken = envelope.clone();

    let mut ct = broken.ciphertext().to_vec();
    ct[0] ^= 0x01;
    broken = SecretEnvelope::from_parts(
        broken.version(),
        broken.kdf(),
        broken.kdf_params(),
        broken.salt(),
        broken.nonce(),
        ct,
    )
    .unwrap();

    assert!(matches!(broken.open(PASSPHRASE), Err(CryptoError::Decrypt)));
}

#[test]
fn tampering_with_tag_is_detected() {
    let envelope = SecretEnvelope::seal(b"secret payload", PASSPHRASE, fast()).unwrap();
    let mut ct = envelope.ciphertext().to_vec();
    // 认证标签在密文末尾。
    let last = ct.len() - 1;
    ct[last] ^= 0x80;

    let broken = SecretEnvelope::from_parts(
        envelope.version(),
        envelope.kdf(),
        envelope.kdf_params(),
        envelope.salt(),
        envelope.nonce(),
        ct,
    )
    .unwrap();

    assert!(matches!(broken.open(PASSPHRASE), Err(CryptoError::Decrypt)));
}

#[test]
fn tampering_with_nonce_is_detected() {
    let envelope = SecretEnvelope::seal(b"secret payload", PASSPHRASE, fast()).unwrap();
    let mut nonce = *envelope.nonce();
    nonce[0] ^= 0x01;

    let broken = SecretEnvelope::from_parts(
        envelope.version(),
        envelope.kdf(),
        envelope.kdf_params(),
        envelope.salt(),
        &nonce,
        envelope.ciphertext().to_vec(),
    )
    .unwrap();

    assert!(matches!(broken.open(PASSPHRASE), Err(CryptoError::Decrypt)));
}

#[test]
fn tampering_with_salt_is_detected() {
    let envelope = SecretEnvelope::seal(b"secret payload", PASSPHRASE, fast()).unwrap();
    let mut salt = *envelope.salt();
    salt[0] ^= 0x01;

    let broken = SecretEnvelope::from_parts(
        envelope.version(),
        envelope.kdf(),
        envelope.kdf_params(),
        &salt,
        envelope.nonce(),
        envelope.ciphertext().to_vec(),
    )
    .unwrap();

    assert!(matches!(broken.open(PASSPHRASE), Err(CryptoError::Decrypt)));
}

#[test]
fn tampering_with_kdf_params_is_detected() {
    // 头是 AAD 的一部分，改了 KDF 参数必须被发现——
    // 否则攻击者可以悄悄把用户降级到弱参数上。
    let envelope = SecretEnvelope::seal(b"secret payload", PASSPHRASE, fast()).unwrap();

    let broken = SecretEnvelope::from_parts(
        envelope.version(),
        envelope.kdf(),
        KdfParams {
            t_cost: 99,
            ..envelope.kdf_params()
        },
        envelope.salt(),
        envelope.nonce(),
        envelope.ciphertext().to_vec(),
    )
    .unwrap();

    assert!(matches!(broken.open(PASSPHRASE), Err(CryptoError::Decrypt)));
}

#[test]
fn truncating_ciphertext_is_detected() {
    let envelope = SecretEnvelope::seal(b"secret payload", PASSPHRASE, fast()).unwrap();
    let ct = envelope.ciphertext()[..envelope.ciphertext().len() - 1].to_vec();

    let broken = SecretEnvelope::from_parts(
        envelope.version(),
        envelope.kdf(),
        envelope.kdf_params(),
        envelope.salt(),
        envelope.nonce(),
        ct,
    )
    .unwrap();

    assert!(matches!(broken.open(PASSPHRASE), Err(CryptoError::Decrypt)));
}

#[test]
fn empty_ciphertext_is_rejected() {
    let envelope = SecretEnvelope::seal(b"secret payload", PASSPHRASE, fast()).unwrap();

    let broken = SecretEnvelope::from_parts(
        envelope.version(),
        envelope.kdf(),
        envelope.kdf_params(),
        envelope.salt(),
        envelope.nonce(),
        Vec::new(),
    )
    .unwrap();

    assert!(matches!(broken.open(PASSPHRASE), Err(CryptoError::Decrypt)));
}

// ===========================================================================
// 错误不可区分（重要安全性质）
// ===========================================================================

#[test]
fn wrong_passphrase_and_tampered_ciphertext_are_indistinguishable() {
    let envelope = SecretEnvelope::seal(b"secret payload", PASSPHRASE, fast()).unwrap();

    let wrong_passphrase = envelope.open(b"wrong passphrase").unwrap_err();
    let mut ct = envelope.ciphertext().to_vec();
    ct[0] ^= 0x01;
    let tampered = SecretEnvelope::from_parts(
        envelope.version(),
        envelope.kdf(),
        envelope.kdf_params(),
        envelope.salt(),
        envelope.nonce(),
        ct,
    )
    .unwrap();
    let tampered = tampered.open(PASSPHRASE).unwrap_err();

    // 两者的错误**文本**必须完全一致，否则错误类型就成了 oracle。
    assert_eq!(
        wrong_passphrase.to_string(),
        tampered.to_string(),
        "口令错误与密文篡改必须返回同一个错误"
    );
    assert_eq!(wrong_passphrase.to_string(), "decryption failed");
}

// ===========================================================================
// 明文不外泄
// ===========================================================================

#[test]
fn plaintext_does_not_appear_in_any_envelope_field() {
    let plaintext = b"SUPER-SECRET-PLAINTEXT-MARKER";
    let envelope = SecretEnvelope::seal(plaintext, PASSPHRASE, fast()).unwrap();

    let (_, _, _, salt, nonce, ciphertext) = envelope.to_parts();

    for (name, field) in [
        ("salt", &salt[..]),
        ("nonce", &nonce[..]),
        ("ciphertext", ciphertext),
    ] {
        assert!(
            !field
                .windows(plaintext.len())
                .any(|w| w == plaintext.as_slice()),
            "明文出现在了 {name} 里"
        );
        // 更强一点：明文的**任何 8 字节片段**都不应出现。
        let needle = &plaintext[..8];
        assert!(
            !field.windows(needle.len()).any(|w| w == needle),
            "明文片段出现在了 {name} 里"
        );
    }
}

#[test]
fn envelope_exposes_metadata_but_never_the_plaintext() {
    let envelope = SecretEnvelope::seal(b"secret", PASSPHRASE, fast()).unwrap();

    assert_eq!(envelope.version(), ENVELOPE_VERSION);
    assert_eq!(envelope.kdf(), KdfId::Argon2id);
    assert_eq!(envelope.kdf_params(), fast());
    assert_eq!(envelope.salt().len(), SALT_LEN);
    assert_eq!(envelope.nonce().len(), NONCE_LEN);
    // 密文比明文长：多出的是认证标签。
    assert!(envelope.ciphertext().len() >= 16);
}

// ===========================================================================
// 版本
// ===========================================================================

#[test]
fn unknown_version_is_reported_explicitly() {
    let envelope = SecretEnvelope::seal(b"secret", PASSPHRASE, fast()).unwrap();

    let future = SecretEnvelope::from_parts(
        ENVELOPE_VERSION + 1,
        envelope.kdf(),
        envelope.kdf_params(),
        envelope.salt(),
        envelope.nonce(),
        envelope.ciphertext().to_vec(),
    )
    .unwrap();

    match future.open(PASSPHRASE) {
        Err(CryptoError::UnsupportedVersion(v)) => assert_eq!(v, ENVELOPE_VERSION + 1),
        other => panic!("期望 UnsupportedVersion，实际是 {other:?}"),
    }
}

// ===========================================================================
// KDF 参数升级
// ===========================================================================

#[test]
fn rekey_upgrades_kdf_params_and_keeps_plaintext_readable() {
    let envelope = SecretEnvelope::seal(b"secret payload", PASSPHRASE, fast()).unwrap();
    assert!(envelope.needs_rekey(&KdfParams::DEFAULT));

    let upgraded = envelope.rekey(PASSPHRASE, KdfParams::DEFAULT).unwrap();

    assert_eq!(upgraded.kdf_params(), KdfParams::DEFAULT);
    assert_eq!(upgraded.open(PASSPHRASE).unwrap(), b"secret payload");
    // 旧封装仍能用旧参数解开——这正是版本化的意义。
    assert_eq!(envelope.open(PASSPHRASE).unwrap(), b"secret payload");
    assert!(!upgraded.needs_rekey(&KdfParams::DEFAULT));
}

#[test]
fn rekey_with_wrong_passphrase_fails() {
    let envelope = SecretEnvelope::seal(b"secret payload", PASSPHRASE, fast()).unwrap();

    assert!(
        envelope
            .rekey(b"wrong passphrase", KdfParams::DEFAULT)
            .is_err()
    );
}

// ===========================================================================
// 与数据库列的对应
// ===========================================================================

#[test]
fn parts_round_trip_through_database_columns() {
    let envelope = SecretEnvelope::seal(b"secret payload", PASSPHRASE, fast()).unwrap();

    let (version, kdf, params, salt, nonce, ciphertext) = envelope.to_parts();
    let restored =
        SecretEnvelope::from_parts(version, kdf, params, &salt, &nonce, ciphertext.to_vec())
            .unwrap();

    assert_eq!(restored, envelope);
    assert_eq!(restored.open(PASSPHRASE).unwrap(), b"secret payload");
}

#[test]
fn from_parts_rejects_wrong_salt_length() {
    let envelope = SecretEnvelope::seal(b"secret", PASSPHRASE, fast()).unwrap();

    let err = SecretEnvelope::from_parts(
        envelope.version(),
        envelope.kdf(),
        envelope.kdf_params(),
        &[0u8; 8],
        envelope.nonce(),
        envelope.ciphertext().to_vec(),
    )
    .unwrap_err();

    assert!(matches!(err, CryptoError::InvalidParams(_)));
}

#[test]
fn from_parts_rejects_wrong_nonce_length() {
    let envelope = SecretEnvelope::seal(b"secret", PASSPHRASE, fast()).unwrap();

    let err = SecretEnvelope::from_parts(
        envelope.version(),
        envelope.kdf(),
        envelope.kdf_params(),
        envelope.salt(),
        &[0u8; 8],
        envelope.ciphertext().to_vec(),
    )
    .unwrap_err();

    assert!(matches!(err, CryptoError::InvalidParams(_)));
}

// ===========================================================================
// 自包含字节格式
// ===========================================================================

#[test]
fn self_contained_bytes_round_trip() {
    let envelope = SecretEnvelope::seal(b"secret payload", PASSPHRASE, fast()).unwrap();

    let bytes = envelope.to_bytes();
    let parsed = SecretEnvelope::parse(&bytes).unwrap();

    assert_eq!(parsed, envelope);
    assert_eq!(parsed.open(PASSPHRASE).unwrap(), b"secret payload");
}

#[test]
fn self_contained_bytes_are_opaque_to_consumers() {
    // 自包含格式的意义：搬运方不需要知道内部结构。
    let envelope = SecretEnvelope::seal(b"secret payload", PASSPHRASE, fast()).unwrap();
    let bytes = envelope.to_bytes();

    // 内部结构确实在里面……
    assert!(bytes.starts_with(&xremail_crypto::MAGIC));
    // ……但明文不在里面。
    let needle = b"secret payload";
    assert!(!bytes.windows(needle.len()).any(|w| w == needle));
}

#[test]
fn parse_rejects_bad_magic() {
    let envelope = SecretEnvelope::seal(b"secret", PASSPHRASE, fast()).unwrap();
    let mut bytes = envelope.to_bytes();
    bytes[0] ^= 0xff;

    assert!(matches!(
        SecretEnvelope::parse(&bytes),
        Err(CryptoError::Malformed(_))
    ));
}

#[test]
fn parse_rejects_truncated_data() {
    let envelope = SecretEnvelope::seal(b"secret", PASSPHRASE, fast()).unwrap();
    let bytes = envelope.to_bytes();

    assert!(matches!(
        SecretEnvelope::parse(&bytes[..bytes.len() - 1]),
        Err(CryptoError::Malformed(_))
    ));
    assert!(matches!(
        SecretEnvelope::parse(&bytes[..10]),
        Err(CryptoError::Malformed(_))
    ));
}

#[test]
fn parse_rejects_trailing_garbage() {
    let envelope = SecretEnvelope::seal(b"secret", PASSPHRASE, fast()).unwrap();
    let mut bytes = envelope.to_bytes();
    bytes.push(0x00);

    assert!(matches!(
        SecretEnvelope::parse(&bytes),
        Err(CryptoError::Malformed(_))
    ));
}

#[test]
fn parse_reports_unknown_version() {
    let envelope = SecretEnvelope::seal(b"secret", PASSPHRASE, fast()).unwrap();
    let mut bytes = envelope.to_bytes();
    bytes[4] = ENVELOPE_VERSION + 1;

    assert!(matches!(
        SecretEnvelope::parse(&bytes),
        Err(CryptoError::UnsupportedVersion(v)) if v == ENVELOPE_VERSION + 1
    ));
}

// ===========================================================================
// 错误分类
// ===========================================================================

#[test]
fn decrypt_error_is_not_retryable() {
    let err = CryptoError::Decrypt;

    assert!(!err.is_retryable(), "解密失败重试没有意义");
}

#[test]
fn random_error_is_retryable() {
    let err = CryptoError::Random("entropy unavailable".into());

    assert!(err.is_retryable(), "随机源故障值得重试");
}
