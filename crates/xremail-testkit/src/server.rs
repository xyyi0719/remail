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
//!
//! ## 状态
//!
//! 邮箱内容放在**共享的可变状态**里（多个连接看到同一份），
//! 这样 `STORE` 的效果能被后续的 `FETCH` / `SEARCH` 看到——
//! 真实服务器就是这样。

use std::net::SocketAddr;
use std::sync::{Arc, Mutex, MutexGuard};

use tokio::io::{AsyncBufReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio_rustls::TlsAcceptor;

use crate::certificate::{CertError, ServerCertificate};
use crate::config::{Fault, MailboxFixture, ServerConfig};
use crate::fetch::{FetchItem, StoreOperation, build_fetch_response, parse_fetch, parse_store};
use crate::parser::{Command, quote};
use crate::search;
use crate::sequence::SequenceSet;

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
        // 邮箱的**活的状态**：配置里的内容是初始值，复制一份出来给连接用。
        let mailboxes = Arc::new(Mutex::new(config.mailboxes.clone()));

        let handle = tokio::spawn(async move {
            tokio::select! {
                _ = shutdown_rx => {}
                _ = accept_loop(listener, acceptor, config, mailboxes) => {}
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
async fn accept_loop(
    listener: TcpListener,
    acceptor: TlsAcceptor,
    config: Arc<ServerConfig>,
    mailboxes: Arc<Mutex<Vec<MailboxFixture>>>,
) {
    loop {
        let Ok((stream, _peer)) = listener.accept().await else {
            continue;
        };
        let config = config.clone();
        let acceptor = acceptor.clone();
        let mailboxes = mailboxes.clone();
        tokio::spawn(async move {
            let _ = handle_connection(stream, acceptor, config, mailboxes).await;
        });
    }
}

/// 处理一个连接。
async fn handle_connection(
    stream: TcpStream,
    acceptor: TlsAcceptor,
    config: Arc<ServerConfig>,
    mailboxes: Arc<Mutex<Vec<MailboxFixture>>>,
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
        //
        // `UID FETCH` 这类命令的**命令名**是 `UID`，子命令才是 `FETCH`。
        // 两种写法都要能匹配上，否则 `Fault::Reject("FETCH")` 拦不住 `UID FETCH`。
        if let Some(Fault::Reject(_)) = config.fault_for(&command.name) {
            reply!(write_half, "{} NO [SERVERBUG] injected fault", command.tag);
            continue;
        }
        if let Some((sub, _)) = command.uid_subcommand() {
            if let Some(Fault::Reject(_)) = config.fault_for(&sub) {
                reply!(write_half, "{} NO [SERVERBUG] injected fault", command.tag);
                continue;
            }
            if let Some(Fault::HangOn(_)) = config.fault_for(&sub) {
                tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
                break;
            }
        }

        let ctx = ServerCtx {
            config: &config,
            mailboxes: &mailboxes,
        };
        match handle_command(&mut write_half, &ctx, &mut state, &command).await {
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
    /// 当前选中的文件夹（在共享状态里的下标）。
    selected: Option<usize>,
    /// 选中的是不是只读（`EXAMINE` 打开的）。
    read_only: bool,
}

/// 命令处理需要的上下文。
///
/// 单独打包是为了不把每个处理函数的参数拉成一长串。
struct ServerCtx<'a> {
    /// 配置。
    config: &'a ServerConfig,
    /// 共享的邮箱状态。
    mailboxes: &'a Mutex<Vec<MailboxFixture>>,
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
    ctx: &ServerCtx<'_>,
    state: &mut SessionState,
    command: &Command,
) -> std::io::Result<Continue>
where
    W: AsyncWrite + Unpin,
{
    let config = ctx.config;
    let mailboxes = ctx.mailboxes;

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
            let listing = {
                let guard = lock_mailboxes(mailboxes);
                guard
                    .iter()
                    .map(|mailbox| {
                        (
                            mailbox.attributes.join(" "),
                            quote(&encode_mailbox_name(&mailbox.name)),
                        )
                    })
                    .collect::<Vec<(String, String)>>()
            };
            for (attributes, name) in listing {
                reply!(writer, "* LIST ({attributes}) \"/\" {name}");
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

            let summary = {
                let guard = lock_mailboxes(mailboxes);
                let index = find_mailbox_index(&guard, &name);
                index.map(|index| {
                    let mailbox = &guard[index];
                    (
                        index,
                        mailbox.messages.len(),
                        mailbox
                            .messages
                            .iter()
                            .filter(|m| m.has_flag("\\Recent"))
                            .count(),
                        mailbox.max_uid(),
                        mailbox.messages.iter().map(|m| m.modseq).max().unwrap_or(0),
                    )
                })
            };

            let Some((index, exists, recent, _max_uid, highest_modseq)) = summary else {
                reply!(writer, "{} NO Mailbox does not exist", command.tag);
                return Ok(Continue::Continue);
            };

            state.selected = Some(index);
            state.read_only = read_only;

            reply!(writer, "* {exists} EXISTS");
            reply!(writer, "* {recent} RECENT");
            reply!(
                writer,
                "* OK [UIDVALIDITY {}] UIDs valid",
                config.uid_validity
            );
            if config.has_capability("CONDSTORE") {
                reply!(
                    writer,
                    "* OK [HIGHESTMODSEQ {highest_modseq}] Highest mod-sequence"
                );
            }
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
            let summary = {
                let guard = lock_mailboxes(mailboxes);
                find_mailbox_index(&guard, &name).map(|index| {
                    let mailbox = &guard[index];
                    let unseen = mailbox
                        .messages
                        .iter()
                        .filter(|m| !m.has_flag("\\Seen"))
                        .count();
                    (mailbox.messages.len(), unseen)
                })
            };

            let Some((exists, unseen)) = summary else {
                reply!(writer, "{} NO Mailbox does not exist", command.tag);
                return Ok(Continue::Continue);
            };

            reply!(
                writer,
                "* STATUS {} (MESSAGES {exists} UNSEEN {unseen} UIDVALIDITY {})",
                quote(&name),
                config.uid_validity
            );
            reply!(writer, "{} OK STATUS completed", command.tag);
        }

        "FETCH" => {
            handle_fetch(&mut writer, ctx, state, &command.tag, &command.args, false).await?;
        }

        "SEARCH" => {
            handle_search(&mut writer, ctx, state, &command.tag, &command.args, false).await?;
        }

        "STORE" => {
            handle_store(
                &mut writer,
                ctx,
                state,
                &command.tag,
                &command.args,
                false,
                "STORE",
            )
            .await?;
        }

        "UID" => {
            // `UID FETCH` / `UID SEARCH` / `UID STORE`。
            let Some((sub, rest)) = command.uid_subcommand() else {
                reply!(writer, "{} BAD Invalid UID command", command.tag);
                return Ok(Continue::Continue);
            };
            match sub.as_str() {
                "FETCH" => {
                    handle_fetch(&mut writer, ctx, state, &command.tag, rest, true).await?;
                }
                "SEARCH" => {
                    handle_search(&mut writer, ctx, state, &command.tag, rest, true).await?;
                }
                "STORE" => {
                    handle_store(
                        &mut writer,
                        ctx,
                        state,
                        &command.tag,
                        rest,
                        true,
                        "UID STORE",
                    )
                    .await?;
                }
                other => {
                    reply!(
                        writer,
                        "{} BAD Unsupported UID command: {}",
                        command.tag,
                        other
                    );
                }
            }
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

/// 处理 `FETCH` / `UID FETCH`。
async fn handle_fetch<W>(
    mut writer: &mut W,
    ctx: &ServerCtx<'_>,
    state: &mut SessionState,
    tag: &str,
    args: &str,
    by_uid: bool,
) -> std::io::Result<()>
where
    W: AsyncWrite + Unpin,
{
    let config = ctx.config;
    let mailboxes = ctx.mailboxes;
    // IMAP 状态机：必须先选中一个文件夹。
    let Some(index) = state.selected else {
        reply!(writer, "{tag} NO FETCH State error");
        return Ok(());
    };

    let (set_text, items) = match parse_fetch(args) {
        Ok(parsed) => parsed,
        Err(err) => {
            reply!(writer, "{tag} BAD {err}");
            return Ok(());
        }
    };
    let set = match SequenceSet::parse(&set_text) {
        Ok(set) => set,
        Err(err) => {
            reply!(writer, "{tag} BAD {err}");
            return Ok(());
        }
    };

    // 取整封正文（不带 PEEK）会**置上 `\Seen`**（RFC 3501 §6.4.8）。
    let sets_seen = items.iter().any(|item| match item {
        FetchItem::Body { peek, .. } => !peek,
        FetchItem::Rfc822 { part } => part.is_none(),
        _ => false,
    });

    // 锁内只算数据，锁外才写响应（锁不能跨 await）。
    let prepared = {
        let mut guard = lock_mailboxes(mailboxes);
        match guard.get_mut(index) {
            None => None,
            Some(mailbox) => {
                let targets = resolve_targets(&set, &mailbox.messages, by_uid);

                if sets_seen {
                    for &target in &targets {
                        let message = &mut mailbox.messages[target];
                        if !message.has_flag("\\Seen") {
                            message.flags.push("\\Seen".to_string());
                            message.modseq += 1;
                        }
                    }
                }

                let with_modseq = config.has_capability("CONDSTORE");
                let responses: Vec<crate::fetch::FetchResponse> = targets
                    .iter()
                    .map(|&target| {
                        let message = &mailbox.messages[target];
                        build_fetch_response(target as u32 + 1, message, &items, with_modseq)
                    })
                    .collect();
                Some(responses)
            }
        }
    };

    let Some(responses) = prepared else {
        reply!(writer, "{tag} NO Mailbox does not exist");
        return Ok(());
    };

    for response in responses {
        write_fetch_response(&mut writer, &response).await?;
    }
    reply!(writer, "{tag} OK FETCH completed");
    Ok(())
}

/// 写一条 FETCH 响应。
///
/// 有字面量时格式是 `* 1 FETCH (UID 1 BODY[HEADER] {23}\r\n<23 字节>)`。
async fn write_fetch_response<W>(
    writer: &mut W,
    response: &crate::fetch::FetchResponse,
) -> std::io::Result<()>
where
    W: AsyncWrite + Unpin,
{
    match &response.literal {
        Some((section, bytes)) => {
            // 字面量的长度写在行尾，**真正的字节在下一行之后**，
            // 所以这一行不能带 `)`——`)` 要等字节发完再补。
            let prefix = if response.simple.is_empty() {
                format!("* {} FETCH ({} {{{}}}", response.seq, section, bytes.len())
            } else {
                format!(
                    "* {} FETCH ({} {} {{{}}}",
                    response.seq,
                    response.simple,
                    section,
                    bytes.len()
                )
            };
            write_line(writer, &prefix).await?;
            writer.write_all(bytes).await?;
            writer.write_all(b")\r\n").await?;
            writer.flush().await
        }
        None => {
            write_line(
                writer,
                &format!("* {} FETCH ({})", response.seq, response.simple),
            )
            .await
        }
    }
}

/// 处理 `SEARCH` / `UID SEARCH`。
async fn handle_search<W>(
    mut writer: &mut W,
    ctx: &ServerCtx<'_>,
    state: &mut SessionState,
    tag: &str,
    args: &str,
    by_uid: bool,
) -> std::io::Result<()>
where
    W: AsyncWrite + Unpin,
{
    let mailboxes = ctx.mailboxes;
    let Some(index) = state.selected else {
        reply!(writer, "{tag} NO SEARCH State error");
        return Ok(());
    };

    let key = match search::parse(args) {
        Ok(key) => key,
        Err(err) => {
            reply!(writer, "{tag} BAD {err}");
            return Ok(());
        }
    };

    let results = {
        let guard = lock_mailboxes(mailboxes);
        match guard.get(index) {
            None => None,
            Some(mailbox) => {
                let max_uid = mailbox.max_uid();
                let hits: Vec<usize> = mailbox
                    .messages
                    .iter()
                    .enumerate()
                    .filter(|(_, message)| search::matches(&key, message, max_uid))
                    .map(|(index, _)| index)
                    .collect();

                Some(if by_uid {
                    hits.iter()
                        .map(|&index| mailbox.messages[index].uid)
                        .collect::<Vec<u32>>()
                } else {
                    hits.iter()
                        .map(|&index| index as u32 + 1)
                        .collect::<Vec<u32>>()
                })
            }
        }
    };

    let Some(results) = results else {
        reply!(writer, "{tag} NO Mailbox does not exist");
        return Ok(());
    };

    let joined = results
        .iter()
        .map(u32::to_string)
        .collect::<Vec<String>>()
        .join(" ");
    reply!(writer, "* SEARCH {joined}");
    reply!(writer, "{tag} OK SEARCH completed");
    Ok(())
}

/// `STORE` 的结果。
enum StoreOutcome {
    /// 出错（`NO`）。
    Failed(String),
    /// 成功，被改动的邮件 `(序号, UID, 标记)`。
    Changed(Vec<(u32, u32, Vec<String>)>),
}

/// 处理 `STORE` / `UID STORE`。
async fn handle_store<W>(
    mut writer: &mut W,
    ctx: &ServerCtx<'_>,
    state: &mut SessionState,
    tag: &str,
    args: &str,
    by_uid: bool,
    label: &str,
) -> std::io::Result<()>
where
    W: AsyncWrite + Unpin,
{
    let mailboxes = ctx.mailboxes;
    let Some(index) = state.selected else {
        reply!(writer, "{tag} NO STORE State error");
        return Ok(());
    };

    // `EXAMINE` 打开的文件夹是只读的，不能改标记。
    if state.read_only {
        reply!(writer, "{tag} NO [READ-ONLY] Mailbox is read-only");
        return Ok(());
    }

    let (set_text, rest) = match split_first_token(args) {
        Some(parts) => parts,
        None => {
            reply!(writer, "{tag} BAD Missing sequence set");
            return Ok(());
        }
    };
    let set = match SequenceSet::parse(set_text) {
        Ok(set) => set,
        Err(err) => {
            reply!(writer, "{tag} BAD {err}");
            return Ok(());
        }
    };
    let request = match parse_store(rest) {
        Ok(request) => request,
        Err(err) => {
            reply!(writer, "{tag} BAD {err}");
            return Ok(());
        }
    };

    // 条件写（RFC 3501 §6.4.6）：**全有或全无**——
    // 只要有一封的 modseq 对不上，整条命令都不改任何东西。
    let outcome = {
        let mut guard = lock_mailboxes(mailboxes);
        match guard.get_mut(index) {
            None => None,
            Some(mailbox) => {
                let targets = resolve_targets(&set, &mailbox.messages, by_uid);

                let conflict = request.unchangedsince.is_some_and(|expected| {
                    targets
                        .iter()
                        .any(|&target| mailbox.messages[target].modseq != expected)
                });

                if conflict {
                    Some(StoreOutcome::Failed(format!(
                        "[MODIFIED] {label} failed: mod-sequence changed"
                    )))
                } else {
                    let mut changes = Vec::new();
                    for &target in &targets {
                        let message = &mut mailbox.messages[target];
                        let before = message.flags.clone();

                        match request.operation {
                            StoreOperation::Add => {
                                for flag in &request.flags {
                                    if !message.has_flag(flag) {
                                        message.flags.push(flag.clone());
                                    }
                                }
                            }
                            StoreOperation::Remove => {
                                message.flags.retain(|flag| {
                                    !request
                                        .flags
                                        .iter()
                                        .any(|remove| remove.eq_ignore_ascii_case(flag))
                                });
                            }
                            StoreOperation::Set => {
                                message.flags = request.flags.clone();
                            }
                        }

                        if message.flags != before {
                            message.modseq += 1;
                            changes.push((target as u32 + 1, message.uid, message.flags.clone()));
                        }
                    }
                    Some(StoreOutcome::Changed(changes))
                }
            }
        }
    };

    match outcome {
        None => {
            reply!(writer, "{tag} NO Mailbox does not exist");
        }
        Some(StoreOutcome::Failed(message)) => {
            reply!(writer, "{tag} NO {message}");
        }
        Some(StoreOutcome::Changed(changes)) => {
            // `.SILENT` 变体不回 untagged FETCH 响应。
            if !request.silent {
                for (seq, uid, flags) in &changes {
                    reply!(
                        writer,
                        "* {seq} FETCH (UID {uid} FLAGS ({}))",
                        flags.join(" ")
                    );
                }
            }
            reply!(writer, "{tag} OK {label} completed");
        }
    }

    Ok(())
}

/// 算出 sequence set 命中哪些邮件（返回下标）。
///
/// `by_uid` 为真时 sequence set 是 **UID**；否则是**序号**（第几封）。
///
/// **不展开 set**——而是拿每封邮件去问 set。
/// 因为 `1:*` 在大 UID 空间下可能覆盖上亿个 UID（163 就是），
/// 展开成 `Vec` 会直接吃掉几个 GB。
fn resolve_targets(
    set: &SequenceSet,
    messages: &[crate::config::MessageFixture],
    by_uid: bool,
) -> Vec<usize> {
    if by_uid {
        let max_uid = messages.iter().map(|m| m.uid).max().unwrap_or(0);
        messages
            .iter()
            .enumerate()
            .filter(|(_, message)| set.contains(message.uid, max_uid))
            .map(|(index, _)| index)
            .collect()
    } else {
        let count = messages.len() as u32;
        messages
            .iter()
            .enumerate()
            .filter(|(index, _)| set.contains((*index as u32) + 1, count))
            .map(|(index, _)| index)
            .collect()
    }
}

/// 切出第一个词（其余原样返回）。
fn split_first_token(input: &str) -> Option<(&str, &str)> {
    let input = input.trim_start();
    match input.find(char::is_whitespace) {
        Some(index) => Some((&input[..index], input[index..].trim_start())),
        None => {
            if input.is_empty() {
                None
            } else {
                Some((input, ""))
            }
        }
    }
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

/// 锁住邮箱状态。
///
/// 用 `std::sync::Mutex`：注意**绝不能**把 guard 跨 `.await` 持有
/// （那会让 future 变成非 `Send`，`tokio::spawn` 就用不了了）。
fn lock_mailboxes(mailboxes: &Mutex<Vec<MailboxFixture>>) -> MutexGuard<'_, Vec<MailboxFixture>> {
    mailboxes
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// 按名字找文件夹下标（接受明文与 UTF-7 两种形态）。
fn find_mailbox_index(mailboxes: &[MailboxFixture], name: &str) -> Option<usize> {
    mailboxes
        .iter()
        .position(|mailbox| mailbox.name == name || encode_mailbox_name(&mailbox.name) == name)
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
