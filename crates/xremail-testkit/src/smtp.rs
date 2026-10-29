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
    /// 拒收的收件人：地址（小写）→ 响应码。
    ///
    /// 用来模拟「用户不存在」「被拉黑」这类**服务器明确拒收**的场景。
    /// 服务器明确拒收时，客户端**不能**继续往下走（更不能发 `DATA`）。
    pub rejected_recipients: Vec<(String, u16)>,
    /// 收完 `DATA` 后**丢弃最终应答**，直接断开连接。
    ///
    /// 这是「不确定」场景的来源：命令已经发出去了，但客户端没拿到最终应答，
    /// 于是**不知道服务器到底收没收收**——也就不能盲重发（可能已经发过了）。
    pub drop_data_response: bool,
    /// 单封邮件大小上限（字节）。
    ///
    /// `None` = 以 EHLO 宣告的 `SIZE` 为准；EHLO 也没宣告 `SIZE` 时 = 不限。
    /// 宣告了上限就**真的**拒收超限邮件——否则「宣告 SIZE」只是摆设。
    pub max_message_size: Option<u64>,
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
            rejected_recipients: Vec::new(),
            drop_data_response: false,
            max_message_size: None,
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

    /// 拒收某个收件人（响应码自选，如 `550`）。
    pub fn rejecting_recipient(mut self, address: &str, code: u16) -> Self {
        self.rejected_recipients
            .push((address.to_ascii_lowercase(), code));
        self
    }

    /// 收完 `DATA` 后**不回最终应答**，直接断开连接。
    ///
    /// 见 [`SmtpServerConfig::drop_data_response`]。
    pub fn dropping_data_response(mut self) -> Self {
        self.drop_data_response = true;
        self
    }

    /// 声明并**强制执行**单封邮件大小上限（字节）。
    ///
    /// EHLO 里的 `SIZE` 会同步成同一个值：宣告什么就执行什么，
    /// 不搞「嘴上说 100、实际放行 1MB」。
    pub fn with_max_message_size(mut self, bytes: u64) -> Self {
        self.max_message_size = Some(bytes);
        self.ehlo_extensions = with_size_extension(&self.ehlo_extensions, bytes);
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
    state: ServerState,
    shutdown: tokio::sync::oneshot::Sender<()>,
    handle: tokio::task::JoinHandle<()>,
}

/// 服务器的共享记账（每个连接都往里写）。
///
/// 单独拎出来是因为连接处理函数已经要传很多参数了；
/// 记账本身是「所有连接共享」的东西，用 `Arc` 传最自然。
#[derive(Clone, Default)]
struct ServerState {
    /// 收到的命令**动词**（大写）。
    commands: Arc<Mutex<Vec<String>>>,
    /// 收到的命令行**原文**（含行终止符）。
    command_lines: Arc<Mutex<Vec<String>>>,
    /// 收到的邮件（已去掉 dot-stuffing，即「邮件本来的样子」）。
    messages: Arc<Mutex<Vec<Vec<u8>>>>,
    /// 邮件的**线上原始字节**（仍带 dot-stuffing）。
    raw_messages: Arc<Mutex<Vec<Vec<u8>>>>,
}

impl ServerState {
    /// 存下一封收到的邮件（正文 + 线上原始字节）。
    fn push_message(&self, logical: Vec<u8>, raw: Vec<u8>) {
        if let Ok(mut log) = self.messages.lock() {
            log.push(logical);
        }
        if let Ok(mut log) = self.raw_messages.lock() {
            log.push(raw);
        }
    }
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
        let state = ServerState::default();
        let state_for_loop = state.clone();

        // 「宣告 SIZE」就得「执行 SIZE」：没有显式上限时，
        // 以 EHLO 里宣告的 SIZE 为准（没宣告 SIZE 就不限）。
        let max_size = config
            .max_message_size
            .or_else(|| parse_size_extension(&config.ehlo_extensions));

        let handle = tokio::spawn(async move {
            tokio::select! {
                _ = shutdown_rx => {}
                _ = accept_loop(listener, acceptor, config, state_for_loop, max_size) => {}
            }
        });

        Ok(Self {
            addr,
            certificate,
            state,
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
        self.state
            .commands
            .lock()
            .map(|c| c.clone())
            .unwrap_or_default()
    }

    /// 收到的命令行**原文**（含行终止符）。
    ///
    /// 与 [`FakeSmtpServer::commands`] 的区别：这里保留参数与行终止符，
    /// 所以能断言「`MAIL FROM` 带没带 `BODY=8BITMIME`」
    /// 「线上行终止符确实是 CRLF」这类**线上格式**问题。
    pub fn command_lines(&self) -> Vec<String> {
        self.state
            .command_lines
            .lock()
            .map(|c| c.clone())
            .unwrap_or_default()
    }

    /// 收到的邮件（按收到的顺序）。
    ///
    /// 字节是**去掉 dot-stuffing 之后**的，即「邮件本来的样子」。
    pub fn messages(&self) -> Vec<Vec<u8>> {
        self.state
            .messages
            .lock()
            .map(|m| m.clone())
            .unwrap_or_default()
    }

    /// 最近收到的一封邮件。
    pub fn last_message(&self) -> Option<Vec<u8>> {
        self.state
            .messages
            .lock()
            .ok()
            .and_then(|m| m.last().cloned())
    }

    /// 邮件的**线上原始字节**（仍带 dot-stuffing）。
    ///
    /// 与 [`FakeSmtpServer::messages`] 配对使用：
    /// 前者看「服务器最终认定的邮件」，后者看「线上到底写了什么字节」。
    /// dot-stuffing 就只能靠后者验证。
    pub fn raw_messages(&self) -> Vec<Vec<u8>> {
        self.state
            .raw_messages
            .lock()
            .map(|m| m.clone())
            .unwrap_or_default()
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
    state: ServerState,
    max_size: Option<u64>,
) {
    loop {
        let Ok((stream, _peer)) = listener.accept().await else {
            continue;
        };
        let config = config.clone();
        let acceptor = acceptor.clone();
        let state = state.clone();
        tokio::spawn(async move {
            let _ = handle_connection(stream, acceptor, config, state, max_size).await;
        });
    }
}

/// 处理一个连接。
async fn handle_connection(
    stream: TcpStream,
    acceptor: TlsAcceptor,
    config: Arc<SmtpServerConfig>,
    state: ServerState,
    max_size: Option<u64>,
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

    serve(conn, acceptor, config, state, max_size).await;
    Ok(())
}

/// 会话循环。
async fn serve(
    mut conn: ServerConn,
    acceptor: TlsAcceptor,
    config: Arc<SmtpServerConfig>,
    state: ServerState,
    max_size: Option<u64>,
) {
    if conn.write_line(&config.greeting).await.is_err() {
        return;
    }

    let mut tls_upgraded = config.implicit_tls;
    // 一次投递事务的状态。
    let mut tx = TransactionState::default();

    loop {
        let Some(raw_line) = conn.read_raw_line_with_terminator().await.ok().flatten() else {
            return;
        };
        // 命令原文留一份（含行终止符）：测试要断言
        // 「`MAIL FROM` 带没带 `BODY=8BITMIME`」「线上行终止符是不是 CRLF」。
        record_line(&state.command_lines, &String::from_utf8_lossy(&raw_line));
        let line = String::from_utf8_lossy(&raw_line).trim().to_string();
        if line.is_empty() {
            continue;
        }

        let (verb, rest) = split_command(&line);
        record(&state.commands, &verb);

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
            "AUTH" => match handle_auth(&mut conn, &config, &rest).await {
                AuthResult::Success => tx.authenticated = true,
                AuthResult::Failed => tx.authenticated = false,
                AuthResult::Abort => return,
            },
            "QUIT" => {
                let _ = conn.write_line("221 Bye").await;
                return;
            }
            // 投递类命令：之前只回一句「250 OK」，那是为了证明
            // 「客户端一次都没发过」。现在要真正实现它们——
            // 提交链路的测试需要服务器**真的**收信、**真的**能拒收。
            "MAIL" => match handle_mail(&config, &tx, &rest, max_size) {
                Ok(()) => {
                    tx.mail_from_seen = true;
                    tx.rcpt_count = 0;
                    let _ = conn.write_line("250 2.1.0 Ok").await;
                }
                Err(reply) => {
                    let _ = conn.write_line(&reply).await;
                }
            },
            "RCPT" => match handle_rcpt(&config, &tx, &rest) {
                Ok(()) => {
                    tx.rcpt_count += 1;
                    let _ = conn.write_line("250 2.1.5 Ok").await;
                }
                Err(reply) => {
                    let _ = conn.write_line(&reply).await;
                }
            },
            "DATA" => {
                if !tx.authenticated && !config.auth_mechanisms.is_empty() {
                    let _ = conn.write_line("530 5.7.0 Authentication required").await;
                    continue;
                }
                if !tx.mail_from_seen {
                    let _ = conn.write_line("503 5.5.1 Error: need MAIL command").await;
                    continue;
                }
                if tx.rcpt_count == 0 {
                    let _ = conn.write_line("503 5.5.1 Error: need RCPT command").await;
                    continue;
                }
                if conn
                    .write_line("354 End data with <CR><LF>.<CR><LF>")
                    .await
                    .is_err()
                {
                    return;
                }
                // 读正文，直到单独一行的 `.`。
                let Some((logical, raw)) = conn.read_data().await.ok().flatten() else {
                    return;
                };
                // 宣告了 `SIZE` 就得**真的执行**：超限的邮件直接拒收。
                if let Some(limit) = max_size
                    && logical.len() as u64 > limit
                {
                    let _ = conn
                        .write_line("552 5.3.4 Message size exceeds fixed limit")
                        .await;
                    continue;
                }
                state.push_message(logical, raw);
                if config.drop_data_response {
                    // 收下了，但**不回最终应答**就把连接断了：
                    // 客户端于是不知道这封信到底有没有被收下——
                    // 这正是「不确定」场景的来源。
                    return;
                }
                let _ = conn.write_line("250 2.0.0 Ok: queued as 00000001").await;
            }
            "RSET" => {
                tx.mail_from_seen = false;
                tx.rcpt_count = 0;
                let _ = conn.write_line("250 2.0.0 Ok").await;
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

/// 一次投递事务的状态。
#[derive(Debug, Default)]
struct TransactionState {
    /// 已认证成功。
    authenticated: bool,
    /// 收到过 `MAIL FROM`。
    mail_from_seen: bool,
    /// 收到过几个 `RCPT TO`。
    rcpt_count: usize,
}

/// `AUTH` 的处理结果。
enum AuthResult {
    /// 认证成功。
    Success,
    /// 认证失败（连接继续，但状态是「未认证」）。
    Failed,
    /// 连接应当关闭。
    Abort,
}

/// 处理 `MAIL FROM:<addr> [参数]`，返回要回给客户端的应答。
///
/// `Err` 里放的是**直接回给客户端的应答原文**——
/// 服务器要拒收时，得给一个像样的理由（真实服务器会给增强状态码）。
fn handle_mail(
    config: &SmtpServerConfig,
    tx: &TransactionState,
    rest: &str,
    max_size: Option<u64>,
) -> Result<(), String> {
    if !tx.authenticated && !config.auth_mechanisms.is_empty() {
        return Err("530 5.7.0 Authentication required".to_string());
    }

    let Some(address) = extract_address(rest) else {
        return Err("501 5.5.4 Syntax: MAIL FROM:<address>".to_string());
    };
    if address.is_empty() {
        return Err("501 5.5.4 Syntax: MAIL FROM:<address>".to_string());
    }

    // RFC 1870：客户端可以先声明邮件大小；超限就直接拒收，别浪费带宽。
    if let Some(limit) = max_size
        && let Some(declared) = declared_size(rest)
        && declared > limit
    {
        return Err("552 5.3.4 Message size exceeds fixed limit".to_string());
    }

    Ok(())
}

/// 处理 `RCPT TO:<addr>`，返回要回给客户端的应答。
fn handle_rcpt(config: &SmtpServerConfig, tx: &TransactionState, rest: &str) -> Result<(), String> {
    if !tx.authenticated && !config.auth_mechanisms.is_empty() {
        return Err("530 5.7.0 Authentication required".to_string());
    }

    let Some(address) = extract_address(rest) else {
        return Err("501 5.5.4 Syntax: RCPT TO:<address>".to_string());
    };

    // 注入的拒收：模拟「用户不存在」「被拉黑」这类**服务器明确拒收**。
    for (rejected, code) in &config.rejected_recipients {
        if rejected.eq_ignore_ascii_case(address) {
            return Err(format!(
                "{code} 5.1.1 <{address}>: Recipient address rejected: User unknown"
            ));
        }
    }

    Ok(())
}

/// 从 `FROM:<addr> [参数]` / `TO:<addr>` 里取出尖括号中的地址。
fn extract_address(rest: &str) -> Option<&str> {
    let start = rest.find('<')? + 1;
    let end = rest[start..].find('>')? + start;
    Some(rest[start..end].trim())
}

/// 从 `MAIL FROM` 的参数里读出 `SIZE=<n>`（RFC 1870）。
fn declared_size(rest: &str) -> Option<u64> {
    rest.split_whitespace()
        .filter_map(|token| token.split_once('='))
        .find(|(key, _)| key.eq_ignore_ascii_case("SIZE"))
        .and_then(|(_, value)| value.parse().ok())
}

/// 从 EHLO 扩展清单里读出 `SIZE <n>`（当作服务器的单封邮件上限）。
fn parse_size_extension(extensions: &[String]) -> Option<u64> {
    extensions
        .iter()
        .filter_map(|ext| {
            let mut parts = ext.split_whitespace();
            let keyword = parts.next()?;
            if !keyword.eq_ignore_ascii_case("SIZE") {
                return None;
            }
            parts.next()?.parse().ok()
        })
        .next()
}

/// 把 EHLO 扩展清单里的 `SIZE` 换成新值（没有就追加）。
fn with_size_extension(extensions: &[String], bytes: u64) -> Vec<String> {
    let mut out: Vec<String> = extensions
        .iter()
        .filter(|ext| {
            !ext.split_whitespace()
                .next()
                .unwrap_or_default()
                .eq_ignore_ascii_case("SIZE")
        })
        .cloned()
        .collect();
    out.push(format!("SIZE {bytes}"));
    out
}

/// 处理 `AUTH`。
async fn handle_auth(conn: &mut ServerConn, config: &SmtpServerConfig, rest: &str) -> AuthResult {
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
        return AuthResult::Failed;
    }

    // 配置里指定了「只接受某一个机制」→ 其余机制一律拒绝。
    if let Some(accepted) = &config.accepted_auth_mechanism
        && !accepted.eq_ignore_ascii_case(&mechanism)
    {
        let _ = conn
            .write_line("504 Unrecognized authentication type")
            .await;
        return AuthResult::Failed;
    }

    let ok = match mechanism.as_str() {
        "PLAIN" => {
            let payload = match initial {
                Some(payload) => payload,
                None => {
                    if conn.write_line("334 ").await.is_err() {
                        return AuthResult::Abort;
                    }
                    match conn.read_line().await {
                        Ok(Some(line)) => line.trim().to_string(),
                        _ => return AuthResult::Abort,
                    }
                }
            };
            check_plain(&payload, config)
        }
        "LOGIN" => {
            if conn.write_line("334 VXNlcm5hbWU6").await.is_err() {
                return AuthResult::Abort;
            }
            let user_b64 = match conn.read_line().await {
                Ok(Some(line)) => line.trim().to_string(),
                _ => return AuthResult::Abort,
            };
            if conn.write_line("334 UGFzc3dvcmQ6").await.is_err() {
                return AuthResult::Abort;
            }
            let pass_b64 = match conn.read_line().await {
                Ok(Some(line)) => line.trim().to_string(),
                _ => return AuthResult::Abort,
            };
            check_login(&user_b64, &pass_b64, config)
        }
        _ => false,
    };

    if ok {
        let _ = conn.write_line("235 2.7.0 Authentication successful").await;
        AuthResult::Success
    } else {
        let _ = conn
            .write_line("535 5.7.8 Error: authentication failed")
            .await;
        AuthResult::Failed
    }
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

/// 去掉行终止符（`\r\n` 或 `\n`）。
fn strip_line_terminator(line: &[u8]) -> &[u8] {
    let end = line
        .strip_suffix(b"\r\n")
        .map_or(line.len(), |rest| rest.len());
    let end = if end > 0 && line[end - 1] == b'\n' {
        end - 1
    } else {
        end
    };
    &line[..end]
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

/// 记录命令原文（含参数）。
fn record_line(lines: &Arc<Mutex<Vec<String>>>, line: &str) {
    if let Ok(mut log) = lines.lock() {
        log.push(line.to_string());
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
        Ok(self
            .read_raw_line()
            .await?
            .map(|bytes| String::from_utf8_lossy(&bytes).to_string()))
    }

    /// 读一行的**原始字节**（**含**行终止符；`None` = 对端关闭）。
    ///
    /// 命令行要用它来断言「线上行终止符确实是 CRLF」——
    /// 少了终止符就看不出用的是 CRLF 还是裸 LF。
    async fn read_raw_line_with_terminator(&mut self) -> std::io::Result<Option<Vec<u8>>> {
        loop {
            if let Some(offset) = self.buf[self.pos..].iter().position(|&b| b == b'\n') {
                // 含 '\n'，也就是连 CR 一起留着。
                let end = self.pos + offset + 1;
                let line = self.buf[self.pos..end].to_vec();
                self.pos = end;
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

    /// 读一行的**原始字节**（不含行终止符；`None` = 对端关闭）。
    ///
    /// 邮件正文可能是任意字节（8 位内容），所以这里**不能**做 UTF-8 转换——
    /// 转换会把非法字节替换成 `U+FFFD`，那就不是「收到的邮件」了。
    async fn read_raw_line(&mut self) -> std::io::Result<Option<Vec<u8>>> {
        loop {
            if let Some(offset) = self.buf[self.pos..].iter().position(|&b| b == b'\n') {
                let end = self.pos + offset;
                let mut line = self.buf[self.pos..end].to_vec();
                self.pos = end + 1;
                if self.pos >= 4096 {
                    self.buf.drain(..self.pos);
                    self.pos = 0;
                }
                // 去掉行终止符里的 CR（LF 已经在切分时去掉了）。
                if line.last() == Some(&b'\r') {
                    line.pop();
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

    /// 读 DATA 正文，直到单独一行的 `.`（RFC 5321 §4.1.1.4）。
    ///
    /// 返回 `(去掉 dot-stuffing 的正文, 线上原始字节)`；
    /// `None` = 对端在读完之前就断了。
    ///
    /// 「线上原始字节」是**原样收到**的字节（含行终止符、含 dot-stuffing），
    /// 用来断言 dot-stuffing 与 CRLF 确实发生在**线上**；
    /// 「正文」则是去掉 dot-stuffing、行终止符统一成 CRLF 之后的**邮件本来的样子**。
    async fn read_data(&mut self) -> std::io::Result<Option<(Vec<u8>, Vec<u8>)>> {
        let mut logical: Vec<u8> = Vec::new();
        let mut raw: Vec<u8> = Vec::new();

        loop {
            let Some(line) = self.read_raw_line_with_terminator().await? else {
                return Ok(None);
            };
            let content = strip_line_terminator(&line);

            // 单独一行的 `.` = 正文结束（不算正文）。
            if content == b"." {
                break;
            }

            // 线上原始字节：一个字节都不改。
            raw.extend_from_slice(&line);

            // 去 dot-stuffing：线上多出来的那一个点要去掉（RFC 5321 §4.5.2）。
            let unstuffed = if content.first() == Some(&b'.') {
                &content[1..]
            } else {
                content
            };
            logical.extend_from_slice(unstuffed);
            logical.extend_from_slice(b"\r\n");
        }

        Ok(Some((logical, raw)))
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
