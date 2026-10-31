//! SMTP **提交**（真发信）协议层。
//!
//! 与 [`crate::probe`] 的分工：
//!
//! * 探测（[`crate::probe`]）：**只读**，回答「这个服务器支持什么」。
//! * 提交（本模块）：**真的发信**，把一封信交给服务器。
//!
//! ## 为什么把「结果」分成四态
//!
//! 提交的结果**不是**简单的「成功 / 失败」。最要命的是第三种：
//!
//! | 状态 | 含义 | 能不能重发 |
//! |---|---|---|
//! | [`SubmitOutcome::Accepted`] | 服务器明确回了 250 —— **这封信已经发出去了** | **不能**（会变成两封） |
//! | [`SubmitOutcome::Rejected`] | 服务器明确拒收（4xx / 5xx）—— **没有发出去** | 4xx 可以，5xx 不该 |
//! | [`SubmitOutcome::NotSent`] | `DATA` 之前连接就断了 —— 服务器手里没有这封信的任何一个字节 | **可以**（收不到两封） |
//! | [`SubmitOutcome::Uncertain`] | `DATA` 写完了但**没拿到最终应答** | **不能盲重发**（可能已经发过了） |
//!
//! 「不确定」是真实存在的：客户端把 `DATA` 写完了，但最终应答在网络上丢了
//! （或者服务器收下之后连接被重置）。这时候服务器可能已经把这封信投递出去了。
//! 盲重发 = 用户收到两封信。
//!
//! 而 `NotSent` 与 `Uncertain` 的分界就是**正文有没有传出去**：
//! SMTP 只有 `DATA` 阶段才传正文，在那之前断了连接，服务器压根没收到过内容。
//!
//! ## 流程
//!
//! 连接 → `EHLO` → （必要时 `STARTTLS` → 重新 `EHLO`）→ `AUTH`
//! → `MAIL FROM` → `RCPT TO`（每个信封收件人一条）→ `DATA` → 读最终应答 → `QUIT`。
//!
//! ## 能力协商：服务器**宣告**了什么才算什么
//!
//! `MAIL FROM` 的 ESMTP 参数**只按服务器在 `EHLO` 里宣告的**来带：
//!
//! | 信封地址 | 8 位正文 | 服务器宣告 | `MAIL FROM` 参数 |
//! |---|---|---|---|
//! | 全 ASCII | 否 | 任意 | 无 |
//! | 全 ASCII | 是 | `8BITMIME` | `BODY=8BITMIME` |
//! | 全 ASCII | 是 | **无** `8BITMIME` | **报错**（不发） |
//! | 有非 ASCII | 任意 | `SMTPUTF8` | `SMTPUTF8`（还要 8 位正文时再加 `BODY=8BITMIME`） |
//! | 有非 ASCII | 任意 | **无** `SMTPUTF8` | **报错**（不发） |
//!
//! 「报错」= `Err(SmtpError::Unsupported)`，而且**发生在 `MAIL FROM` 之前**——
//! 一个字节都没发出去。半途失败是最糟的：事务开了头，服务器那边悬着，
//! 用户既没收到邮件，也没收到一个干净的结果。
//!
//! ## RFC 依据
//!
//! * **RFC 6152 §3** —— 服务器没宣告 `8BITMIME` 时，客户端「**不得以任何理由**」
//!   发送含 US-ASCII 范围（hex 00-7F）之外字节的内容。
//! * **RFC 6531 §3.2 / §3.4** —— 用了 UTF-8 地址就**必须**在 `MAIL FROM` 上带
//!   `SMTPUTF8` 参数；反过来，服务器没宣告 `SMTPUTF8` 时客户端**不得**发国际化地址。
//!   RFC 6531 §3.4 还说：信封与正文都**不需要**这个能力时，
//!   客户端**不应该**（SHOULD NOT）带这个参数——所以 ASCII 邮件不带它。
//!
//! ## 实测的各家服务器（2026-10，见 `docs/PROTOCOLS.md`）
//!
//! | 服务器 | `8BITMIME` | `SMTPUTF8` | 后果 |
//! |---|---|---|---|
//! | `smtp.163.com:465` | ✅ | ❌ | 只能发 ASCII 地址；非 ASCII 地址**直接报错** |
//! | `smtp.qq.com:465` | ✅ | ✅ | 非 ASCII 地址可以发（自动带 `SMTPUTF8`） |
//!
//! 也就是说：非 ASCII 邮箱地址在 163 上**发不出去**，这不是 bug，是服务器没这个能力；
//! 客户端唯一正确的做法就是**提前说清楚**，而不是发一个必然被拒的事务。

use crate::error::SmtpError;
use crate::session::{
    AuthMechanism, MailFromParams, SmtpCapabilities, SmtpSession, address_requires_smtputf8,
};
use crate::transport::{SmtpConfig, TlsMode, TlsTrust};

/// `EHLO` 用的域名。
///
/// RFC 5321 §4.1.1.1 要求客户端在 `EHLO` 里报出自己的域名。
/// 这里用 `localhost`：提交场景下我们**不**想把用户的主机名泄给服务器，
/// 而真实服务器也不关心这个值（它只用来看你是不是客户端）。
const EHLO_DOMAIN: &str = "localhost";

/// SMTP 信封：信封收件人，**与邮件头里的 To/Cc/Bcc 是两回事**。
///
/// 为什么必须分开：`Bcc` 的收件人**不能**出现在邮件头里（否则收件人互相看得见），
/// 但它**必须**出现在信封里（不然服务器不会投给它）。
/// `Envelope` 就是「信封」，与 MIME 层的 `To` / `Cc` / `Bcc` 头互不干涉。
#[derive(Debug, Clone, Default)]
pub struct Envelope {
    /// 发件人地址（`addr@x`，不带显示名）。
    pub sender: String,
    /// 全部收件人（to + cc + bcc 合并后的信封收件人）。
    pub recipients: Vec<String>,
}

/// 一次提交请求。
#[derive(Debug, Clone)]
pub struct SubmitRequest {
    /// 信封（发件人与收件人）。
    pub envelope: Envelope,
    /// 完整邮件字节（由 MIME 构建层产出）。
    pub message: Vec<u8>,
    /// 正文是否含 8 位内容。
    ///
    /// `true` → `MAIL FROM` 带 `BODY=8BITMIME`，但**前提**是服务器宣告了
    /// `8BITMIME`；服务器没宣告就**直接报错**（RFC 6152 §3：
    /// 客户端不能在没有 `8BITMIME` 的服务器上发 8 位正文）。
    ///
    /// `false` → 这封信是纯 7 位，不需要任何 ESMTP 参数。
    pub eight_bit: bool,
}

/// 提交走到哪一步。
///
/// 有了它，失败时才能告诉用户「卡在哪一步」——
/// 「被拒收」和「没发出去」在用户眼里是两件事。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubmitStage {
    /// 连接与问候（`220`）。
    Connect,
    /// 认证（`AUTH`）。
    Auth,
    /// `MAIL FROM`。
    MailFrom,
    /// `RCPT TO`。
    RcptTo,
    /// `DATA`。
    Data,
    /// `QUIT`。
    Quit,
}

/// 提交结果。
#[derive(Debug, Clone)]
pub enum SubmitOutcome {
    /// 服务器明确接受（250）。**这封信已经发出去了。**
    Accepted {
        /// 服务器给的响应码（一般是 `250`）。
        code: u16,
        /// 服务器的响应正文（原文）。
        message: String,
    },
    /// 服务器明确拒收（4xx / 5xx），**没有**发出去，可以安全重试或放弃。
    Rejected {
        /// 服务器给的响应码。
        code: u16,
        /// 服务器的响应正文（原文，含增强状态码）。
        message: String,
        /// 被拒在哪一步。
        stage: SubmitStage,
    },
    /// 传输层在 `DATA` **之前**就断了 —— 这封信**肯定没发出去**。
    ///
    /// ## 为什么要和 [`SubmitOutcome::Uncertain`] 分开
    ///
    /// SMTP 只有在 `DATA` 阶段才会把**正文**传出去。
    /// 在那之前连接断了（`AUTH` / `MAIL FROM` / `RCPT TO`），
    /// 服务器手里**没有这封信的任何一个字节**——
    /// 它最多开了一个空事务，连接一断就作废。
    ///
    /// 两种情况的后果完全不同：
    /// * [`SubmitOutcome::NotSent`] → 重发**安全**（收件人不会收到两封）；
    /// * [`SubmitOutcome::Uncertain`] → 重发**可能**变成两封。
    ///
    /// 混为一谈的代价是：明明没发出去，却告诉用户「别重复发送」，
    /// 用户于是把这封信永远留在草稿箱里。
    NotSent {
        /// 为什么断了（传输层错误原文）。
        reason: String,
        /// 卡在哪一步。
        stage: SubmitStage,
    },
    /// 命令发出去了但**没拿到最终应答** —— 不知道服务器到底收没收收。
    /// **不能盲重发**（可能已经发过了）。
    Uncertain {
        /// 为什么没拿到应答（传输层错误原文）。
        reason: String,
        /// 卡在哪一步。
        stage: SubmitStage,
    },
}

impl SubmitOutcome {
    /// 4xx（临时故障）可以重试；5xx（永久拒收）不该重试。
    ///
    /// 另外三种状态里：
    ///
    /// * [`SubmitOutcome::Accepted`]：已经发出去了，重发 = 收件人收到两封。**不可重试**。
    /// * [`SubmitOutcome::Uncertain`]：可能已经发出去了，重发同样可能变成两封。
    ///   **不确定 ≠ 可重试**。
    /// * [`SubmitOutcome::NotSent`]：`DATA` 之前就断了，服务器手里没有这封信，
    ///   重发**安全**。
    pub fn is_retryable(&self) -> bool {
        match self {
            SubmitOutcome::Rejected { code, .. } => (400..500).contains(code),
            SubmitOutcome::NotSent { .. } => true,
            SubmitOutcome::Accepted { .. } | SubmitOutcome::Uncertain { .. } => false,
        }
    }
}

/// 按 SMTP 提交一封邮件。
///
/// 流程：连接 → EHLO → （必要时 STARTTLS）→ AUTH → MAIL FROM → RCPT TO（每个收件人一次）
/// → DATA → 读最终应答 → QUIT。
///
/// # 返回值
///
/// * `Ok(SubmitOutcome::Accepted)`：服务器明确收下了（250）——**信已经发出去了**。
/// * `Ok(SubmitOutcome::Rejected)`：服务器明确拒收——**信没有发出去**。
/// * `Ok(SubmitOutcome::Uncertain)`：命令发出去了但没拿到最终应答——
///   **不知道有没有发出去**，不能盲重发。
/// * `Err(SmtpError)`：连都没连上 / TLS 握手失败 / 入参不合法这类**更早**的失败。
///
/// # 不会 panic
///
/// 所有失败都以返回值表达：连接断了、应答不合法、超时——都不会 panic。
pub async fn submit_message(
    config: &SmtpConfig,
    trust: &TlsTrust,
    user: &str,
    password: &str,
    request: &SubmitRequest,
) -> Result<SubmitOutcome, SmtpError> {
    validate(request)?;

    // 1) 连接。
    let mut session = match config.tls_mode {
        TlsMode::Direct => SmtpSession::connect_tls(config, trust).await?,
        // 明文连接只用于 STARTTLS 升级路径——
        // 后面必定会升级，升级不了就报错（不降级）。
        TlsMode::Starttls => SmtpSession::connect_plaintext(config).await?,
    };

    // 2) 跑完整个提交流程。
    let outcome = run_submission(
        &mut session,
        trust,
        user,
        password,
        request,
        config.tls_mode,
        &config.host,
    )
    .await;

    // 3) 尽量 `QUIT`：无论成功失败都礼貌收尾。
    //    连接已经断了（不确定态）时 `QUIT` 会失败——那没关系，忽略它。
    //    **不能**因为收尾失败就把已经拿到的结果弄丢。
    let _ = session.quit().await;

    outcome
}

/// 跑完整个提交流程（不含 `QUIT` 收尾）。
async fn run_submission(
    session: &mut SmtpSession,
    trust: &TlsTrust,
    user: &str,
    password: &str,
    request: &SubmitRequest,
    tls_mode: TlsMode,
    host: &str,
) -> Result<SubmitOutcome, SmtpError> {
    // 1) 问候。服务器在问候里就拒收（`554` 之类）——连门都没进。
    let greeting = session.greeting().await?;
    if !greeting.is_success() {
        return Ok(SubmitOutcome::Rejected {
            code: greeting.code,
            message: greeting.text,
            stage: SubmitStage::Connect,
        });
    }

    // 2) EHLO（必要时 STARTTLS 后再 EHLO）。
    let capabilities = negotiate(session, trust, tls_mode).await?;

    // 3) **能力检查**：在发出任何投递命令之前就把「发不了」说清楚。
    //
    // 顺序很要紧——这一步在 `AUTH` **之前**：
    // 已经知道这封信发不出去，就没必要把口令送出去。
    let mail_from_params = match decide_mail_from_params(request, &capabilities, host) {
        Ok(params) => params,
        Err(err) => return Err(err),
    };

    // 4) 认证。
    if let Some(outcome) = authenticate(session, &capabilities, user, password).await? {
        return Ok(outcome);
    }

    // 5) `MAIL FROM`。
    //
    // 参数是**算出来的**，不是猜的：`mail_from_params` 已经按服务器宣告的
    // 能力把该带的参数带齐、把不该带的排除了；
    // 服务器缺能力时它已经报错了（一个字节都没发出去）。
    let mail_from = match session
        .mail_from_with_params(&request.envelope.sender, mail_from_params)
        .await
    {
        Ok(response) => response,
        // 命令发出去了却没拿到应答：正文**还没开始传**
        //（`DATA` 之后才传），所以这封信肯定没发出去。
        Err(err) => {
            return Ok(SubmitOutcome::NotSent {
                reason: err.to_string(),
                stage: SubmitStage::MailFrom,
            });
        }
    };
    if !mail_from.is_success() {
        return Ok(SubmitOutcome::Rejected {
            code: mail_from.code,
            message: mail_from.text,
            stage: SubmitStage::MailFrom,
        });
    }

    // 6) `RCPT TO`：每个信封收件人一条。
    //
    // **任何一个被拒就停在这里**：不再往下走，更不发 `DATA`。
    // 发了 `DATA` 就等于「这封信要投出去了」——明知道有收件人被拒还发，
    // 是把用户的信投给不该收的人。
    for recipient in &request.envelope.recipients {
        let response = match session.rcpt_to(recipient).await {
            Ok(response) => response,
            Err(err) => {
                return Ok(SubmitOutcome::NotSent {
                    reason: err.to_string(),
                    stage: SubmitStage::RcptTo,
                });
            }
        };
        if !response.is_success() {
            return Ok(SubmitOutcome::Rejected {
                code: response.code,
                message: response.text,
                stage: SubmitStage::RcptTo,
            });
        }
    }

    // 7) `DATA`：写正文，读最终应答。
    let data = match session.data(&request.message).await {
        Ok(response) => response,
        // 正文已经写出去了，却没拿到最终应答 → **不确定**。
        // 这封信可能已经被服务器收下（甚至已经投递），
        // 所以**不能**把它当成失败去重发。
        Err(err) => {
            return Ok(SubmitOutcome::Uncertain {
                reason: err.to_string(),
                stage: SubmitStage::Data,
            });
        }
    };
    if !data.is_success() {
        return Ok(SubmitOutcome::Rejected {
            code: data.code,
            message: data.text,
            stage: SubmitStage::Data,
        });
    }

    // 服务器明确回了 2xx：**这封信已经发出去了**。
    Ok(SubmitOutcome::Accepted {
        code: data.code,
        message: data.text,
    })
}

/// `EHLO` + （必要时）`STARTTLS` + 再 `EHLO`。
///
/// 返回 **TLS 阶段**的能力集（权威版本）：
/// RFC 3207 §4.2 要求握手之后**重新** `EHLO`——
/// 很多服务器只在 TLS 之后才宣告 `AUTH`，
/// 也可能在 TLS 之后才（或不再）宣告 `8BITMIME` / `SMTPUTF8`。
async fn negotiate(
    session: &mut SmtpSession,
    trust: &TlsTrust,
    tls_mode: TlsMode,
) -> Result<SmtpCapabilities, SmtpError> {
    match tls_mode {
        TlsMode::Direct => {
            let response = session.ehlo(EHLO_DOMAIN).await?;
            if !response.is_success() {
                return Err(SmtpError::Smtp(format!("EHLO 被拒绝: {}", response.text)));
            }
            Ok(session.capabilities().clone())
        }
        TlsMode::Starttls => {
            // 明文阶段先 `EHLO`：服务器告不宣告 `STARTTLS`。
            let first = session.ehlo(EHLO_DOMAIN).await?;
            if !first.is_success() {
                return Err(SmtpError::Smtp(format!("EHLO 被拒绝: {}", first.text)));
            }
            if !session.capabilities().has_keyword("STARTTLS") {
                // **不降级**：没有 TLS 就不提交（更不认证）。
                // 宁可报错，也不能在明文上把口令送出去。
                return Err(SmtpError::Unsupported(
                    "服务器不宣告 STARTTLS，拒绝在明文上提交".into(),
                ));
            }

            session.starttls(trust).await?;

            // 握手后**必须**重新 `EHLO`。
            let second = session.ehlo(EHLO_DOMAIN).await?;
            if !second.is_success() {
                return Err(SmtpError::Smtp(format!(
                    "TLS 升级后的 EHLO 被拒绝: {}",
                    second.text
                )));
            }
            Ok(session.capabilities().clone())
        }
    }
}

/// 按服务器宣告的能力，决定 `MAIL FROM` 该带哪些参数。
///
/// 服务器缺能力时**在这里就报错**——在发 `MAIL FROM` 之前。
///
/// ## 为什么必须在发命令之前报错
///
/// 服务器拒绝一个不合规矩的 `MAIL FROM`，拒的是**整个事务**：
/// 已经开的事务作废、已经发的命令白费，用户还得重新来一遍。
/// 与其发出去碰壁，不如一开始就讲清楚「这台服务器发不了这封信」。
///
/// ## 为什么地址与正文分开判
///
/// 两个扩展管的是两件事，缺哪个就报哪个：
///
/// * `SMTPUTF8`（RFC 6531 §3.2 / §3.4）管**信封地址**里的非 ASCII；
/// * `8BITMIME`（RFC 6152 §3）管**正文**里的 8 位内容。
///
/// 报错要说清楚是**哪个地址**、缺**哪个能力**——用户能照着改。
fn decide_mail_from_params(
    request: &SubmitRequest,
    capabilities: &SmtpCapabilities,
    host: &str,
) -> Result<MailFromParams, SmtpError> {
    // 1) 非 ASCII 地址 → 需要 `SMTPUTF8`。
    let utf8_addresses: Vec<&str> = std::iter::once(request.envelope.sender.as_str())
        .chain(request.envelope.recipients.iter().map(String::as_str))
        .filter(|address| address_requires_smtputf8(address))
        .collect();

    if !utf8_addresses.is_empty() && !capabilities.supports_smtputf8() {
        // 把**所有**出问题的地址都列出来：用户可能一次改掉，
        // 只报第一个会让人来回试。
        let listed = utf8_addresses.join("」「");
        return Err(SmtpError::Unsupported(format!(
            "地址「{listed}」含非 ASCII 字符，需要 SMTPUTF8 扩展（RFC 6531），但服务器 {host} 没有宣告 SMTPUTF8：这封信发不出去，尚未发出任何投递命令；请改用纯 ASCII 地址，或换一个宣告了 SMTPUTF8 的服务器"
        )));
    }

    // 2) 8 位正文 → 需要 `8BITMIME`。
    //
    // RFC 6152 §3 给了客户端两条路：把正文**转成 7 位**（网关转换），
    // 或者把 8 位当成**永久性失败**（按投递失败处理）。
    // 这里选后者：正文用什么编码（quoted-printable / base64）是**MIME 构建层**的
    // 决定；在这一层偷偷重编码，会让线上的字节和正文里声明的 `Content-Transfer-Encoding`
    // 对不上——收件人看到的是坏信，而不是一句清楚的错误。
    if request.eight_bit && !capabilities.supports_8bitmime() {
        return Err(SmtpError::Unsupported(format!(
            "这封信的正文含 8 位内容，需要 8BITMIME 扩展（RFC 6152），但服务器 {host} 没有宣告 8BITMIME：这封信发不出去，尚未发出任何投递命令；请把正文转成 7 位（quoted-printable / base64），或换一个宣告了 8BITMIME 的服务器"
        )));
    }

    Ok(MailFromParams {
        body_8bitmime: request.eight_bit && capabilities.supports_8bitmime(),
        smtputf8: !utf8_addresses.is_empty() && capabilities.supports_smtputf8(),
    })
}

/// 认证。
///
/// 返回 `Some(outcome)` = 认证没过，提交就此结束；
/// `None` = 认证通过（或服务器压根不要求认证），可以继续往下走。
async fn authenticate(
    session: &mut SmtpSession,
    capabilities: &SmtpCapabilities,
    user: &str,
    password: &str,
) -> Result<Option<SubmitOutcome>, SmtpError> {
    let mechanisms = &capabilities.auth_mechanisms;
    if mechanisms.is_empty() {
        // 服务器没宣告 `AUTH`：没有认证机制可用。
        // 有些服务器（比如本地中继）确实不要认证——那就直接发。
        tracing::warn!("服务器没有宣告 AUTH 机制，跳过认证直接提交");
        return Ok(None);
    }

    let Some(mechanism) = select_mechanism(mechanisms) else {
        return Ok(Some(SubmitOutcome::Rejected {
            code: 504,
            message: format!("服务器宣告的认证机制本客户端都不支持: {mechanisms:?}"),
            stage: SubmitStage::Auth,
        }));
    };

    let response = match mechanism {
        AuthMechanism::Plain => session.auth_plain(user, password).await,
        AuthMechanism::Login => session.auth_login(user, password).await,
        // XOAUTH2 需要 OAuth 令牌，不是账号口令——本批不实现。
        AuthMechanism::XOAuth2 => {
            return Err(SmtpError::Unsupported(
                "XOAUTH2 尚未实现（它需要 OAuth 令牌，不是账号口令）".into(),
            ));
        }
    };

    let response = match response {
        Ok(response) => response,
        // 命令发出去了却没拿到应答：`AUTH` 阶段还没开始传正文，
        // 所以这封信**肯定没发出去**（`DATA` 之前断了连接）。
        Err(err) => {
            return Ok(Some(SubmitOutcome::NotSent {
                reason: err.to_string(),
                stage: SubmitStage::Auth,
            }));
        }
    };

    if !response.is_success() {
        return Ok(Some(SubmitOutcome::Rejected {
            code: response.code,
            message: response.text,
            stage: SubmitStage::Auth,
        }));
    }

    Ok(None)
}

/// 选一个可用的认证机制。
///
/// 默认顺序 `LOGIN` → `PLAIN`：`LOGIN` 更老、几乎所有服务器都实现；
/// `PLAIN` 在部分服务器上要求先完成 TLS 升级才开放。
///
/// `XOAUTH2` **不选**：它需要 OAuth 令牌，不是账号口令。
fn select_mechanism(mechanisms: &[String]) -> Option<AuthMechanism> {
    let available: Vec<AuthMechanism> = mechanisms
        .iter()
        .filter_map(|name| AuthMechanism::parse(name))
        .collect();

    [AuthMechanism::Login, AuthMechanism::Plain]
        .into_iter()
        .find(|candidate| available.contains(candidate))
}

/// 入参检查。
fn validate(request: &SubmitRequest) -> Result<(), SmtpError> {
    if request.envelope.sender.trim().is_empty() {
        return Err(SmtpError::Invalid("发件人地址为空".into()));
    }
    if request.envelope.recipients.is_empty() {
        return Err(SmtpError::Invalid("没有收件人".into()));
    }
    if request
        .envelope
        .recipients
        .iter()
        .any(|recipient| recipient.trim().is_empty())
    {
        return Err(SmtpError::Invalid("收件人地址为空".into()));
    }
    Ok(())
}
