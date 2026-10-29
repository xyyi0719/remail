//! 假 SMTP 服务器。
//!
//! 用途：让 SMTP 协议层与探测工具的测试**不依赖真实邮箱**。
//! 真实邮箱只能复现一种情况；假服务器能复现**各种怪癖**——
//! 比如不支持 STARTTLS、只在 TLS 之后才宣告 `AUTH`、只认 `LOGIN` 不认 `PLAIN`。
//!
//! ## 安全性
//!
//! * 用**自签证书**（复用 [`crate::certificate`]）：真实客户端不可能信任它。
//!   测试要显式把那一张 CA 交给客户端
//!   （`xremail_protocol_smtp::TlsTrust::Roots`）。
//! * 监听地址是 `127.0.0.1`，只对本机可见。
//!
//! ## 命令日志
//!
//! 服务器把**收到的每一条命令**记下来（[`FakeSmtpServer::commands`]）。
//! 这让测试可以断言「客户端一次都没有发过投递类命令」——
//! 这是只读探测的红线，不能靠肉眼看代码。

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::TlsAcceptor;
use tokio_rustls::server::TlsStream;

use crate::certificate::{CertError, ServerCertificate};

/// 假 SMTP 服务器的配置。
#[derive(Debug, Clone)]
pub struct SmtpServerConfig {
    /// `220` 问候语。
    pub greeting: String,
    /// EHLO 返回的扩展清单（原文逐条，如 `SIZE 52428800`）。
    pub ehlo_extensions: Vec<String>,
    /// 是否支持 STARTTLS（支持时 EHLO 里会多一条 `STARTTLS`）。
    pub support_starttls: bool,
    /// 是否用隐式 TLS（连上就是 TLS，465 那种）。
    pub implicit_tls: bool,
    /// 宣告的 AUTH 机制（空 = 不宣告 `AUTH` 扩展）。
    pub auth_mechanisms: Vec<String>,
    /// 实际被接受的认证机制（`None` = 宣告的都接受）。
    pub accepted_auth_mechanism: Option<String>,
    /// AUTH 是否只在 TLS 阶段宣告。
    ///
    /// 真实服务器大多如此（明文阶段不宣告 `AUTH`），
    /// 所以默认 `true`——这让「客户端在握手后**重新 EHLO**」这件事可被测试观察到。
    pub advertise_auth_before_tls: bool,
    /// 期望的用户名。
    pub user: String,
    /// 期望的口令。
    pub password: String,
}

impl Default for SmtpServerConfig {
    fn default() -> Self {
        Self {
            greeting: "220 fake.smtp.test ESMTP FakeSMTP ready".to_string(),
            ehlo_extensions: vec![
                "SIZE 52428800".to_string(),
                "8BITMIME".to_string(),
                "SMTPUTF8".to_string(),
                "PIPELINING".to_string(),
                "DSN".to_string(),
                "CHUNKING".to_string(),
            ],
            support_starttls: true,
            implicit_tls: false,
            auth_mechanisms: vec!["PLAIN".to_string(), "LOGIN".to_string()],
            accepted_auth_mechanism: None,
            advertise_auth_before_tls: false,
            user: "user@example.com".to_string(),
            password: "secret".to_string(),
        }
    }
}

impl SmtpServerConfig {
    /// 默认配置：支持 STARTTLS，宣告 `PLAIN` / `LOGIN`。
    pub fn new() -> Self {
        Self::default()
    }

    /// 明文服务器：**不**宣告 STARTTLS。
    pub fn plaintext() -> Self {
        Self {
            support_starttls: false,
            ..Self::default()
        }
    }

    /// 隐式 TLS 服务器（465 那种）。
    pub fn implicit_tls() -> Self {
        Self {
            implicit_tls: true,
            support_starttls: false,
            ..Self::default()
        }
    }

    /// 指定 EHLO 扩展清单（`STARTTLS` 与 `AUTH` 由服务器自己补）。
    pub fn with_extensions(mut self, extensions: &[&str]) -> Self {
        self.ehlo_extensions = extensions.iter().map(|s| s.to_string()).collect();
        self
    }

    /// 指定问候语。
    pub fn with_greeting(mut self, greeting: &str) -> Self {
        self.greeting = greeting.to_string();
        self
    }

    /// 开关 STARTTLS 支持。
    pub fn with_starttls(mut self, supported: bool) -> Self {
        self.support_starttls = supported;
        self
    }

    /// 指定宣告的认证机制与期望的凭据。
    pub fn with_auth(mut self, mechanisms: &[&str], user: &str, password: &str) -> Self {
        self.auth_mechanisms = mechanisms.iter().map(|s| s.to_string()).collect();
        self.user = user.to_string();
        self.password = password.to_string();
        self
    }

    /// 只接受某一个机制（其余机制回 `504`）。
    pub fn accepting_only(mut self, mechanism: &str) -> Self {
        self.accepted_auth_mechanism = Some(mechanism.to_string());
        self
    }

    /// 完全不宣告 `AUTH`。
    pub fn without_auth(mut self) -> Self {
        self.auth_mechanisms.clear();
        self
    }
}

/// 假 SMTP 服务器错误。
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SmtpServerError {
    /// 监听失败。
    #[error("监听失败: {0}")]
    Listen(String),

    /// TLS 接受器构造失败。
    #[error("TLS 接受器构造失败: {0}")]
    Tls(String),

    /// 证书生成失败。
    #[error("证书生成失败: {0}")]
    Certificate(#[from] CertError),
}

/// 假 SMTP 服务器。
pub struct FakeSmtpServer {
    addr: SocketAddr,
    certificate: Arc<ServerCertificate>,
    commands: Arc<Mutex<Vec<String>>>,
    shutdown: tokio::sync::oneshot::Sender<()>,
    handle: tokio::task::JoinHandle<()>,
}

impl FakeSmtpServer {
    /// 启动一个假服务器。
    pub async fn start(config: SmtpServerConfig) -> Result<Self, SmtpServerError> {
        // 证书里的名字要和客户端连的主机名一致，否则主机名校验会失败
        // ——那正是我们要测的东西。
        let certificate = Arc::new(ServerCertificate::self_signed("localhost")?);

        let acceptor = TlsAcceptor::from(Arc::new(
            rustls::ServerConfig::builder()
                .with_no_client_auth()
                .with_single_cert(
                    certificate.chain.clone(),
                    certificate.private_key.clone_key(),
                )
                .map_err(|e| SmtpServerError::Tls(e.to_string()))?,
        ));

        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .map_err(|e| SmtpServerError::Listen(e.to_string()))?;
        let addr = listener
            .local_addr()
            .map_err(|e| SmtpServerError::Listen(e.to_string()))?;

        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
        let config = Arc::new(config);
        let commands = Arc::new(Mutex::new(Vec::new()));
        let commands_for_loop = commands.clone();

        let handle = tokio::spawn(async move {
            tokio::select! {
                _ = shutdown_rx => {}
                _ = accept_loop(listener, acceptor, config, commands_for_loop) => {}
            }
        });

        Ok(Self {
            addr,
            certificate,
            commands,
            shutdown: shutdown_tx,
            handle,
        })
    }

    /// 监听地址。
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// 端口。
    pub fn port(&self) -> u16 {
        self.addr.port()
    }

    /// 客户端要连的主机名（与证书里的 SAN 一致）。
    pub fn host(&self) -> &'static str {
        "localhost"
    }

    /// 证书。
    pub fn certificate(&self) -> &ServerCertificate {
        &self.certificate
    }

    /// 只信这张假证书的根集合。
    pub fn root_store(&self) -> rustls::RootCertStore {
        self.certificate.root_store()
    }

    /// 只信这张假证书的客户端 TLS 配置。
    pub fn client_config(&self) -> rustls::ClientConfig {
        rustls::ClientConfig::builder()
            .with_root_certificates(self.certificate.root_store())
            .with_no_client_auth()
    }

    /// 把这张自签证书导出成 PEM 文本。
    ///
    /// 测试用得上：客户端要显式信任它，而信任锚通常以 PEM 文件的形式给出。
    pub fn ca_pem(&self) -> String {
        use base64::Engine;
        use base64::engine::general_purpose::STANDARD;

        let der = self.certificate.chain[0].as_ref();
        format!(
            "-----BEGIN CERTIFICATE-----\n{}\n-----END CERTIFICATE-----\n",
            STANDARD.encode(der)
        )
    }

    /// 收到的命令（动词，大写）。
    ///
    /// 测试用它断言「客户端一次都没有发过投递类命令」。
    pub fn commands(&self) -> Vec<String> {
        self.commands.lock().map(|c| c.clone()).unwrap_or_default()
    }

    /// 停掉服务器。
    pub async fn shutdown(self) {
        let _ = self.shutdown.send(());
        let _ = self.handle.await;
    }
}

/// 接受连接循环。
async fn accept_loop(
    listener: TcpListener,
    acceptor: TlsAcceptor,
    config: Arc<SmtpServerConfig>,
    commands: Arc<Mutex<Vec<String>>>,
) {
    loop {
        let Ok((stream, _peer)) = listener.accept().await else {
            continue;
        };
        let config = config.clone();
        let acceptor = acceptor.clone();
        let commands = commands.clone();
        tokio::spawn(async move {
            let _ = handle_connection(stream, acceptor, config, commands).await;
        });
    }
}

/// 处理一个连接。
async fn handle_connection(
    stream: TcpStream,
    acceptor: TlsAcceptor,
    config: Arc<SmtpServerConfig>,
    commands: Arc<Mutex<Vec<String>>>,
) -> std::io::Result<()> {
    let _ = stream.set_nodelay(true);

    let conn = if config.implicit_tls {
        // 隐式 TLS：连上就握手。
        let tls = match acceptor.accept(stream).await {
            Ok(stream) => stream,
            // 握手失败（客户端不信任我们的自签证书）→ 直接断开。
            Err(_) => return Ok(()),
        };
        ServerConn::tls(tls)
    } else {
        ServerConn::plain(stream)
    };

    serve(conn, acceptor, config, commands).await;
    Ok(())
}

/// 会话循环。
async fn serve(
    mut conn: ServerConn,
    acceptor: TlsAcceptor,
    config: Arc<SmtpServerConfig>,
    commands: Arc<Mutex<Vec<String>>>,
) {
    if conn.write_line(&config.greeting).await.is_err() {
        return;
    }

    let mut tls_upgraded = config.implicit_tls;

    loop {
        let Some(line) = conn.read_line().await.ok().flatten() else {
            return;
        };
        let line = line.trim().to_string();
        if line.is_empty() {
            continue;
        }

        let (verb, rest) = split_command(&line);
        record(&commands, &verb);

        match verb.as_str() {
            "EHLO" | "HELO" => {
                for line in ehlo_lines(&config, tls_upgraded) {
                    if conn.write_line(&line).await.is_err() {
                        return;
                    }
                }
            }
            "STARTTLS" => {
                if !config.support_starttls {
                    let _ = conn.write_line("502 Command not implemented").await;
                    continue;
                }
                if conn.write_line("220 Ready to start TLS").await.is_err() {
                    return;
                }
                conn = match conn.into_tls(&acceptor).await {
                    Ok(conn) => conn,
                    Err(_) => return,
                };
                tls_upgraded = true;
            }
            "AUTH" => {
                if !handle_auth(&mut conn, &config, &rest).await {
                    return;
                }
            }
            "QUIT" => {
                let _ = conn.write_line("221 Bye").await;
                return;
            }
            // 投递类命令：服务器**实现**它们，是为了能证明「客户端一次都没发过」。
            // 真收到时会被记进命令日志，测试据此断言只读红线。
            "MAIL" | "RCPT" | "RSET" => {
                let _ = conn.write_line("250 OK").await;
            }
            "DATA" => {
                let _ = conn.write_line("354 End data with <CR><LF>.<CR><LF>").await;
            }
            "NOOP" | "VRFY" | "EXPN" | "HELP" => {
                let _ = conn.write_line("252 OK").await;
            }
            _ => {
                let _ = conn.write_line("500 Command not recognized").await;
            }
        }
    }
}

/// 处理 `AUTH`，返回 `false` 表示连接应当关闭。
async fn handle_auth(conn: &mut ServerConn, config: &SmtpServerConfig, rest: &str) -> bool {
    let mut parts = rest.split_whitespace();
    let mechanism = parts.next().unwrap_or("").to_ascii_uppercase();
    let initial = parts.next().map(|s| s.to_string());

    // 服务器没宣告这个机制 → 不接受。
    if !config
        .auth_mechanisms
        .iter()
        .any(|m| m.eq_ignore_ascii_case(&mechanism))
    {
        let _ = conn
            .write_line("504 Unrecognized authentication type")
            .await;
        return true;
    }

    // 配置里指定了「只接受某一个机制」→ 其余机制一律拒绝。
    if let Some(accepted) = &config.accepted_auth_mechanism
        && !accepted.eq_ignore_ascii_case(&mechanism)
    {
        let _ = conn
            .write_line("504 Unrecognized authentication type")
            .await;
        return true;
    }

    let ok = match mechanism.as_str() {
        "PLAIN" => {
            let payload = match initial {
                Some(payload) => payload,
                None => {
                    if conn.write_line("334 ").await.is_err() {
                        return false;
                    }
                    match conn.read_line().await {
                        Ok(Some(line)) => line.trim().to_string(),
                        _ => return false,
                    }
                }
            };
            check_plain(&payload, config)
        }
        "LOGIN" => {
            if conn.write_line("334 VXNlcm5hbWU6").await.is_err() {
                return false;
            }
            let user_b64 = match conn.read_line().await {
                Ok(Some(line)) => line.trim().to_string(),
                _ => return false,
            };
            if conn.write_line("334 UGFzc3dvcmQ6").await.is_err() {
                return false;
            }
            let pass_b64 = match conn.read_line().await {
                Ok(Some(line)) => line.trim().to_string(),
                _ => return false,
            };
            check_login(&user_b64, &pass_b64, config)
        }
        _ => false,
    };

    if ok {
        let _ = conn.write_line("235 2.7.0 Authentication successful").await;
    } else {
        let _ = conn
            .write_line("535 5.7.8 Error: authentication failed")
            .await;
    }
    true
}

/// 校验 `AUTH PLAIN` 的载荷（`\0user\0password`，也可能带 authzid）。
fn check_plain(payload: &str, config: &SmtpServerConfig) -> bool {
    let Ok(decoded) = STANDARD.decode(payload.as_bytes()) else {
        return false;
    };
    let Ok(text) = String::from_utf8(decoded) else {
        return false;
    };

    let parts: Vec<&str> = text.split('\0').collect();
    if parts.len() < 2 {
        return false;
    }
    let user = parts[parts.len() - 2];
    let pass = parts[parts.len() - 1];
    user == config.user && pass == config.password
}

/// 校验 `AUTH LOGIN` 的两段 base64。
fn check_login(user_b64: &str, pass_b64: &str, config: &SmtpServerConfig) -> bool {
    let Ok(user) = STANDARD.decode(user_b64.as_bytes()) else {
        return false;
    };
    let Ok(pass) = STANDARD.decode(pass_b64.as_bytes()) else {
        return false;
    };
    let Ok(user) = String::from_utf8(user) else {
        return false;
    };
    let Ok(pass) = String::from_utf8(pass) else {
        return false;
    };
    user == config.user && pass == config.password
}

/// 生成 EHLO 响应行。
fn ehlo_lines(config: &SmtpServerConfig, tls_upgraded: bool) -> Vec<String> {
    let mut extensions: Vec<String> = config.ehlo_extensions.clone();

    if config.support_starttls
        && !extensions
            .iter()
            .any(|e| e.eq_ignore_ascii_case("STARTTLS"))
    {
        extensions.push("STARTTLS".to_string());
    }

    if !config.auth_mechanisms.is_empty()
        && (tls_upgraded || config.advertise_auth_before_tls)
        && !extensions
            .iter()
            .any(|e| e.to_ascii_uppercase().starts_with("AUTH"))
    {
        extensions.push(format!("AUTH {}", config.auth_mechanisms.join(" ")));
    }

    let identity = "fake.smtp.test Hello";
    if extensions.is_empty() {
        return vec![format!("250 {identity}")];
    }

    let mut lines = Vec::with_capacity(extensions.len() + 1);
    lines.push(format!("250-{identity}"));
    for (index, extension) in extensions.iter().enumerate() {
        if index + 1 == extensions.len() {
            lines.push(format!("250 {extension}"));
        } else {
            lines.push(format!("250-{extension}"));
        }
    }
    lines
}

/// 拆出命令动词与参数。
fn split_command(line: &str) -> (String, String) {
    match line.split_once(' ') {
        Some((verb, rest)) => (verb.to_ascii_uppercase(), rest.trim().to_string()),
        None => (line.to_ascii_uppercase(), String::new()),
    }
}

/// 记录命令。
fn record(commands: &Arc<Mutex<Vec<String>>>, verb: &str) {
    if let Ok(mut log) = commands.lock() {
        log.push(verb.to_string());
    }
}

/// 服务器侧的连接。
///
/// 和客户端一样自己实现按行读写：`STARTTLS` 要把**同一条连接**从明文
/// 升级成 TLS，中间不能有已经读进用户态缓冲区的明文字节。
struct ServerConn {
    inner: Option<Conn>,
    buf: Vec<u8>,
    pos: usize,
}

enum Conn {
    Plain(TcpStream),
    Tls(Box<TlsStream<TcpStream>>),
}

impl ServerConn {
    /// 明文连接。
    fn plain(tcp: TcpStream) -> Self {
        Self {
            inner: Some(Conn::Plain(tcp)),
            buf: Vec::new(),
            pos: 0,
        }
    }

    /// 已握手的 TLS 连接。
    fn tls(tls: TlsStream<TcpStream>) -> Self {
        Self {
            inner: Some(Conn::Tls(Box::new(tls))),
            buf: Vec::new(),
            pos: 0,
        }
    }

    /// 读一行（`None` = 对端关闭）。
    async fn read_line(&mut self) -> std::io::Result<Option<String>> {
        loop {
            if let Some(offset) = self.buf[self.pos..].iter().position(|&b| b == b'\n') {
                let end = self.pos + offset;
                let line = String::from_utf8_lossy(&self.buf[self.pos..end]).to_string();
                self.pos = end + 1;
                if self.pos >= 4096 {
                    self.buf.drain(..self.pos);
                    self.pos = 0;
                }
                return Ok(Some(line));
            }

            let mut chunk = [0u8; 1024];
            let read = {
                let inner = self
                    .inner
                    .as_mut()
                    .ok_or_else(|| std::io::Error::other("连接已关闭"))?;
                match inner {
                    Conn::Plain(tcp) => tcp.read(&mut chunk).await?,
                    Conn::Tls(tls) => tls.read(&mut chunk).await?,
                }
            };

            if read == 0 {
                return Ok(None);
            }
            self.buf.extend_from_slice(&chunk[..read]);
        }
    }

    /// 写一行（自动补 CRLF）。
    async fn write_line(&mut self, line: &str) -> std::io::Result<()> {
        let mut payload = String::with_capacity(line.len() + 2);
        payload.push_str(line);
        payload.push_str("\r\n");
        let bytes = payload.as_bytes();

        let inner = self
            .inner
            .as_mut()
            .ok_or_else(|| std::io::Error::other("连接已关闭"))?;
        match inner {
            Conn::Plain(tcp) => tcp.write_all(bytes).await?,
            Conn::Tls(tls) => tls.write_all(bytes).await?,
        }
        Ok(())
    }

    /// 明文 → TLS 原地升级。
    async fn into_tls(self, acceptor: &TlsAcceptor) -> std::io::Result<Self> {
        let inner = self
            .inner
            .ok_or_else(|| std::io::Error::other("连接已关闭"))?;
        match inner {
            Conn::Plain(tcp) => {
                let tls = acceptor.accept(tcp).await?;
                Ok(Self::tls(tls))
            }
            Conn::Tls(_) => Err(std::io::Error::other("已经是 TLS，不能再升级")),
        }
    }
}
