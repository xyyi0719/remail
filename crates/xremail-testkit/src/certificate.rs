//! 假 IMAP 服务器的 TLS 证书。
//!
//! 假服务器用**自签**证书：真实客户端不可能信任它。
//! 所以测试要把这张 CA 显式交给客户端（见
//! `xremail_protocol_imap::TlsTrust::Roots`），
//! 而**生产路径**始终用系统根，**没有**「跳过校验」的开关。

use rcgen::{CertificateParams, DnType, KeyPair, SanType};
use rustls::RootCertStore;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};

/// 假服务器的自签证书与私钥。
pub struct ServerCertificate {
    /// 证书链（叶子在前）。
    pub chain: Vec<CertificateDer<'static>>,
    /// 私钥。
    pub private_key: PrivateKeyDer<'static>,
    /// 证书里写的域名（客户端要用它做 SNI 与主机名校验）。
    pub subject_name: String,
}

impl ServerCertificate {
    /// 生成一张自签证书。
    ///
    /// `subject_name` 是客户端连接时用的主机名，必须与证书里的 SAN 一致，
    /// 否则主机名校验会失败（那正是我们要测的东西）。
    pub fn self_signed(subject_name: &str) -> Result<Self, CertError> {
        let key_pair = KeyPair::generate().map_err(|e| CertError::Key(e.to_string()))?;

        let mut params = CertificateParams::new(vec![subject_name.to_string()])
            .map_err(|e| CertError::Params(e.to_string()))?;
        params
            .distinguished_name
            .push(DnType::CommonName, subject_name);
        params
            .distinguished_name
            .push(DnType::OrganizationName, "Xremail Testkit");
        params.subject_alt_names = vec![SanType::DnsName(subject_name.to_string().try_into()?)];

        let cert = params
            .self_signed(&key_pair)
            .map_err(|e| CertError::Generate(e.to_string()))?;

        Ok(Self {
            chain: vec![CertificateDer::from(cert.der().to_vec())],
            private_key: PrivateKeyDer::try_from(key_pair.serialize_der())
                .map_err(|e| CertError::Key(e.to_string()))?,
            subject_name: subject_name.to_string(),
        })
    }

    /// 只信这张证书的根集合。
    ///
    /// 测试用它构造客户端配置：**只**信假服务器，不信系统根。
    pub fn root_store(&self) -> RootCertStore {
        let mut roots = RootCertStore::empty();
        for cert in &self.chain {
            // 自签证书自己是根。
            let _ = roots.add(cert.clone());
        }
        roots
    }
}

/// 证书生成错误。
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CertError {
    /// 密钥生成失败。
    #[error("生成密钥失败: {0}")]
    Key(String),
    /// 证书参数不合法。
    #[error("证书参数不合法: {0}")]
    Params(String),
    /// 证书生成失败。
    #[error("生成证书失败: {0}")]
    Generate(String),
    /// 主机名不合法。
    #[error("主机名不合法: {0}")]
    InvalidHost(String),
}

impl From<rcgen::Error> for CertError {
    fn from(value: rcgen::Error) -> Self {
        CertError::Params(value.to_string())
    }
}
