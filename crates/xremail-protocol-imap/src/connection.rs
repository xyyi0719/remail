//! TLS 连接与登录。
//!
//! ## 安全约定（见 [SECURITY §2.2](../../../docs/SECURITY.md)）
//!
//! * **只走隐式 TLS**（IMAPS 993）。不提供明文模式，也不提供「跳过证书校验」开关。
//! * 使用**系统根证书**校验。系统根不可用时**报错**，不静默改为不校验。
//! * 连接与命令都有**超时**。
//! * 错误信息里**不含**用户名或口令。

use std::sync::Arc;
use std::time::Duration;

use async_imap::Client;
use async_imap::types::Capabilities;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, BufReader};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;
use tokio_rustls::client::TlsStream;

use crate::error::ProtocolError;

/// 协议层使用的流类型。
///
/// 用 `BufReader` 包一层是为了**先读掉服务器问候行**再交给 IMAP 客户端，
/// 这样问候行不会和后续响应混在一起；`BufReader` 同时实现 `AsyncWrite`，
/// 因此不影响命令下发。
pub type ImapStream = BufReader<TlsStream<TcpStream>>;

/// 已登录的 IMAP 会话。
pub type Session = async_imap::Session<ImapStream>;

/// 客户端自报身份（RFC 2971 的 `ID`）。
///
/// **不是可选的礼貌行为**：163 的实测表明，不发 `ID` 时服务器会把
/// `EXAMINE` / `SELECT` 一律回成 `NO ... Unsafe Login`——邮箱根本进不去。
/// 所以只要服务器宣告了 `ID` 能力，就**必须**在登录后发一次。
///
/// 发送时机：LOGIN 成功之后、任何邮箱访问命令之前。
pub const CLIENT_IDENTIFICATION: &[(&str, Option<&str>)] = &[
    ("name", Some("Xremail")),
    ("version", Some(env!("CARGO_PKG_VERSION"))),
];

/// 连接配置。
#[derive(Debug, Clone)]
pub struct ImapConfig {
    /// 服务器主机名。
    pub host: String,
    /// 端口（生产用 993 隐式 TLS）。
    pub port: u16,
    /// 建连超时。
    pub connect_timeout: Duration,
    /// 单条命令的超时。
    pub command_timeout: Duration,
}

impl ImapConfig {
    /// 用主机名构造（端口固定为隐式 TLS 的 993）。
    pub fn imaps(host: impl Into<String>) -> Self {
        Self {
            host: host.into(),
            port: 993,
            connect_timeout: Duration::from_secs(15),
            command_timeout: Duration::from_secs(30),
        }
    }
}

/// TLS 会话信息（脱敏后用于记录）。
///
/// **不包含证书主题 / 签发方 / 有效期**：rustls 不暴露这些，
/// 解析需要额外的 X.509 依赖。证书细节由预认证探测记录在
/// [PROTOCOLS.md](../../../docs/PROTOCOLS.md)；这里只记录
/// “用的是哪个协议版本与套件、证书链是否校验通过”。
#[derive(Debug, Clone)]
pub struct TlsInfo {
    /// 协议版本，如 `TLSv1.3`。
    pub protocol: String,
    /// 协商出的加密套件。
    pub cipher: String,
    /// 证书链与主机名是否校验通过。
    ///
    /// rustls 默认就是校验的；能走到这里说明已通过。
    pub verified: bool,
}

/// 建立 TLS 连接并登录。
///
/// 返回：已登录会话、**认证后**的能力集、TLS 会话信息。
///
/// 能力集一定是拿到了的：登录时优先用 LOGIN 响应里带的，没有就补发一次
/// `CAPABILITY`（因为发不发 `ID` 取决于服务器是否宣告 `ID` 能力）。
pub async fn connect_and_login(
    config: &ImapConfig,
    username: &str,
    password: &str,
) -> Result<(Session, Capabilities, TlsInfo), ProtocolError> {
    let tls_info = connect_tls(config).await?;
    let stream = tls_info.0;
    let info = tls_info.1;

    // 读掉问候行。失败说明对端不是 IMAP 服务器。
    let mut stream = stream;
    let mut greeting = String::new();
    tokio::time::timeout(config.command_timeout, stream.read_line(&mut greeting))
        .await
        .map_err(|_| ProtocolError::Timeout("读取服务器问候行超时".into()))?
        .map_err(|e| ProtocolError::Io(format!("读取问候行失败: {e}")))?;
    if !greeting.to_ascii_uppercase().starts_with("* OK") {
        return Err(ProtocolError::Imap(format!(
            "服务器问候行不是 * OK: {}",
            first_line(&greeting)
        )));
    }

    let client = Client::new(stream);
    let (mut session, capabilities) = tokio::time::timeout(
        config.command_timeout,
        client.login_with_capabilities(username, password),
    )
    .await
    .map_err(|_| ProtocolError::Timeout("登录超时".into()))?
    .map_err(|(err, _client)| {
        // 错误信息里**不带**用户名/口令。
        ProtocolError::Auth(err.to_string())
    })?;

    // 登录后立刻自报身份（若服务器支持）。见 `CLIENT_IDENTIFICATION` 的说明：
    // 缺这一步，163 会拒绝所有邮箱访问命令。
    let capabilities = match capabilities {
        Some(caps) => caps,
        None => session
            .capabilities()
            .await
            .map_err(|e| ProtocolError::Imap(format!("读取能力集失败: {e}")))?,
    };

    if capabilities.has_str("ID") {
        tokio::time::timeout(
            config.command_timeout,
            session.id(CLIENT_IDENTIFICATION.iter().copied()),
        )
        .await
        .map_err(|_| ProtocolError::Timeout("发送 ID 超时".into()))?
        .map_err(|e| ProtocolError::Imap(format!("发送客户端标识失败: {e}")))?;
    }

    Ok((session, capabilities, info))
}

/// 建立 TLS 连接，返回流与会话信息。
async fn connect_tls(config: &ImapConfig) -> Result<(ImapStream, TlsInfo), ProtocolError> {
    let tls_connector = build_tls_connector()?;

    let tcp = tokio::time::timeout(
        config.connect_timeout,
        TcpStream::connect((config.host.as_str(), config.port)),
    )
    .await
    .map_err(|_| ProtocolError::Timeout(format!("连接 {}:{} 超时", config.host, config.port)))?
    .map_err(|e| {
        ProtocolError::Connect(format!("连接 {}:{} 失败: {e}", config.host, config.port))
    })?;

    // Nagle 会给 IMAP 带来延迟。
    let _ = tcp.set_nodelay(true);

    let server_name = rustls::pki_types::ServerName::try_from(config.host.clone())
        .map_err(|e| ProtocolError::Tls(format!("主机名不合法: {e}")))?;

    let stream = tokio::time::timeout(
        config.connect_timeout,
        tls_connector.connect(server_name, tcp),
    )
    .await
    .map_err(|_| ProtocolError::Timeout("TLS 握手超时".into()))?
    .map_err(|e| ProtocolError::Tls(format!("TLS 握手失败: {e}")))?;

    let info = tls_info(&stream);
    Ok((BufReader::new(stream), info))
}

/// 构造 TLS 连接器：只用系统根证书。
fn build_tls_connector() -> Result<TlsConnector, ProtocolError> {
    let mut roots = rustls::RootCertStore::empty();
    // `load_native_certs` 返回的是一个**结构体**（certs + errors），
    // 不是 Result：部分证书加载失败时仍会返回已加载的那些。
    // 这里只要有任何一个错误就**整体失败**——宁可报错，也不静默少信任一些根。
    let loaded = rustls_native_certs::load_native_certs();
    if !loaded.errors.is_empty() {
        return Err(ProtocolError::Tls(format!(
            "读取系统根证书时出错（{} 项）",
            loaded.errors.len()
        )));
    }
    if loaded.certs.is_empty() {
        // 系统根为空 ⇒ 无法校验证书。此时**必须失败**，
        // 不能退化成「不校验」。
        return Err(ProtocolError::Tls(
            "系统根证书为空，无法校验服务端证书".into(),
        ));
    }
    for cert in loaded.certs {
        roots
            .add(cert)
            .map_err(|e| ProtocolError::Tls(format!("系统根证书无效: {e}")))?;
    }

    let config = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();

    Ok(TlsConnector::from(Arc::new(config)))
}

/// 提取 TLS 会话信息。
fn tls_info(stream: &TlsStream<TcpStream>) -> TlsInfo {
    let (_io, session) = stream.get_ref();

    TlsInfo {
        protocol: session
            .protocol_version()
            .map(|p| format!("{p:?}"))
            .unwrap_or_else(|| "未知".into()),
        cipher: session
            .negotiated_cipher_suite()
            .map(|c| format!("{c:?}"))
            .unwrap_or_else(|| "未知".into()),
        // rustls 默认强制校验证书链与主机名；握手成功即已通过。
        verified: true,
    }
}

/// 取第一行（用于错误信息，避免把整个响应塞进消息）。
fn first_line(text: &str) -> String {
    text.lines().next().unwrap_or("").trim().to_string()
}

/// 供其他模块复用的流读写约束。
pub trait Io: AsyncRead + AsyncWrite + Unpin {}
impl<T: AsyncRead + AsyncWrite + Unpin> Io for T {}
