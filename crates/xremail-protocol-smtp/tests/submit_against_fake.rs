//! 用假 SMTP 服务器验证**真发信**（提交）链路。
//!
//! 探测链路有 [`probe_against_fake.rs`](probe_against_fake.rs) 守着；
//! 这个文件守的是**真的把信发出去**的那条路。
//!
//! 覆盖（与需求一一对应）：
//!
//! 1. 正常提交：服务器收到 `MAIL FROM` → `RCPT TO` → `DATA` → 回 250。
//! 2. 多收件人：每个收件人一条 `RCPT TO`。
//! 3. 认证失败 → `Rejected { stage: Auth }`，且**没有** `MAIL FROM`。
//! 4. `RCPT` 被拒 → `Rejected { stage: RcptTo }`，且**没有** `DATA`。
//! 5. 4xx 可重试、5xx 不可重试、`Uncertain` 不可重试。
//! 6. `DATA` 之后最终应答丢失 → `Uncertain { stage: Data }`。
//! 7. `8BITMIME`：服务器支持才带 `BODY=8BITMIME`；服务器**不支持**而邮件确实是
//!    8 位 → **报错**（一个字节都不发出去）。
//! 8. dot-stuffing：正文里以 `.` 开头的行，线上变成 `..`。
//! 9. 写给服务器的行一律 CRLF。
//! 10. `SIZE` 限制：服务器声明的上限要**真的**生效。
//! 11. 无论成功失败都尽量 `QUIT`。
//! 12. `SMTPUTF8`：**只有**地址含非 ASCII 且服务器宣告了它才带（能力协商的完整
//!     用例见 [`capability_negotiation.rs`](capability_negotiation.rs)）。

use std::time::Duration;

use xremail_protocol_smtp::{
    Envelope, SmtpConfig, SmtpError, SubmitOutcome, SubmitRequest, SubmitStage, TlsTrust,
    submit_message,
};
use xremail_testkit::{FakeSmtpServer, SmtpServerConfig};

const USER: &str = "user@example.com";
const PASSWORD: &str = "s3cret-pass";

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

/// 把结果拆出来：不是 `Accepted` 就 panic，并把细节打出来。
fn expect_accepted(outcome: &SubmitOutcome) -> (u16, String) {
    match outcome {
        SubmitOutcome::Accepted { code, message } => (*code, message.clone()),
        other => panic!("应当被接受，实际: {other:?}"),
    }
}

#[tokio::test]
async fn normal_submission_is_accepted_and_the_message_is_stored() {
    let server = start_server(SmtpServerConfig::new()).await;

    // 用 CRLF 的正文：线上与存储都应该与输入**逐字节相同**。
    let message = b"Subject: hello\r\nFrom: user@example.com\r\n\r\nbody line\r\n";
    let request = make_request(USER, &["friend@example.com"], message);

    let outcome = submit_message(
        &config_for(&server),
        &trust(&server),
        USER,
        PASSWORD,
        &request,
    )
    .await
    .expect("提交应当成功");

    let (code, _) = expect_accepted(&outcome);
    assert_eq!(code, 250, "服务器应当回 250");

    // 服务器确实收到了完整邮件字节。
    let messages = server.messages();
    assert_eq!(messages.len(), 1, "应当只收到一封信");
    assert_eq!(messages[0], message.to_vec(), "收到的邮件与输入不一致");

    let commands = server.commands();
    server.shutdown().await;

    assert!(commands.contains(&"MAIL".to_string()));
    assert!(commands.contains(&"RCPT".to_string()));
    assert!(commands.contains(&"DATA".to_string()));
}

#[tokio::test]
async fn three_recipients_produce_exactly_three_rcpt_commands() {
    let server = start_server(SmtpServerConfig::new()).await;

    let request = make_request(
        USER,
        &["a@example.com", "b@example.com", "c@example.com"],
        b"Subject: many\r\n\r\nbody\r\n",
    );

    let outcome = submit_message(
        &config_for(&server),
        &trust(&server),
        USER,
        PASSWORD,
        &request,
    )
    .await
    .expect("提交应当成功");
    expect_accepted(&outcome);

    let commands = server.commands();
    let lines = server.command_lines();
    server.shutdown().await;

    let rcpt_count = commands.iter().filter(|c| *c == "RCPT").count();
    assert_eq!(rcpt_count, 3, "每个收件人一条 RCPT，实际: {commands:?}");

    // 每个地址各一条，顺序一致。
    let rcpt_lines: Vec<&String> = lines
        .iter()
        .filter(|line| line.starts_with("RCPT TO:"))
        .collect();
    assert_eq!(rcpt_lines.len(), 3, "实际: {lines:?}");
    assert!(rcpt_lines[0].contains("a@example.com"));
    assert!(rcpt_lines[1].contains("b@example.com"));
    assert!(rcpt_lines[2].contains("c@example.com"));
}

#[tokio::test]
async fn auth_failure_is_rejected_before_any_mail_from() {
    let server = start_server(SmtpServerConfig::new()).await;

    let request = make_request(USER, &["friend@example.com"], b"Subject: x\r\n\r\nbody\r\n");

    let outcome = submit_message(
        &config_for(&server),
        &trust(&server),
        USER,
        "wrong-password",
        &request,
    )
    .await
    .expect("认证失败也要有结果，不是传输错误");

    match &outcome {
        SubmitOutcome::Rejected { code, stage, .. } => {
            assert_eq!(*stage, SubmitStage::Auth);
            assert_eq!(*code, 535, "服务器应当回 535");
        }
        other => panic!("应当是被拒收，实际: {other:?}"),
    }

    let commands = server.commands();
    server.shutdown().await;

    // 关键红线：认证没过就**不许**发 `MAIL FROM`。
    assert!(
        !commands.contains(&"MAIL".to_string()),
        "认证失败后不该发 MAIL FROM，实际命令: {commands:?}"
    );
    assert!(
        !commands.contains(&"DATA".to_string()),
        "认证失败后不该发 DATA，实际命令: {commands:?}"
    );
}

#[tokio::test]
async fn rejected_recipient_stops_before_data() {
    let server =
        start_server(SmtpServerConfig::new().rejecting_recipient("nobody@example.com", 550)).await;

    let request = make_request(
        USER,
        &["ok@example.com", "nobody@example.com", "third@example.com"],
        b"Subject: x\r\n\r\nbody\r\n",
    );

    let outcome = submit_message(
        &config_for(&server),
        &trust(&server),
        USER,
        PASSWORD,
        &request,
    )
    .await
    .expect("提交应当有结果");

    match &outcome {
        SubmitOutcome::Rejected { code, stage, .. } => {
            assert_eq!(*stage, SubmitStage::RcptTo);
            assert_eq!(*code, 550);
        }
        other => panic!("应当是被拒收，实际: {other:?}"),
    }

    let commands = server.commands();
    server.shutdown().await;

    // 任何一个收件人被拒，就**不许**发 DATA。
    assert!(
        !commands.contains(&"DATA".to_string()),
        "RCPT 被拒后不该发 DATA，实际命令: {commands:?}"
    );
    // 也不该给第三个收件人发 RCPT。
    let rcpt_count = commands.iter().filter(|c| *c == "RCPT").count();
    assert_eq!(rcpt_count, 2, "被拒后应当停住，实际命令: {commands:?}");
}

#[tokio::test]
async fn retryability_follows_the_response_code() {
    // 4xx = 临时故障 → 可以重试。
    let server =
        start_server(SmtpServerConfig::new().rejecting_recipient("busy@example.com", 450)).await;

    let request = make_request(USER, &["busy@example.com"], b"Subject: x\r\n\r\nbody\r\n");
    let outcome = submit_message(
        &config_for(&server),
        &trust(&server),
        USER,
        PASSWORD,
        &request,
    )
    .await
    .expect("提交应当有结果");

    match &outcome {
        SubmitOutcome::Rejected { code, .. } => assert_eq!(*code, 450),
        other => panic!("应当是被拒收，实际: {other:?}"),
    }
    assert!(
        outcome.is_retryable(),
        "4xx 是临时故障，应当可以重试: {outcome:?}"
    );
    server.shutdown().await;

    // 5xx = 永久拒收 → 不该重试。
    let rejected_5xx = SubmitOutcome::Rejected {
        code: 550,
        message: "no such user".into(),
        stage: SubmitStage::RcptTo,
    };
    assert!(!rejected_5xx.is_retryable(), "5xx 是永久拒收，不该重试");

    // 不确定 ≠ 可重试：可能已经发出去了。
    let uncertain = SubmitOutcome::Uncertain {
        reason: "连接被对端关闭".into(),
        stage: SubmitStage::Data,
    };
    assert!(
        !uncertain.is_retryable(),
        "不确定状态下盲重发可能变成两封信，绝不能自动重试"
    );

    // 已经发出去了 → 更不能重试。
    let accepted = SubmitOutcome::Accepted {
        code: 250,
        message: "Ok".into(),
    };
    assert!(!accepted.is_retryable(), "已接受的信不能重发");
}

#[tokio::test]
async fn lost_final_response_after_data_is_uncertain() {
    // 服务器收完 DATA 之后**直接断连接**，连最终应答都不回。
    let server = start_server(SmtpServerConfig::new().dropping_data_response()).await;

    let message = b"Subject: maybe\r\n\r\nbody\r\n";
    let request = make_request(USER, &["friend@example.com"], message);

    let outcome = submit_message(
        &config_for(&server),
        &trust(&server),
        USER,
        PASSWORD,
        &request,
    )
    .await
    .expect("提交应当有结果");

    match &outcome {
        SubmitOutcome::Uncertain { stage, .. } => {
            assert_eq!(*stage, SubmitStage::Data, "不确定发生在 DATA 之后");
        }
        other => panic!("必须是「不确定」，实际: {other:?}"),
    }

    assert!(
        !outcome.is_retryable(),
        "不确定状态不能自动重发（可能已经发出去了）"
    );

    // 服务器**确实**收到了正文——所以「不确定」不是空想：
    // 邮件可能已经被收下，只是应答没到。
    let messages = server.messages();
    server.shutdown().await;
    assert_eq!(messages.len(), 1, "服务器应当已经收到了正文");
    assert_eq!(messages[0], message.to_vec());
}

#[tokio::test]
async fn eight_bitmime_parameter_follows_server_support() {
    // 服务器宣告了 `8BITMIME` → `MAIL FROM` 带 `BODY=8BITMIME`。
    let server = start_server(SmtpServerConfig::new()).await;

    let mut request = make_request(USER, &["friend@example.com"], b"Subject: x\r\n\r\nbody\r\n");
    request.eight_bit = true;

    let outcome = submit_message(
        &config_for(&server),
        &trust(&server),
        USER,
        PASSWORD,
        &request,
    )
    .await
    .expect("提交应当成功");
    expect_accepted(&outcome);

    let lines = server.command_lines();
    server.shutdown().await;

    let mail_from = lines
        .iter()
        .find(|line| line.starts_with("MAIL FROM:"))
        .expect("应当有 MAIL FROM");
    assert!(
        mail_from.contains("BODY=8BITMIME"),
        "服务器支持 8BITMIME 时要带 BODY=8BITMIME，实际: {mail_from:?}"
    );
}

#[tokio::test]
async fn eight_bitmime_parameter_is_omitted_when_unsupported() {
    // 服务器**没有**宣告 `8BITMIME` → 绝不能带 `BODY=8BITMIME`。
    let server =
        start_server(SmtpServerConfig::new().with_extensions(&["SIZE 52428800", "PIPELINING"]))
            .await;

    // 这封信是纯 7 位 → 不需要任何 ESMTP 参数。
    let request = make_request(USER, &["friend@example.com"], b"Subject: x\r\n\r\nbody\r\n");

    let outcome = submit_message(
        &config_for(&server),
        &trust(&server),
        USER,
        PASSWORD,
        &request,
    )
    .await
    .expect("提交应当成功");
    expect_accepted(&outcome);

    let lines = server.command_lines();
    server.shutdown().await;

    let mail_from = lines
        .iter()
        .find(|line| line.starts_with("MAIL FROM:"))
        .expect("应当有 MAIL FROM");
    assert!(
        !mail_from.contains("8BITMIME"),
        "服务器不支持 8BITMIME 时不能带这个参数，实际: {mail_from:?}"
    );
}

#[tokio::test]
async fn eight_bit_body_is_refused_when_server_does_not_announce_8bitmime() {
    // 服务器**没有**宣告 `8BITMIME`，而这封信确实是 8 位 →
    // RFC 6152 §3：客户端不能在没有 8BITMIME 的服务器上发 8 位正文。
    // 所以必须**报错**，而不是「默默降级成 7 位」——
    // 默默降级会把已经被编码成 8 位的正文原样发出去（对方按 7 位解，乱码）。
    let server =
        start_server(SmtpServerConfig::new().with_extensions(&["SIZE 52428800", "PIPELINING"]))
            .await;

    let mut request = make_request(USER, &["friend@example.com"], b"Subject: x\r\n\r\nbody\r\n");
    request.eight_bit = true;

    let result = submit_message(
        &config_for(&server),
        &trust(&server),
        USER,
        PASSWORD,
        &request,
    )
    .await;

    let mail_from_lines = server.mail_from_lines();
    let messages = server.messages();
    server.shutdown().await;

    match result {
        Err(err) => {
            assert!(matches!(err, SmtpError::Unsupported(_)), "实际: {err}");
            assert!(
                err.to_string().contains("8BITMIME"),
                "报错要说清楚缺哪个能力，实际: {err}"
            );
        }
        Ok(outcome) => panic!("服务器不支持 8BITMIME 时应当报错，实际: {outcome:?}"),
    }

    assert!(
        mail_from_lines.is_empty(),
        "发不出去就不该发出 MAIL FROM，实际: {mail_from_lines:?}"
    );
    assert!(messages.is_empty(), "不该收到任何邮件");
}

#[tokio::test]
async fn dot_leading_lines_are_stuffed_on_the_wire() {
    let server = start_server(SmtpServerConfig::new()).await;

    // 正文里有一行**只有一个点**——不转义的话服务器会以为正文结束了。
    let message = b"Subject: dots\r\n\r\n.\r\nlast line\r\n";
    let request = make_request(USER, &["friend@example.com"], message);

    let outcome = submit_message(
        &config_for(&server),
        &trust(&server),
        USER,
        PASSWORD,
        &request,
    )
    .await
    .expect("提交应当成功");
    expect_accepted(&outcome);

    let messages = server.messages();
    let raw = server.raw_messages();
    server.shutdown().await;

    // 服务器去掉 dot-stuffing 之后，正文必须与输入**逐字节相同**。
    assert_eq!(messages[0], message.to_vec(), "正文被截断或被改了");

    // 线上原始字节里，那一行必须是 `..`。
    assert_eq!(
        raw[0], b"Subject: dots\r\n\r\n..\r\nlast line\r\n",
        "线上没有做 dot-stuffing"
    );
    // 结束符是**单独一行**的 `.`，不算正文。
    assert!(!raw[0].ends_with(b".\r\n"), "结束符混进正文里了");
}

#[tokio::test]
async fn every_command_line_ends_with_crlf() {
    let server = start_server(SmtpServerConfig::new()).await;

    let request = make_request(
        USER,
        &["a@example.com", "b@example.com"],
        b"Subject: crlf\r\n\r\nbody\r\n",
    );

    let outcome = submit_message(
        &config_for(&server),
        &trust(&server),
        USER,
        PASSWORD,
        &request,
    )
    .await
    .expect("提交应当成功");
    expect_accepted(&outcome);

    let lines = server.command_lines();
    server.shutdown().await;

    assert!(!lines.is_empty(), "应当记到命令行");
    for line in &lines {
        assert!(line.ends_with("\r\n"), "命令行没有以 CRLF 结尾: {line:?}");
        assert!(
            !line.trim_end().ends_with('\n'),
            "行尾有多余的换行: {line:?}"
        );
    }
}

#[tokio::test]
async fn data_payload_uses_crlf_even_when_the_message_uses_lf() {
    // MIME 层交出来的字节**不保证**是 CRLF（RFC 5322 允许裸 LF），
    // 但线上必须是 CRLF——否则不少服务器会把整封信当成二进制乱码。
    let server = start_server(SmtpServerConfig::new()).await;

    let request = make_request(USER, &["friend@example.com"], b"Subject: lf\n\nbody line\n");

    let outcome = submit_message(
        &config_for(&server),
        &trust(&server),
        USER,
        PASSWORD,
        &request,
    )
    .await
    .expect("提交应当成功");
    expect_accepted(&outcome);

    let messages = server.messages();
    let raw = server.raw_messages();
    server.shutdown().await;

    // 线上正文：每一行都是 CRLF。
    assert_eq!(
        raw[0], b"Subject: lf\r\n\r\nbody line\r\n",
        "线上正文没有统一成 CRLF"
    );
    for (index, byte) in raw[0].iter().enumerate() {
        if *byte == b'\n' {
            assert!(
                index > 0 && raw[0][index - 1] == b'\r',
                "线上出现了裸 LF，位置 {index}"
            );
        }
    }
    // 服务器看到的邮件也是 CRLF 版的（不是原始的裸 LF 版）。
    assert_eq!(messages[0], b"Subject: lf\r\n\r\nbody line\r\n".to_vec());
}

#[tokio::test]
async fn oversized_message_is_rejected_by_the_server() {
    // 服务器宣告 `SIZE 100`，并且**真的**拒收超限的邮件。
    let server = start_server(SmtpServerConfig::new().with_max_message_size(100)).await;

    let big = vec![b'x'; 500];
    let mut message = b"Subject: big\r\n\r\n".to_vec();
    message.extend_from_slice(&big);
    message.extend_from_slice(b"\r\n");

    let request = make_request(USER, &["friend@example.com"], &message);

    let outcome = submit_message(
        &config_for(&server),
        &trust(&server),
        USER,
        PASSWORD,
        &request,
    )
    .await
    .expect("提交应当有结果");

    match &outcome {
        SubmitOutcome::Rejected {
            code,
            message,
            stage,
        } => {
            assert_eq!(*code, 552, "服务器应当回 552，实际: {message:?}");
            assert_eq!(*stage, SubmitStage::Data);
        }
        other => panic!("超限的邮件应当被拒收，实际: {other:?}"),
    }

    let messages = server.messages();
    server.shutdown().await;
    assert!(messages.is_empty(), "被拒收的邮件不该被存下来");
}

#[tokio::test]
async fn quit_is_sent_on_success_and_on_rejection() {
    // 成功时。
    let server = start_server(SmtpServerConfig::new()).await;
    let request = make_request(USER, &["friend@example.com"], b"Subject: x\r\n\r\nbody\r\n");
    let outcome = submit_message(
        &config_for(&server),
        &trust(&server),
        USER,
        PASSWORD,
        &request,
    )
    .await
    .expect("提交应当成功");
    expect_accepted(&outcome);
    let commands = server.commands();
    server.shutdown().await;
    assert!(
        commands.contains(&"QUIT".to_string()),
        "成功也要礼貌收尾，实际命令: {commands:?}"
    );

    // 失败时（收件人被拒）。
    let server =
        start_server(SmtpServerConfig::new().rejecting_recipient("nobody@example.com", 550)).await;
    let request = make_request(USER, &["nobody@example.com"], b"Subject: x\r\n\r\nbody\r\n");
    let outcome = submit_message(
        &config_for(&server),
        &trust(&server),
        USER,
        PASSWORD,
        &request,
    )
    .await
    .expect("提交应当有结果");
    assert!(matches!(outcome, SubmitOutcome::Rejected { .. }));
    let commands = server.commands();
    server.shutdown().await;
    assert!(
        commands.contains(&"QUIT".to_string()),
        "失败也要礼貌收尾，实际命令: {commands:?}"
    );
}

#[tokio::test]
async fn smtputf8_is_not_sent_this_round() {
    // 服务器宣告了 `SMTPUTF8`，但地址全是 ASCII → 依然**不**带这个参数。
    // （地址含非 ASCII 时才带，见 `capability_negotiation.rs`。）
    let server = start_server(SmtpServerConfig::new()).await;

    let request = make_request(USER, &["friend@example.com"], b"Subject: x\r\n\r\nbody\r\n");
    let outcome = submit_message(
        &config_for(&server),
        &trust(&server),
        USER,
        PASSWORD,
        &request,
    )
    .await
    .expect("提交应当成功");
    expect_accepted(&outcome);

    let lines = server.command_lines();
    server.shutdown().await;

    for line in &lines {
        assert!(
            !line.contains("SMTPUTF8"),
            "ASCII 地址不该出现 SMTPUTF8 参数: {line:?}"
        );
    }
}

#[tokio::test]
async fn invalid_envelope_is_an_error_not_a_panic() {
    let server = start_server(SmtpServerConfig::new()).await;

    // 没有收件人。
    let request = make_request(USER, &[], b"Subject: x\r\n\r\nbody\r\n");
    let result = submit_message(
        &config_for(&server),
        &trust(&server),
        USER,
        PASSWORD,
        &request,
    )
    .await;
    server.shutdown().await;

    match result {
        Err(err) => {
            assert!(matches!(err, SmtpError::Invalid(_)), "实际: {err}");
        }
        Ok(outcome) => panic!("没有收件人应当直接报错，实际: {outcome:?}"),
    }
}
