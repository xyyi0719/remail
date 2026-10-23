//! 假 IMAP 服务器。
//!
//! 用途：让同步引擎、协议层的测试**不依赖真实邮箱**。
//! 真实邮箱只能复现一种情况；假服务器能复现**各种怪癖**——
//! 比如 163 拒绝 `EXAMINE`、要求先发 `ID`、认证前拒绝一切命令。
//!
//! ## 安全性
//!
//! * 用**自签证书**：真实客户端不可能信任它。测试要显式把那一张 CA
//!   交给客户端（`xremail_protocol_imap::TlsTrust::Roots`）。
//! * 监听地址是 `127.0.0.1`，只对本机可见。
//! * 不实现任何写命令的完整语义（`STORE` 等留待同步测试需要时再加）。

use std::net::SocketAddr;
use std::sync::Arc;

use tokio::io::{AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::TlsAcceptor;

use crate::certificate::{CertError, ServerCertificate};
use crate::config::{Fault, MailboxFixture, ServerConfig};
use crate::parser::{Command, quote};

/// 回一行响应（自动加 CRLF）。
///
/// 宏而不是函数：IMAP 响应几乎都要带 tag 与参数，
/// 写成宏能直接用 `{}` 占位，不必在每个调用点包一层 `format!`。
macro_rules! reply {
    ($writer:expr, $($arg:tt)*) => {{
        // 先同步格式化好再 await：`std::fmt::Arguments` 不是 Send，
        // 跨 await 持有它会让整个 future 变成非 Send，tokio::spawn 就用不了了。
        let mut buffer = String::new();
        {
            use std::fmt::Write as _;
            let _ = buffer.write_fmt(format_args!($($arg)*));
        }
        write_line(&mut $writer, &buffer).await?
    }};
}

/// 假服务器错误。
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ServerError {
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

/// 假 IMAP 服务器。
pub struct FakeImapServer {
    addr: SocketAddr,
    certificate: Arc<ServerCertificate>,
    shutdown: tokio::sync::oneshot::Sender<()>,
    handle: tokio::task::JoinHandle<()>,
}

impl FakeImapServer {
    /// 启动一个假服务器。
    pub async fn start(config: ServerConfig) -> Result<Self, ServerError> {
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
                .map_err(|e| ServerError::Tls(e.to_string()))?,
        ));

        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .map_err(|e| ServerError::Listen(e.to_string()))?;
        let addr = listener
            .local_addr()
            .map_err(|e| ServerError::Listen(e.to_string()))?;

        let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
        let config = Arc::new(config);

        let handle = tokio::spawn(async move {
            tokio::select! {
                _ = shutdown_rx => {}
                _ = accept_loop(listener, acceptor, config) => {}
            }
        });

        Ok(Self {
            addr,
            certificate,
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
    ///
    /// 客户端配置由**协议层**自己拼（它才知道该用哪个 provider），
    /// 这里只给根集合。
    pub fn root_store(&self) -> rustls::RootCertStore {
        self.certificate.root_store()
    }

    /// 只信这张假证书的客户端 TLS 配置。
    ///
    /// 给需要**绕过协议层**直接连的测试用（如验证服务器自身行为）。
    pub fn client_config(&self) -> rustls::ClientConfig {
        rustls::ClientConfig::builder()
            .with_root_certificates(self.certificate.root_store())
            .with_no_client_auth()
    }

    /// 停掉服务器。
    pub async fn shutdown(self) {
        let _ = self.shutdown.send(());
        let _ = self.handle.await;
    }
}

/// 接受连接循环。
async fn accept_loop(listener: TcpListener, acceptor: TlsAcceptor, config: Arc<ServerConfig>) {
    loop {
        let Ok((stream, _peer)) = listener.accept().await else {
            continue;
        };
        let config = config.clone();
        let acceptor = acceptor.clone();
        tokio::spawn(async move {
            let _ = handle_connection(stream, acceptor, config).await;
        });
    }
}

/// 处理一个连接。
async fn handle_connection(
    stream: TcpStream,
    acceptor: TlsAcceptor,
    config: Arc<ServerConfig>,
) -> std::io::Result<()> {
    let _ = stream.set_nodelay(true);
    let tls_stream = match acceptor.accept(stream).await {
        Ok(stream) => stream,
        // 握手失败（客户端不信任我们的自签证书）→ 直接断开。
        Err(_) => return Ok(()),
    };

    let (read_half, mut write_half) = tokio::io::split(tls_stream);
    let mut reader = BufReader::new(read_half);

    // 问候行。
    reply!(
        write_half,
        "* OK [CAPABILITY {}] Fake IMAP ready",
        config.capabilities.join(" ")
    );

    let mut state = SessionState::default();

    loop {
        let mut line = String::new();
        let read = reader.read_line(&mut line).await?;
        if read == 0 {
            break;
        }

        let Some(command) = Command::parse(&line) else {
            continue;
        };

        // 故障注入：挂起（不响应）→ 客户端应当超时。
        if let Some(Fault::HangOn(_)) = config.fault_for(&command.name) {
            tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
            break;
        }

        // 故障注入：拒绝。
        if let Some(Fault::Reject(_)) = config.fault_for(&command.name) {
            reply!(write_half, "{} NO [SERVERBUG] injected fault", command.tag);
            continue;
        }

        match handle_command(&mut write_half, &config, &mut state, &command).await {
            Ok(Continue::Continue) => {}
            Ok(Continue::Close) => break,
            Err(_) => break,
        }
    }

    Ok(())
}

/// 会话状态。
#[derive(Debug, Default)]
struct SessionState {
    /// 是否已认证。
    authenticated: bool,
    /// 是否已发过 `ID`。
    identified: bool,
}

/// 命令处理结果。
enum Continue {
    /// 继续。
    Continue,
    /// 关闭连接。
    Close,
}

/// 处理一条命令。
async fn handle_command<W>(
    mut writer: &mut W,
    config: &ServerConfig,
    state: &mut SessionState,
    command: &Command,
) -> std::io::Result<Continue>
where
    W: AsyncWrite + Unpin,
{
    // 163 行为：**认证前拒绝一切命令**（连 CAPABILITY / NOOP / LOGOUT 都不行）。
    // 我们的客户端总是先 LOGIN，所以这不会影响它；这条规则是为了
    // 保证假服务器不会比真实服务商更宽松。
    if !state.authenticated && command.name != "LOGIN" {
        reply!(writer, "{} BAD command not support", command.tag);
        return Ok(Continue::Continue);
    }

    match command.name.as_str() {
        "CAPABILITY" => {
            if config.has_fault(&Fault::NoCapabilityResponse) {
                reply!(writer, "{} BAD command not support", command.tag);
                return Ok(Continue::Continue);
            }
            reply!(writer, "* CAPABILITY {}", config.capabilities.join(" "));
            reply!(writer, "{} OK CAPABILITY completed", command.tag);
        }

        "LOGIN" => {
            let values = command.quoted_values();
            let user = values.first().cloned().unwrap_or_default();
            let password = values.get(1).cloned().unwrap_or_default();

            let rejected = config.has_fault(&Fault::RejectLogin)
                || user != config.user
                || password != config.password;

            if rejected {
                reply!(
                    writer,
                    "{} NO [AUTHENTICATIONFAILED] Invalid credentials",
                    command.tag
                );
                return Ok(Continue::Continue);
            }

            state.authenticated = true;
            reply!(writer, "{} OK LOGIN completed", command.tag);

            if config.has_fault(&Fault::DropAfterLogin) {
                return Ok(Continue::Close);
            }
        }

        "ID" => {
            if !state.authenticated {
                reply!(writer, "{} NO Not authenticated", command.tag);
                return Ok(Continue::Continue);
            }
            state.identified = true;
            // 服务器也报一下自己的身份（RFC 2971）。
            reply!(
                writer,
                "* ID (\"name\" \"FakeIMAP\" \"vendor\" \"Xremail Testkit\")"
            );
            reply!(writer, "{} OK ID completed", command.tag);
        }

        "LIST" | "LSUB" => {
            if let Some(err) = check_mailbox_access(config, state) {
                reply!(writer, "{} NO {}", command.tag, err);
                return Ok(Continue::Continue);
            }
            for mailbox in &config.mailboxes {
                reply!(
                    writer,
                    "* LIST ({}) \"/\" {}",
                    mailbox.attributes.join(" "),
                    quote(&encode_mailbox_name(&mailbox.name))
                );
            }
            reply!(writer, "{} OK LIST completed", command.tag);
        }

        "SELECT" | "EXAMINE" => {
            if command.name == "EXAMINE" && config.reject_examine {
                reply!(
                    writer,
                    "{} NO EXAMINE Unsafe Login. Please contact kefu@188.com for help",
                    command.tag
                );
                return Ok(Continue::Continue);
            }
            if let Some(err) = check_mailbox_access(config, state) {
                reply!(writer, "{} NO {}", command.tag, err);
                return Ok(Continue::Continue);
            }

            let read_only = command.name == "EXAMINE";
            let name = command.first_quoted().unwrap_or_default();
            let Some(mailbox) = find_mailbox(&config.mailboxes, &name) else {
                reply!(writer, "{} NO Mailbox does not exist", command.tag);
                return Ok(Continue::Continue);
            };

            let exists = mailbox.messages.len();
            let recent = mailbox
                .messages
                .iter()
                .filter(|m| !m.flags.iter().any(|f| f.eq_ignore_ascii_case("\\Seen")))
                .count();

            reply!(writer, "* {exists} EXISTS");
            reply!(writer, "* {recent} RECENT");
            reply!(
                writer,
                "* OK [UIDVALIDITY {}] UIDs valid",
                config.uid_validity
            );
            reply!(
                writer,
                "* FLAGS (\\Answered \\Seen \\Deleted \\Draft \\Flagged)"
            );
            reply!(
                writer,
                "* OK [PERMANENTFLAGS (\\Answered \\Seen \\Deleted \\Draft \\Flagged)] Limited"
            );

            let mode = if read_only { "READ-ONLY" } else { "READ-WRITE" };
            reply!(
                writer,
                "{} OK [{mode}] {} completed",
                command.tag,
                command.name
            );
        }

        "STATUS" => {
            if let Some(err) = check_mailbox_access(config, state) {
                reply!(writer, "{} NO {}", command.tag, err);
                return Ok(Continue::Continue);
            }
            let name = command.first_quoted().unwrap_or_default();
            let Some(mailbox) = find_mailbox(&config.mailboxes, &name) else {
                reply!(writer, "{} NO Mailbox does not exist", command.tag);
                return Ok(Continue::Continue);
            };

            let exists = mailbox.messages.len();
            let unseen = mailbox
                .messages
                .iter()
                .filter(|m| !m.flags.iter().any(|f| f.eq_ignore_ascii_case("\\Seen")))
                .count();

            reply!(
                writer,
                "* STATUS {} (MESSAGES {exists} UNSEEN {unseen} UIDVALIDITY {})",
                quote(&name),
                config.uid_validity
            );
            reply!(writer, "{} OK STATUS completed", command.tag);
        }

        "NOOP" => {
            reply!(writer, "{} OK NOOP completed", command.tag);
        }

        "LOGOUT" => {
            reply!(writer, "* BYE Fake IMAP signing off");
            reply!(writer, "{} OK LOGOUT completed", command.tag);
            return Ok(Continue::Close);
        }

        _ => {
            reply!(writer, "{} BAD command not support", command.tag);
        }
    }

    Ok(Continue::Continue)
}

/// 检查邮箱访问权限。
///
/// 复现 163 的两道人墙：未认证不能访问；要求 `ID` 时没发 `ID` 也不能访问。
fn check_mailbox_access(config: &ServerConfig, state: &SessionState) -> Option<&'static str> {
    if !state.authenticated {
        return Some("Not authenticated");
    }
    if config.require_id && !state.identified {
        return Some("Unsafe Login. Please contact kefu@188.com for help");
    }
    None
}

/// 按名字找文件夹（接受明文与 UTF-7 两种形态）。
fn find_mailbox<'a>(mailboxes: &'a [MailboxFixture], name: &str) -> Option<&'a MailboxFixture> {
    mailboxes
        .iter()
        .find(|mailbox| mailbox.name == name || encode_mailbox_name(&mailbox.name) == name)
}

/// 把文件夹名编码成 IMAP 线路形态（modified UTF-7）。
///
/// 中文名在线上是 `&XfJT0ZAB-` 这种形态。
fn encode_mailbox_name(name: &str) -> String {
    if name.is_ascii() {
        return name.to_string();
    }
    let mut out = String::new();
    out.push('&');
    let mut buffer: Vec<u8> = Vec::new();
    for unit in name.encode_utf16() {
        buffer.extend_from_slice(&unit.to_be_bytes());
    }
    out.push_str(&modified_b64_encode(&buffer));
    out.push('-');
    out
}

/// modified BASE64 编码（`/` 写作 `,`，无填充）。
fn modified_b64_encode(input: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+,";

    let mut out = String::new();
    let mut buffer: u32 = 0;
    let mut bits: u32 = 0;
    for &byte in input {
        buffer = (buffer << 8) | u32::from(byte);
        bits += 8;
        while bits >= 6 {
            bits -= 6;
            out.push(ALPHABET[((buffer >> bits) & 0x3f) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(ALPHABET[((buffer << (6 - bits)) & 0x3f) as usize] as char);
    }
    out
}

/// 写一行（自动加 CRLF）。
async fn write_line<W>(writer: &mut W, line: &str) -> std::io::Result<()>
where
    W: AsyncWrite + Unpin,
{
    writer.write_all(line.as_bytes()).await?;
    writer.write_all(b"\r\n").await?;
    writer.flush().await
}
