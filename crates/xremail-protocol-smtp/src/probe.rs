//! 只读 SMTP 探测。
//!
//! ## 严格只读
//!
//! 探测只执行 `EHLO` / `STARTTLS` / `AUTH` / `QUIT`——
//! **绝不**执行 `MAIL FROM` / `RCPT TO` / `DATA`。
//! 理由见 [`crate::session`] 模块的说明：探测工具一旦带上有投递能力的命令，
//! 「只是跑一下探测」和「误发一封信」之间就没有技术屏障了。
//!
//! ## 流程
//!
//! 1. 读 `220` 问候。
//! 2. `EHLO`，解析能力集。
//! 3. 需要 STARTTLS 时：确认服务器**宣告**了 `STARTTLS` → 升级 → **重新 EHLO**
//!    （RFC 3207 §4.2）。
//! 4. 按服务器宣告的机制认证（`PLAIN` / `LOGIN`）。
//! 5. `QUIT`。
//!
//! ## 脱敏
//!
//! 报告里**不出现**口令。用户名由调用方自己脱敏（见 `mask_user`）。

use crate::error::SmtpError;
use crate::session::{AuthMechanism, SmtpResponse, SmtpSession};
use crate::transport::{SmtpConfig, TlsInfo, TlsMode, TlsTrust};

/// 探测配置。
#[derive(Debug, Clone)]
pub struct ProbeConfig {
    /// 服务器配置。
    pub server: SmtpConfig,
    /// `EHLO` 自报的域名。
    pub ehlo_domain: String,
    /// 是否尝试认证。
    pub authenticate: bool,
    /// 优先使用的认证机制（`None` = 按固定优先级自动挑）。
    pub preferred_mechanism: Option<AuthMechanism>,
    /// 严格 TLS 模式：服务器拒绝 ClientHello 就**直接报错**，不降级重试。
    ///
    /// 默认 `false`（允许回退到 TLS 1.2 并在报告里写明）。
    pub strict_tls: bool,
}

impl ProbeConfig {
    /// 用服务器配置构造（默认会认证）。
    pub fn new(server: SmtpConfig) -> Self {
        Self {
            server,
            ehlo_domain: "localhost".to_string(),
            authenticate: true,
            preferred_mechanism: None,
            strict_tls: false,
        }
    }

    /// 只探测，不认证。
    pub fn without_auth(mut self) -> Self {
        self.authenticate = false;
        self
    }

    /// 严格模式：不接受 TLS 版本回退。
    pub fn strict_tls(mut self) -> Self {
        self.strict_tls = true;
        self
    }
}

/// 一条 EHLO 扩展。
#[derive(Debug, Clone)]
pub struct Extension {
    /// 关键字（大写），如 `8BITMIME`。
    pub keyword: String,
    /// 参数（如 `SIZE` 的 `52428800`、`AUTH` 的 `LOGIN PLAIN`）。
    pub params: Vec<String>,
    /// 服务器返回的原文（不含 `250-` 前缀）。
    pub raw: String,
}

/// 扩展支持度。
///
/// 每一项都对应发送链路里的一个具体决策：能不能发 8 位正文、
/// 能不能用 UTF-8 标题、能不能一次发很大的附件。
#[derive(Debug, Clone, Default)]
pub struct ExtensionSupport {
    /// `8BITMIME` —— 允许 `BODY=8BITMIME`。
    pub eight_bit_mime: bool,
    /// `SMTPUTF8` —— 允许 UTF-8 邮箱地址与非 ASCII 标题。
    pub smtputf8: bool,
    /// `SIZE` —— 单封邮件大小上限（字节）。
    pub size: Option<u64>,
    /// `STARTTLS` —— 支持明文连接后升级到 TLS。
    pub starttls: bool,
    /// `PIPELINING` —— 支持命令流水线。
    pub pipelining: bool,
    /// `DSN` —— 支持投递状态通知。
    pub dsn: bool,
    /// `CHUNKING` —— 支持 `BDAT` 分块传输。
    pub chunking: bool,
    /// `BINARYMIME` —— 支持二进制 MIME 传输。
    pub binary_mime: bool,
    /// `ENHANCEDSTATUSCODES` —— 增强状态码。
    pub enhanced_status_codes: bool,
    /// `REQUIRETLS` —— 服务器**要求**必须走 TLS。
    pub requiretls: bool,
    /// `AUTH` —— 宣告了认证机制。
    pub auth: bool,
}

/// 认证结果。
#[derive(Debug, Clone)]
pub enum AuthOutcome {
    /// 没认证（`--no-auth`）。
    Skipped,
    /// 服务器没有宣告任何**本客户端能实现**的机制。
    Unsupported {
        /// 服务器宣告的全部机制（含没实现的）。
        mechanisms: Vec<String>,
    },
    /// 认证成功。
    Success {
        /// 实际用的机制。
        mechanism: AuthMechanism,
        /// 服务器的原文（如 `235 Authentication successful`）。
        response: String,
    },
    /// 认证失败。
    ///
    /// `response` 是**服务器返回的原文**，其中**不含口令**。
    Failed {
        /// 实际用的机制。
        mechanism: AuthMechanism,
        /// 服务器的原文（如 `535 5.7.8 Error: authentication failed`）。
        response: String,
    },
}

/// 探测报告。
#[derive(Debug, Clone)]
pub struct ProbeReport {
    /// 服务器问候原文（整行，如 `220 smtp.qq.com ESMTP Tencent ESMail`）。
    pub greeting: String,
    /// TLS 会话信息（含证书体检）。没走 TLS 时是 `None`。
    pub tls: Option<TlsInfo>,
    /// 实际使用的 TLS 模式。
    pub tls_mode: TlsMode,
    /// STARTTLS 是否真的执行了。
    pub starttls_performed: bool,
    /// 明文阶段的 EHLO 扩展（走 STARTTLS 时有内容；隐式 TLS 时为空）。
    pub ehlo_before_tls: Vec<Extension>,
    /// TLS 阶段的 EHLO 扩展（**权威**：握手后服务器可能改变能力集）。
    pub ehlo: Vec<Extension>,
    /// 扩展支持度（以 TLS 阶段的 EHLO 为准）。
    pub extensions: ExtensionSupport,
    /// 明文阶段宣告的 AUTH 机制。
    pub auth_mechanisms_before_tls: Vec<String>,
    /// TLS 阶段宣告的 AUTH 机制（**权威**）。
    pub auth_mechanisms: Vec<String>,
    /// 认证结果。
    pub auth: AuthOutcome,
    /// TLS 版本回退记录。
    ///
    /// 服务器**拒绝**了 TLS 1.3 的 ClientHello（只认 1.2）时，
    /// 探测会自动改用 TLS 1.2 **重试整条链路**，并在这里写明。
    ///
    /// 这是**版本问题**的处置，不是**信任问题**的让步：
    /// 证书始终按系统根校验、主机名始终核对，**一点没有放松**。
    pub tls_version_fallback: Option<TlsFallback>,
}

/// TLS 版本回退记录。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TlsFallback {
    /// 服务器拒绝时的原始信息（来自 fatal alert）。
    pub rejected_detail: String,
    /// 回退后实际使用的 TLS 版本（如 `TLSv1.2`）。
    pub used_version: String,
}

/// 执行只读探测（用**系统根**校验服务端证书）。
pub async fn probe(
    config: &ProbeConfig,
    user: &str,
    password: &str,
) -> Result<ProbeReport, SmtpError> {
    probe_with_trust(config, &TlsTrust::SystemRoots, user, password).await
}

/// 执行只读探测，信任来源可指定。
///
/// [`TlsTrust::Roots`] 只给**测试**用（假服务器的自签证书）。
///
/// ## TLS 版本回退
///
/// 先用默认版本集（TLS 1.3 + 1.2）试一次。服务器**拒绝** ClientHello
/// （只认 TLS 1.2，比如 `smtp.wo.cn`）时，会**改用 TLS 1.2 重试整条链路**，
/// 并把回退写进 [`ProbeReport::tls_version_fallback`]。
///
/// 要严格模式（拒绝就报错、不降级），用 [`ProbeConfig::strict_tls`]。
pub async fn probe_with_trust(
    config: &ProbeConfig,
    trust: &TlsTrust,
    user: &str,
    password: &str,
) -> Result<ProbeReport, SmtpError> {
    if config.strict_tls {
        return probe_once(config, trust, user, password).await;
    }

    match probe_once(config, trust, user, password).await {
        Err(SmtpError::TlsVersionRejected(detail)) => {
            let mut retry = config.clone();
            retry.server = config.server.clone().with_tls12_only();
            let mut report = probe_once(&retry, trust, user, password).await?;
            report.tls_version_fallback = Some(TlsFallback {
                rejected_detail: detail,
                used_version: report
                    .tls
                    .as_ref()
                    .map(|tls| tls.protocol.clone())
                    .unwrap_or_else(|| "TLSv1.2".to_string()),
            });
            Ok(report)
        }
        other => other,
    }
}

/// 探测的**单次**尝试（不做任何回退）。
async fn probe_once(
    config: &ProbeConfig,
    trust: &TlsTrust,
    user: &str,
    password: &str,
) -> Result<ProbeReport, SmtpError> {
    let server = &config.server;

    let mut session = match server.tls_mode {
        TlsMode::Direct => SmtpSession::connect_tls(server, trust).await?,
        TlsMode::Starttls => SmtpSession::connect_plaintext(server).await?,
    };

    // 1) 问候。
    let greeting = session.greeting().await?;
    if greeting.code != 220 {
        return Err(SmtpError::Smtp(format!(
            "服务器问候不是 220: {}",
            greeting.text
        )));
    }
    let greeting_raw = greeting
        .lines
        .first()
        .map(|l| l.trim().to_string())
        .unwrap_or_default();

    // 2) EHLO。
    //
    // 明文（STARTTLS）模式下，第一次 EHLO 发生在**明文**阶段：
    // 它回答了「服务器告不宣告 STARTTLS」。
    // 隐式 TLS 模式下，第一次 EHLO 就已经在 TLS 里了。
    let mut ehlo_before_tls: Vec<Extension> = Vec::new();
    let mut auth_mechanisms_before_tls: Vec<String> = Vec::new();
    let mut starttls_performed = false;

    let ehlo: Vec<Extension> = if server.tls_mode == TlsMode::Starttls {
        let first = session.ehlo(&config.ehlo_domain).await?;
        if !first.is_success() {
            return Err(SmtpError::Smtp(format!("EHLO 被拒绝: {}", first.text)));
        }
        ehlo_before_tls = parse_extensions(&first);
        auth_mechanisms_before_tls = auth_mechanisms(&ehlo_before_tls);

        if !ehlo_before_tls.iter().any(|e| e.keyword == "STARTTLS") {
            // 服务器没有宣告 STARTTLS → **明确报错**，不降级到明文。
            // 在明文上继续意味着口令会裸奔在网络上。
            return Err(SmtpError::Unsupported(format!(
                "服务器 {}:{} 没有宣告 STARTTLS，拒绝在明文上继续",
                server.host, server.port
            )));
        }

        session.starttls(trust).await?;
        starttls_performed = true;

        // RFC 3207 §4.2：握手之后**必须**重新 EHLO——
        // 很多服务器只在 TLS 之后才宣告 AUTH。
        let second = session.ehlo(&config.ehlo_domain).await?;
        if !second.is_success() {
            return Err(SmtpError::Smtp(format!(
                "TLS 升级后的 EHLO 被拒绝: {}",
                second.text
            )));
        }
        parse_extensions(&second)
    } else {
        let first = session.ehlo(&config.ehlo_domain).await?;
        if !first.is_success() {
            return Err(SmtpError::Smtp(format!("EHLO 被拒绝: {}", first.text)));
        }
        parse_extensions(&first)
    };

    let extensions = extension_support(&ehlo);
    let mechanisms = auth_mechanisms(&ehlo);
    let tls = session.tls_info();

    // 4) 认证。
    let auth = authenticate(&mut session, config, &mechanisms, user, password).await;

    // 5) 收尾。
    let _ = session.quit().await;

    Ok(ProbeReport {
        greeting: greeting_raw,
        tls,
        tls_mode: server.tls_mode,
        starttls_performed,
        ehlo_before_tls,
        ehlo,
        extensions,
        auth_mechanisms_before_tls,
        auth_mechanisms: mechanisms,
        auth,
        tls_version_fallback: None,
    })
}

/// 认证。
///
/// 认证失败**不算探测失败**：报告里要如实写「成功 / 失败（服务器原文）」，
/// 否则用户看不出是「机制不对」还是「口令不对」。
async fn authenticate(
    session: &mut SmtpSession,
    config: &ProbeConfig,
    mechanisms: &[String],
    user: &str,
    password: &str,
) -> AuthOutcome {
    if !config.authenticate {
        return AuthOutcome::Skipped;
    }

    let Some(mechanism) = select_mechanism(mechanisms, config.preferred_mechanism) else {
        return AuthOutcome::Unsupported {
            mechanisms: mechanisms.to_vec(),
        };
    };

    let result = match mechanism {
        AuthMechanism::Plain => session.auth_plain(user, password).await,
        AuthMechanism::Login => session.auth_login(user, password).await,
        AuthMechanism::XOAuth2 => Err(SmtpError::Unsupported(
            "XOAUTH2 尚未实现（它需要 OAuth 令牌，不是账号口令）".into(),
        )),
    };

    match result {
        Ok(response) if response.is_success() => AuthOutcome::Success {
            mechanism,
            response: response.raw_text(),
        },
        Ok(response) => AuthOutcome::Failed {
            mechanism,
            response: response.raw_text(),
        },
        // 错误信息里不含口令：这里带出去的是服务器原文或传输层错误。
        Err(err) => AuthOutcome::Failed {
            mechanism,
            response: err.to_string(),
        },
    }
}

/// 选一个可用的认证机制。
///
/// 默认顺序 `LOGIN` → `PLAIN`：`LOGIN` 是更老、几乎所有服务器都实现的机制；
/// `PLAIN` 在部分服务器上要求先完成 TLS 升级才开放。
/// **哪家到底用哪个，只能实测**——这就是这个工具存在的理由。
fn select_mechanism(
    mechanisms: &[String],
    preferred: Option<AuthMechanism>,
) -> Option<AuthMechanism> {
    let available: Vec<AuthMechanism> = mechanisms
        .iter()
        .filter_map(|m| AuthMechanism::parse(m))
        .collect();

    if let Some(preferred) = preferred {
        return available.contains(&preferred).then_some(preferred);
    }

    [AuthMechanism::Login, AuthMechanism::Plain]
        .into_iter()
        .find(|candidate| available.contains(candidate))
}
/// 把 EHLO 响应解析成扩展清单。
///
/// 响应第一行是服务器问候（`250-smtp.163.com Hello`），
/// 后面每行一条扩展（RFC 5321 §4.1.4）。
pub fn parse_extensions(response: &SmtpResponse) -> Vec<Extension> {
    response
        .texts()
        .into_iter()
        // 跳过第一行（问候）。
        .skip(1)
        .map(|raw| {
            let mut parts = raw.split_whitespace();
            let mut keyword = parts.next().unwrap_or_default().to_ascii_uppercase();
            let mut params: Vec<String> = Vec::new();

            // 老式写法 `AUTH=LOGIN PLAIN`：等号后面是参数。
            if let Some(idx) = keyword.find('=') {
                let rest = keyword[idx + 1..].to_string();
                keyword = keyword[..idx].to_string();
                if !rest.is_empty() {
                    params.push(rest);
                }
            }
            params.extend(parts.map(|s| s.to_string()));

            Extension {
                keyword,
                params,
                raw,
            }
        })
        .collect()
}

/// 从扩展清单里解析 AUTH 机制。
pub fn auth_mechanisms(extensions: &[Extension]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();

    for ext in extensions {
        if ext.keyword != "AUTH" {
            continue;
        }
        out.extend(ext.params.iter().filter(|p| !p.is_empty()).cloned());
    }

    out.sort();
    out.dedup();
    out
}

/// 从扩展清单里提取支持度。
pub fn extension_support(extensions: &[Extension]) -> ExtensionSupport {
    let mut support = ExtensionSupport::default();

    for ext in extensions {
        match ext.keyword.as_str() {
            "8BITMIME" => support.eight_bit_mime = true,
            "SMTPUTF8" => support.smtputf8 = true,
            "STARTTLS" => support.starttls = true,
            "PIPELINING" => support.pipelining = true,
            "DSN" => support.dsn = true,
            "CHUNKING" => support.chunking = true,
            "BINARYMIME" => support.binary_mime = true,
            "ENHANCEDSTATUSCODES" => support.enhanced_status_codes = true,
            "REQUIRETLS" => support.requiretls = true,
            "AUTH" => support.auth = true,
            "SIZE" => support.size = ext.params.first().and_then(|p| p.parse().ok()),
            _ => {}
        }
    }

    support
}
