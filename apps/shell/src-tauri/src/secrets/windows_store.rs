//! Windows 凭据管理器适配。
//!
//! 目标平台：Windows Credential Manager（`advapi32!` 的
//! `CredWriteW` / `CredReadW` / `CredDeleteW`）。
//!
//! 契约（`SecretStore` 的语义与错误约定）见 [`super`](super) 的模块文档。
//!
//! ## 条目长什么样
//!
//! * **类型**：`CRED_TYPE_GENERIC`（通用凭据）。通用凭据是唯一一种
//!   内容由我们自己定义的类型；域密码、证书那些由系统管理，格式不由我们定。
//! * **条目名（TargetName）**：**原样**用传进来的 `secret_ref`
//!   （如 `xremail:account:<uuid>`）。`get` / `set` / `delete`
//!   对同一个 `secret_ref` 必须用**完全一样**的条目名——差一个字符就是另一条凭据。
//!   Windows 对通用凭据的条目名没有格式要求（惯例上以 `\` 结尾，
//!   但那是"某台机器上的某个用户"那种命名风格，不是硬性约束）。
//! * **内容（CredentialBlob）**：**原始字节**，不是宽字符串，读回来也是原始字节。
//!   机密本身已经是字节（文本机密由调用方按 UTF-8 构造），这里**原样写入**：
//!   再编一次 UTF-8 会让读回来的字节对不上原来的值。
//!   读的时候也不做"字符串强转"——包成 [`SecretValue`] 交给上层判断。
//! * **持久化**：`CRED_PERSIST_LOCAL_MACHINE`。条目对当前 Windows 用户长期可见，
//!   注销、重启后仍在（`CRED_PERSIST_SESSION` 会在注销后消失，对"记住授权码"没用）。
//! * **UserName**：通用凭据要求它非空，填一个占位 `"xremail"`（不含任何机密）。
//!
//! ## 失败语义
//!
//! * 条目**不存在**不是错误：`get` 返回 `Ok(None)`、`delete` 返回 `Ok(false)`。
//! * 其余错误一律**明确失败**，并按语义映射到 [`SecretStoreError`] 的变体。
//! * 错误信息里**不含**机密内容——只说"哪一步被系统拒绝了、错误码是多少"。

use async_trait::async_trait;
use windows::Win32::Foundation::{
    ERROR_ACCESS_DENIED, ERROR_INVALID_PARAMETER, ERROR_NOT_FOUND, FILETIME,
};
use windows::Win32::Security::Credentials::{
    CRED_FLAGS, CRED_MAX_CREDENTIAL_BLOB_SIZE, CRED_PERSIST_LOCAL_MACHINE, CRED_TYPE_GENERIC,
    CREDENTIALW, CredDeleteW, CredFree, CredReadW, CredWriteW,
};
use windows::core::{Error as WinError, PCWSTR, PWSTR};

use xremail_api::SecretStoreError;
use xremail_api::secret::{SecretStore, SecretValue};

/// 通用凭据的 `UserName` 占位值。
///
/// 通用凭据要求 `UserName` 非空指针；这里只放应用名，不含任何机密。
const USER_NAME: &str = "xremail";

/// Windows 凭据管理器适配。
///
/// 无状态：凭据管理器是系统级的，全进程共用一份。
#[derive(Debug, Clone, Copy, Default)]
pub struct WindowsSecretStore;

impl WindowsSecretStore {
    /// 构造。
    pub const fn new() -> Self {
        Self
    }
}

#[async_trait]
impl SecretStore for WindowsSecretStore {
    async fn get(&self, secret_ref: &str) -> Result<Option<SecretValue>, SecretStoreError> {
        let target = target_name(secret_ref)?;
        let mut raw: *mut CREDENTIALW = std::ptr::null_mut();
        // 读的时候类型必须与写入时一致，否则读到的是另一类凭据。
        let read = unsafe { CredReadW(PCWSTR(target.as_ptr()), CRED_TYPE_GENERIC, None, &mut raw) };
        match read {
            Ok(()) => {
                // 这块内存是 Windows 分配的，用完必须归还，否则会漏。
                let credential = unsafe { &*raw };
                let size = credential.CredentialBlobSize as usize;
                let bytes = if size == 0 || credential.CredentialBlob.is_null() {
                    Vec::new()
                } else {
                    // blob 是原始字节；长度由 CredentialBlobSize 给出（不含结尾的 NUL）。
                    unsafe { std::slice::from_raw_parts(credential.CredentialBlob, size) }.to_vec()
                };
                unsafe { CredFree(raw as *const core::ffi::c_void) };
                Ok(Some(SecretValue::new(bytes)))
            }
            // "本来就没有"不是错误。
            Err(err) if win32_code(&err) == Some(ERROR_NOT_FOUND.0) => Ok(None),
            Err(err) => Err(map_error(&err, "读取")),
        }
    }

    async fn set(&self, secret_ref: &str, value: &SecretValue) -> Result<(), SecretStoreError> {
        let bytes = value.as_bytes();
        if bytes.len() > CRED_MAX_CREDENTIAL_BLOB_SIZE as usize {
            // 截断等于把凭据写坏：宁可写不进去，也不能存一个读回来是错的值。
            return Err(SecretStoreError::Invalid(format!(
                "凭据内容 {} 字节超过 Windows 凭据管理器的上限 {CRED_MAX_CREDENTIAL_BLOB_SIZE} 字节",
                bytes.len()
            )));
        }

        let mut target = target_name(secret_ref)?;
        let mut user = wide(USER_NAME);
        // blob 的指针在调用期间必须一直有效：Windows 只读它，不拷贝它。
        let mut blob = bytes.to_vec();
        let credential = CREDENTIALW {
            Flags: CRED_FLAGS(0),
            Type: CRED_TYPE_GENERIC,
            TargetName: PWSTR::from_raw(target.as_mut_ptr()),
            Comment: PWSTR::null(),
            LastWritten: FILETIME {
                dwLowDateTime: 0,
                dwHighDateTime: 0,
            },
            CredentialBlobSize: blob.len() as u32,
            CredentialBlob: if blob.is_empty() {
                std::ptr::null_mut()
            } else {
                blob.as_mut_ptr()
            },
            Persist: CRED_PERSIST_LOCAL_MACHINE,
            AttributeCount: 0,
            Attributes: std::ptr::null_mut(),
            TargetAlias: PWSTR::null(),
            UserName: PWSTR::from_raw(user.as_mut_ptr()),
        };
        // 同名条目会被**直接覆盖**——凭据管理器本身没有"已存在"的语义。
        unsafe { CredWriteW(&credential, 0) }.map_err(|err| map_error(&err, "写入"))?;
        Ok(())
    }

    async fn delete(&self, secret_ref: &str) -> Result<bool, SecretStoreError> {
        let target = target_name(secret_ref)?;
        match unsafe { CredDeleteW(PCWSTR(target.as_ptr()), CRED_TYPE_GENERIC, None) } {
            Ok(()) => Ok(true),
            // 本来就没有：报告"没删掉"，而不是报错。
            Err(err) if win32_code(&err) == Some(ERROR_NOT_FOUND.0) => Ok(false),
            Err(err) => Err(map_error(&err, "删除")),
        }
    }
}

/// 构造条目名（TargetName）的 UTF-16 缓冲区。
///
/// Win32 的字符串一律以 NUL 结尾，所以要自己补结尾的 0。
fn target_name(secret_ref: &str) -> Result<Vec<u16>, SecretStoreError> {
    if secret_ref.is_empty() {
        return Err(SecretStoreError::Invalid("凭据引用不能为空".into()));
    }
    Ok(wide(secret_ref))
}

/// 把 `&str` 转成带结尾 NUL 的 UTF-16 缓冲区。
fn wide(s: &str) -> Vec<u16> {
    let mut buf: Vec<u16> = s.encode_utf16().collect();
    buf.push(0);
    buf
}

/// 从 `windows::core::Error` 里还原 Win32 错误码。
///
/// windows-rs 把 Win32 错误码包装成 `HRESULT_FROM_WIN32(x)`
/// （`0x8007_0000 | x`），低 16 位才是原始错误码。
/// 不是这一族 HRESULT 时（例如 COM 错误）返回 `None`——
/// 别把不相干的码硬说成 Win32 码。
fn win32_code(err: &WinError) -> Option<u32> {
    let raw = err.code().0 as u32;
    if raw & 0xFFFF_0000 == 0x8007_0000 {
        Some(raw & 0xFFFF)
    } else {
        None
    }
}

/// 把 Win32 错误映射成 [`SecretStoreError`]。
///
/// `action` 是中文动作名（"写入" / "读取" / "删除"），拼进信息里，
/// 让用户知道是**哪一步**被系统拒绝了。
/// 错误信息里只有系统给的说明与错误码，**没有机密内容**。
fn map_error(err: &WinError, action: &str) -> SecretStoreError {
    let code = win32_code(err);
    let suffix = match code {
        Some(code) => format!("（错误码 {code}）"),
        None => format!("（HRESULT 0x{:08X}）", err.code().0 as u32),
    };
    // 系统自带的说明（随系统语言变化）；拿不到就只留错误码。
    let detail = err.message();
    let detail = if detail.trim().is_empty() {
        suffix
    } else {
        format!("{detail}{suffix}")
    };
    match code {
        // 权限不足：用户看得见、也改得动（换个权限再试），归"被拒绝"。
        Some(code) if code == ERROR_ACCESS_DENIED.0 => {
            SecretStoreError::Denied(format!("Windows 凭据管理器拒绝{action}：{detail}"))
        }
        // 参数非法：条目本身不合法（过长、名字不合法等），重试没用。
        Some(code) if code == ERROR_INVALID_PARAMETER.0 => SecretStoreError::Invalid(format!(
            "Windows 凭据管理器认为{action}的条目不合法：{detail}"
        )),
        // 其余（含 ERROR_NO_SUCH_LOGON_SESSION：没有登录会话，钥匙串不可用）
        // 都归"不可用"：这台机器上现在拿不到凭据管理器。
        _ => SecretStoreError::Unavailable(format!("Windows 凭据管理器{action}失败：{detail}")),
    }
}
