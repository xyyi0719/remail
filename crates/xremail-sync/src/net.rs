//! 同步引擎共用的网络辅助。

use std::time::Duration;

use xremail_protocol_imap::{CommandOutcome, ProtocolError, Session, run_checked};

/// 单条 IMAP 命令的超时。
///
/// **必须有**：async-imap 自己**没有**命令超时，
/// 服务器不回应时客户端会一直挂着，同步任务就卡死在那儿。
pub const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);

/// 跑一条命令，带超时。
///
/// 超时按**传输层故障**处理——命令**可能**已经发出去了，
/// 所以调用方要按「不确定服务器执行了没」来对待（见 ARCHITECTURE §5）。
pub async fn run_checked_with_timeout(
    session: &mut Session,
    command: &str,
) -> Result<CommandOutcome, ProtocolError> {
    match tokio::time::timeout(COMMAND_TIMEOUT, run_checked(session, command)).await {
        Ok(result) => result,
        Err(_) => Err(ProtocolError::Timeout(format!(
            "命令超时（{COMMAND_TIMEOUT:?}）: {command}"
        ))),
    }
}
