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
//! ## SMTPUTF8（RFC 6531）：本批**不做**
//!
//! 探测显示：**QQ 支持 `SMTPUTF8`，163 不支持**。
//!
//! 一旦要用（非 ASCII 的邮箱地址，或者标题里直接放 UTF-8 而不是 RFC 2047 编码），
//! **必须先确认服务器宣告了 `SMTPUTF8`**——没宣告就发 `SMTPUTF8` 参数，
//! 服务器会直接拒收整个事务。退路是**退化成 RFC 2047 编码**
//! （`=?UTF-8?B?...?=` / `=?UTF-8?Q?...?=`），也就是把非 ASCII 标题全部编码掉。
//!
//! 位置已经留好：[`crate::probe::ExtensionSupport::smtputf8`] 就是那个开关，
//! 但**本模块不会**主动使用它——要开这条路，先把地址与标题的编码策略定下来。

use crate::error::SmtpError;
use crate::probe::{Extension, auth_mechanisms, extension_support, parse_extensions};
use crate::session::{AuthMechanism, SmtpSession};
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
    /// 服务器支持 `8BITMIME` 时为 true → `MAIL FROM` 带 `BODY=8BITMIME`。
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
    let extensions = negotiate(session, trust, tls_mode).await?;
    let support = extension_support(&extensions);

    // 3) 认证。
    if let Some(outcome) = authenticate(session, &extensions, user, password).await? {
        return Ok(outcome);
    }

    // 4) `MAIL FROM`。
    //
    // `BODY=8BITMIME` 只在**服务器宣告了 `8BITMIME`**时才带：
    // 服务器没宣告还带这个参数，它会拒收整个事务。
    // 至于「这封信本身是不是 8 位内容」，由调用方通过 `SubmitRequest::eight_bit` 决定。
    //
    // ## SMTPUTF8 的位置（RFC 6531）
    //
    // 这里**没有** `SMTPUTF8` 参数，这是有意的：
    // 探测显示 QQ 支持、163 不支持，而服务器不支持时带上它会被拒收。
    // 真要用，必须先确认 `support.smtputf8`，否则退化成 RFC 2047 编码。
    let body_8bitmime = request.eight_bit && support.eight_bit_mime;
    let mail_from = match session
        .mail_from(&request.envelope.sender, body_8bitmime)
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

    // 5) `RCPT TO`：每个信封收件人一条。
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

    // 6) `DATA`：写正文，读最终应答。
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
/// 返回 **TLS 阶段**的扩展清单（权威版本）：
/// RFC 3207 §4.2 要求握手之后**重新** `EHLO`——
/// 很多服务器只在 TLS 之后才宣告 `AUTH`。
async fn negotiate(
    session: &mut SmtpSession,
    trust: &TlsTrust,
    tls_mode: TlsMode,
) -> Result<Vec<Extension>, SmtpError> {
    match tls_mode {
        TlsMode::Direct => {
            let response = session.ehlo(EHLO_DOMAIN).await?;
            if !response.is_success() {
                return Err(SmtpError::Smtp(format!("EHLO 被拒绝: {}", response.text)));
            }
            Ok(parse_extensions(&response))
        }
        TlsMode::Starttls => {
            // 明文阶段先 `EHLO`：服务器告不宣告 `STARTTLS`。
            let first = session.ehlo(EHLO_DOMAIN).await?;
            if !first.is_success() {
                return Err(SmtpError::Smtp(format!("EHLO 被拒绝: {}", first.text)));
            }
            let before = parse_extensions(&first);
            if !before.iter().any(|ext| ext.keyword == "STARTTLS") {
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
            Ok(parse_extensions(&second))
        }
    }
}

/// 认证。
///
/// 返回 `Some(outcome)` = 认证没过，提交就此结束；
/// `None` = 认证通过（或服务器压根不要求认证），可以继续往下走。
async fn authenticate(
    session: &mut SmtpSession,
    extensions: &[Extension],
    user: &str,
    password: &str,
) -> Result<Option<SubmitOutcome>, SmtpError> {
    let mechanisms = auth_mechanisms(extensions);
    if mechanisms.is_empty() {
        // 服务器没宣告 `AUTH`：没有认证机制可用。
        // 有些服务器（比如本地中继）确实不要认证——那就直接发。
        tracing::warn!("服务器没有宣告 AUTH 机制，跳过认证直接提交");
        return Ok(None);
    }

    let Some(mechanism) = select_mechanism(&mechanisms) else {
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
