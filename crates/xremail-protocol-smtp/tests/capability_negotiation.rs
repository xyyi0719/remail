//! ESMTP 能力协商的用例：`8BITMIME`（RFC 6152）与 `SMTPUTF8`（RFC 6531）。
//!
//! 覆盖（与需求一一对应）：
//!
//! 1. ASCII 地址 + 服务器宣告 `8BITMIME` → `MAIL FROM` 带 `BODY=8BITMIME`，**不带** `SMTPUTF8`。
//! 2. ASCII 地址 + 服务器**不**宣告 `8BITMIME` → `MAIL FROM` **不带** `BODY=8BITMIME`。
//! 3. 非 ASCII 地址 + 服务器宣告 `SMTPUTF8` → `MAIL FROM` 同时带 `BODY=8BITMIME` 与 `SMTPUTF8`。
//! 4. 非 ASCII 地址 + 服务器**不**宣告 `SMTPUTF8` → **明确报错**，且**一个 `MAIL FROM` 都没发出去**。
//! 5. 正文是 8 位 + 服务器**不**宣告 `8BITMIME` → **明确报错**。
//! 6. 服务器宣告了 `SMTPUTF8` 但地址全是 ASCII → **不**带 `SMTPUTF8`。
//! 7. 假服务器自己也守规矩：不合规的 `MAIL FROM` 它会拒。

use std::time::Duration;

use xremail_protocol_smtp::{
    Envelope, SmtpConfig, SmtpError, SmtpResponse, SubmitOutcome, SubmitRequest, TlsTrust,
    submit_message,
};
use xremail_testkit::{FakeSmtpServer, SmtpServerConfig};

const USER: &str = "user@example.com";
const PASSWORD: &str = "s3cret-pass";

/// 一个国际化的收件人地址（含非 ASCII）。
const UTF8_RECIPIENT: &str = "用户@例子.中国";
/// 一个国际化的发件人地址（含非 ASCII）。
const UTF8_SENDER: &str = "发件人@example.com";

/// 造一个指向假服务器的提交配置。
fn config_for(server: &FakeSmtpServer) -> SmtpConfig {
    let mut config = SmtpConfig::starttls(server.host(), server.port());
    config.connect_timeout = Duration::from_secs(5);
    config.command_timeout = Duration::from_secs(5);
    config
}

/// 假服务器只用自签证书，测试要把那张 CA 显式交给客户端。
fn trust(server: &FakeSmtpServer) -> TlsTrust {
    TlsTrust::Roots(server.root_store())
}

/// 造一个提交请求。
fn make_request(sender: &str, recipients: &[&str], message: &[u8]) -> SubmitRequest {
    SubmitRequest {
        envelope: Envelope {
            sender: sender.to_string(),
            recipients: recipients.iter().map(|r| r.to_string()).collect(),
        },
        message: message.to_vec(),
        eight_bit: false,
    }
}

/// 启动一个「标准」的假服务器：STARTTLS + `PLAIN`/`LOGIN`。
async fn start_server(config: SmtpServerConfig) -> FakeSmtpServer {
    FakeSmtpServer::start(config.with_auth(&["PLAIN", "LOGIN"], USER, PASSWORD))
        .await
        .expect("假服务器应当能启动")
}

/// 取服务器收到的 `MAIL FROM` 原始行（去掉行终止符）。
fn mail_from_line(server: &FakeSmtpServer) -> String {
    let line = server.mail_from().expect("应当收到过 MAIL FROM");
    line.trim_end().to_string()
}

/// 提交一封信并返回结果（**不**期待成功）。
async fn submit(
    server: &FakeSmtpServer,
    request: &SubmitRequest,
) -> Result<SubmitOutcome, SmtpError> {
    submit_message(&config_for(server), &trust(server), USER, PASSWORD, request).await
}

#[tokio::test]
async fn ascii_address_with_8bitmime_announcement_sends_body_parameter() {
    // 服务器宣告 `8BITMIME` → `MAIL FROM` 带 `BODY=8BITMIME`。
    let server = start_server(SmtpServerConfig::new()).await;

    let mut request = make_request(USER, &["friend@example.com"], b"Subject: x\r\n\r\nbody\r\n");
    request.eight_bit = true;

    let outcome = submit(&server, &request).await.expect("提交应当成功");
    assert!(matches!(outcome, SubmitOutcome::Accepted { .. }));

    let mail_from = mail_from_line(&server);
    server.shutdown().await;

    assert_eq!(
        mail_from, "MAIL FROM:<user@example.com> BODY=8BITMIME",
        "ASCII 地址不该带 SMTPUTF8，参数顺序也不该变"
    );
}

#[tokio::test]
async fn ascii_address_without_8bitmime_announcement_omits_body_parameter() {
    // 服务器**不**宣告 `8BITMIME` → `MAIL FROM` 绝不能带 `BODY=8BITMIME`。
    let server = start_server(SmtpServerConfig::new().without_8bitmime()).await;

    let request = make_request(USER, &["friend@example.com"], b"Subject: x\r\n\r\nbody\r\n");

    let outcome = submit(&server, &request).await.expect("提交应当成功");
    assert!(matches!(outcome, SubmitOutcome::Accepted { .. }));

    let mail_from = mail_from_line(&server);
    server.shutdown().await;

    assert_eq!(
        mail_from, "MAIL FROM:<user@example.com>",
        "服务器没宣告 8BITMIME 时不能带这个参数"
    );
}

#[tokio::test]
async fn non_ascii_address_with_smtputf8_announcement_sends_both_parameters() {
    // 服务器宣告 `SMTPUTF8` + `8BITMIME` → 两个参数都带。
    let server = start_server(SmtpServerConfig::new()).await;

    let mut request = make_request(USER, &[UTF8_RECIPIENT], b"Subject: x\r\n\r\nbody\r\n");
    request.eight_bit = true;

    let outcome = submit(&server, &request).await.expect("提交应当成功");
    assert!(matches!(outcome, SubmitOutcome::Accepted { .. }));

    let mail_from = mail_from_line(&server);
    let messages = server.messages();
    server.shutdown().await;

    assert!(
        mail_from.contains("BODY=8BITMIME"),
        "8 位正文要带 BODY=8BITMIME，实际: {mail_from:?}"
    );
    assert!(
        mail_from.contains("SMTPUTF8"),
        "非 ASCII 地址要带 SMTPUTF8，实际: {mail_from:?}"
    );
    assert_eq!(messages.len(), 1, "服务器应当收到这封信");
}

#[tokio::test]
async fn non_ascii_address_without_smtputf8_is_an_error_and_nothing_is_sent() {
    // 服务器**不**宣告 `SMTPUTF8` → 必须报错，而且**一个字都不能发出去**。
    let server = start_server(SmtpServerConfig::new().without_smtputf8()).await;

    let request = make_request(USER, &[UTF8_RECIPIENT], b"Subject: x\r\n\r\nbody\r\n");

    let result = submit(&server, &request).await;

    let mail_from_lines = server.mail_from_lines();
    let messages = server.messages();
    let commands = server.commands();
    server.shutdown().await;

    match result {
        Err(err) => {
            assert!(
                matches!(err, SmtpError::Unsupported(_)),
                "服务器不支持就该报「能力不支持」，实际: {err}"
            );
            let text = err.to_string();
            assert!(
                text.contains(UTF8_RECIPIENT),
                "报错要说清楚是哪个地址，实际: {text}"
            );
            assert!(
                text.contains("SMTPUTF8"),
                "报错要说清楚缺哪个能力，实际: {text}"
            );
        }
        Ok(outcome) => panic!("服务器不支持 SMTPUTF8 时应当报错，实际: {outcome:?}"),
    }

    assert!(
        mail_from_lines.is_empty(),
        "一个 MAIL FROM 都不该发出去，实际: {mail_from_lines:?}"
    );
    assert!(messages.is_empty(), "不该收到任何邮件");
    assert!(
        !commands.iter().any(|c| c == "DATA"),
        "不该走到 DATA，实际命令: {commands:?}"
    );
}

#[tokio::test]
async fn non_ascii_sender_without_smtputf8_is_an_error() {
    // 发件人地址含非 ASCII —— 同样要报错，也要说清楚是哪个地址。
    let server = start_server(SmtpServerConfig::new().without_smtputf8()).await;

    let request = make_request(
        UTF8_SENDER,
        &["friend@example.com"],
        b"Subject: x\r\n\r\nbody\r\n",
    );

    let result = submit(&server, &request).await;
    let mail_from_lines = server.mail_from_lines();
    server.shutdown().await;

    match result {
        Err(err) => {
            let text = err.to_string();
            assert!(matches!(err, SmtpError::Unsupported(_)), "实际: {err}");
            assert!(
                text.contains(UTF8_SENDER),
                "报错要说清楚是哪个地址，实际: {text}"
            );
        }
        Ok(outcome) => panic!("服务器不支持 SMTPUTF8 时应当报错，实际: {outcome:?}"),
    }
    assert!(
        mail_from_lines.is_empty(),
        "一个 MAIL FROM 都不该发出去，实际: {mail_from_lines:?}"
    );
}

#[tokio::test]
async fn eight_bit_body_without_8bitmime_announcement_is_an_error() {
    // 正文是 8 位，但服务器**不**宣告 `8BITMIME` → 明确报错。
    let server = start_server(SmtpServerConfig::new().without_8bitmime()).await;

    let mut request = make_request(USER, &["friend@example.com"], b"Subject: x\r\n\r\nbody\r\n");
    request.eight_bit = true;

    let result = submit(&server, &request).await;
    let mail_from_lines = server.mail_from_lines();
    server.shutdown().await;

    match result {
        Err(err) => {
            assert!(matches!(err, SmtpError::Unsupported(_)), "实际: {err}");
            let text = err.to_string();
            assert!(
                text.contains("8BITMIME"),
                "报错要说清楚缺哪个能力，实际: {text}"
            );
        }
        Ok(outcome) => panic!("服务器不支持 8BITMIME 时应当报错，实际: {outcome:?}"),
    }
    assert!(
        mail_from_lines.is_empty(),
        "一个 MAIL FROM 都不该发出去，实际: {mail_from_lines:?}"
    );
}

#[tokio::test]
async fn smtputf8_is_not_sent_for_ascii_addresses() {
    // 服务器**宣告**了 `SMTPUTF8`，但地址全是 ASCII → 依然不带这个参数。
    // 带上没用，服务器只会拿它当「这封信用了 UTF-8」的声明。
    let server = start_server(SmtpServerConfig::new().with_smtputf8()).await;

    let request = make_request(USER, &["friend@example.com"], b"Subject: x\r\n\r\nbody\r\n");

    let outcome = submit(&server, &request).await.expect("提交应当成功");
    assert!(matches!(outcome, SubmitOutcome::Accepted { .. }));

    let mail_from = mail_from_line(&server);
    server.shutdown().await;

    assert!(
        !mail_from.contains("SMTPUTF8"),
        "ASCII 地址不该带 SMTPUTF8，实际: {mail_from:?}"
    );
}

#[tokio::test]
async fn ehlo_capabilities_are_parsed_into_a_model() {
    // 直接验能力模型：`EHLO` 应答 → 关键字与支持度。
    let response = SmtpResponse {
        code: 250,
        lines: vec![
            "250-smtp.example.com Hello".to_string(),
            "250-SIZE 73400320".to_string(),
            "250-8BITMIME".to_string(),
            "250-SMTPUTF8".to_string(),
            "250-AUTH LOGIN PLAIN".to_string(),
            "250-PIPELINING".to_string(),
        ],
        text: "PIPELINING".to_string(),
    };

    let capabilities = xremail_protocol_smtp::SmtpCapabilities::parse(&response);

    assert!(capabilities.supports_8bitmime(), "应当认出 8BITMIME");
    assert!(capabilities.supports_smtputf8(), "应当认出 SMTPUTF8");
    assert_eq!(
        capabilities.auth_mechanisms,
        vec!["LOGIN".to_string(), "PLAIN".to_string()],
        "应当认出 AUTH 的机制"
    );
    assert_eq!(capabilities.support.size, Some(73_400_320));
    assert!(capabilities.support.pipelining);
    assert!(capabilities.has_keyword("8BITMIME"));
}

#[tokio::test]
async fn fake_server_rejects_smtputf8_parameter_it_did_not_announce() {
    // 假服务器自己也要守规矩：没宣告 `SMTPUTF8` 却收到这个参数 → 555。
    let server = FakeSmtpServer::start(SmtpServerConfig::new().without_auth().without_smtputf8())
        .await
        .expect("假服务器应当能启动");

    let replies = raw_exchange(
        &server,
        &["EHLO localhost", "MAIL FROM:<user@example.com> SMTPUTF8"],
    )
    .await;
    server.shutdown().await;

    let last = replies.last().expect("应当有应答");
    assert!(
        last.starts_with("555"),
        "没宣告 SMTPUTF8 却带这个参数应当被拒（555），实际: {last:?}"
    );
}

#[tokio::test]
async fn fake_server_rejects_internationalized_address_without_the_parameter() {
    // 宣告了 `SMTPUTF8`，但客户端没带参数就发国际化地址 → 553。
    let server = FakeSmtpServer::start(SmtpServerConfig::new().without_auth().with_smtputf8())
        .await
        .expect("假服务器应当能启动");

    let replies = raw_exchange(&server, &["EHLO localhost", "MAIL FROM:<用户@例子.中国>"]).await;
    server.shutdown().await;

    let last = replies.last().expect("应当有应答");
    assert!(
        last.starts_with("553"),
        "国际化地址不带 SMTPUTF8 参数应当被拒（553），实际: {last:?}"
    );
}

/// `SMTPUTF8` 是**事务级**参数：跟着 `MAIL FROM` 走。
#[tokio::test]
async fn fake_server_tracks_smtputf8_at_transaction_level() {
    // `MAIL FROM` 带 `SMTPUTF8` → 事务里的 `RCPT TO` 允许非 ASCII；
    // `MAIL FROM` 不带 → 事务里的 `RCPT TO` 非 ASCII 拒收（553）。
    let server = FakeSmtpServer::start(SmtpServerConfig::new().without_auth().with_smtputf8())
        .await
        .expect("假服务器应当能启动");

    let accepted = raw_exchange(
        &server,
        &[
            "EHLO localhost",
            "MAIL FROM:<user@example.com> SMTPUTF8",
            "RCPT TO:<用户@例子.中国>",
        ],
    )
    .await;

    let rejected = raw_exchange(
        &server,
        &[
            "EHLO localhost",
            "MAIL FROM:<user@example.com>",
            "RCPT TO:<用户@例子.中国>",
        ],
    )
    .await;
    server.shutdown().await;

    assert!(
        accepted
            .last()
            .is_some_and(|reply| reply.starts_with("250")),
        "SMTPUTF8 事务里应当接受非 ASCII 收件人，实际: {:?}",
        accepted.last()
    );
    assert!(
        rejected
            .last()
            .is_some_and(|reply| reply.starts_with("553")),
        "非 SMTPUTF8 事务里应当拒绝非 ASCII 收件人，实际: {:?}",
        rejected.last()
    );
}

/// 用**裸 TCP** 和假服务器说话，返回每条命令的应答（多行应答用 ` | ` 连接）。
///
/// 客户端库不会发出不合规的 `MAIL FROM`，所以要验「假服务器自己守规矩」
/// 只能用裸连接手写命令。
async fn raw_exchange(server: &FakeSmtpServer, commands: &[&str]) -> Vec<String> {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

    let mut stream = tokio::net::TcpStream::connect((server.host(), server.port()))
        .await
        .expect("应当能连上假服务器");
    let (read_half, mut write_half) = stream.split();
    let mut reader = BufReader::new(read_half);

    // 先读问候。
    let mut greeting = String::new();
    reader.read_line(&mut greeting).await.expect("应当读到问候");

    let mut replies = Vec::new();
    for command in commands {
        write_half
            .write_all(format!("{command}\r\n").as_bytes())
            .await
            .expect("应当能写命令");
        write_half.flush().await.expect("应当能刷出命令");

        // 多行应答：最后一行是「<code> <text>」，前面的是「<code>-<text>」。
        let mut lines = Vec::new();
        loop {
            let mut line = String::new();
            let read = reader.read_line(&mut line).await.expect("应当能读应答");
            assert!(read > 0, "连接被关掉了");
            let trimmed = line.trim_end().to_string();
            let is_final = trimmed.len() > 3 && trimmed.as_bytes()[3] == b' ';
            lines.push(trimmed);
            if is_final {
                break;
            }
        }
        replies.push(lines.join(" | "));
    }

    replies
}
