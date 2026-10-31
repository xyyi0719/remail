//! 附件的数量 / 大小 / 类型上限。
//!
//! ## 为什么要有这一层
//!
//! 写信面板原先没有任何附件约束：整个文件被读成 `number[]` 塞进 IPC 参数，
//! 几十 MB 的文件会把参数体积撑爆（JSON 化的字节数组体积是二进制的 3–4 倍），
//! 而且收件方服务端多半会拒收。
//!
//! 本模块是**纯函数**：只吃元数据（文件名、大小），不碰文件系统、不碰数据库。
//! 调用方在把文件读进内存**之前**先问一句 `check_attachment_limits`，
//! 就能把超限的文件挡在 IPC 之前。
//!
//! ## 为什么用扩展名黑名单，而不是 MIME 类型白名单
//!
//! MIME 类型是**发送方自己填的**：前端完全可以给一个 `.exe` 声明
//! `text/plain`，所以拿 MIME 做拦截等于没拦（「类型是发送方自己填的，能骗人」）。
//! 扩展名虽然同样能被改，但它对应的是**收件人双击时真正会执行的那个文件**，
//! 因此它至少能挡住「手滑把可执行文件发出去了」这类事故。
//!
//! **这不是安全边界**：把 `a.exe` 改名成 `a.dat` 就能绕过。真正的防线在
//! 收件方的服务端和客户端扫描（以及用户的判断）。这里的定位是
//! 「便宜的一道护栏」，不是「不可绕过的策略」。
//!
//! ## 取舍说明
//!
//! 黑名单把 `.py` / `.rb` / `.php` 这类**脚本**也列了进去——它们确实可执行，
//! 但程序员之间互相发个脚本是很常见的需求。这属于**偏保守**的选择：
//! 觉得太严时，直接从 [`BLOCKED_ATTACHMENT_EXTENSIONS`] 里删掉对应条目即可
//! （那是公开常量，改一行的事），不需要改逻辑。

use std::fmt;

use serde::{Deserialize, Serialize};

/// 单个附件的大小上限：**10 MiB**。
///
/// 取值理由：
/// * 常见的 PDF、扫描件、几张照片在 1–5 MiB，10 MiB 覆盖绝大多数日常附件；
/// * 再大就基本是视频/压缩包，这类内容走邮件附件不合适（应走网盘链接）；
/// * 10 MiB 原始字节经 MIME base64 编码后约 13.3 MiB，给整封邮件留足了
///   25 MiB 的常见服务商上限里的余量。
pub const MAX_ATTACHMENT_BYTES: u64 = 10 * 1024 * 1024;

/// 单封邮件的附件个数上限：**5 个**。
///
/// 取值理由：收件人的阅读体验在 5 个左右就到临界点（列表会横向溢出），
/// 而且部分邮件网关对**部件数量**（不只大小）也有限制。
pub const MAX_ATTACHMENTS_PER_MESSAGE: usize = 5;

/// 单封邮件的附件**总大小**上限：**18 MiB**（原始字节，未编码）。
///
/// 取值理由（这里是本模块唯一「不跟随直觉」的数字）：
/// MIME 附件走 base64 编码，体积会膨胀约 **4/3**。
/// `18 MiB × 4/3 = 24 MiB`，加上 MIME 头部仍在常见服务商的 **25 MiB** 整封上限之内；
/// 而 20 MiB 的原始字节编码后是 26.7 MiB，**会超过** 25 MiB 的线上限。
/// 所以这里取 18 而不是直觉上的 20——留出的 1 MiB 余量给 MIME 头和折行。
pub const MAX_TOTAL_ATTACHMENT_BYTES: u64 = 18 * 1024 * 1024;

/// 会被拦下的扩展名（不含点，统一小写）。
///
/// 覆盖：Windows 可执行/加载模块/安装包、Windows 与跨平台脚本、macOS 应用包与
/// 脚本、Linux 可执行包与安装包、快捷方式/启动器。
///
/// 取舍：
/// * `.dll` / `.ocx` / `.so` / `.class` 是**可加载的代码**而不是直接可执行，
///   但同样危险，一并拦下；
/// * `.txt` / `.md` / `.csv` 等纯文本**没有**拦——它们是最常见的合法附件，
///   拦掉会误伤；
/// * 名单里没有的、我们没预料到的类型（比如新出现的运行时）**不会**被拦，
///   这是黑名单的固有代价：漏一个就少挡一个。
pub const BLOCKED_ATTACHMENT_EXTENSIONS: &[&str] = &[
    // ── Windows 可执行 / 加载模块 / 安装包 ──────────────────────────────
    "exe",
    "com",
    "scr",
    "pif",
    "cpl",
    "dll",
    "ocx",
    "drv",
    "sys",
    "msi",
    "msp",
    "msix",
    "appx",
    "appxbundle",
    "application",
    "msc",
    "gadget",
    "job",
    "inf", // ── Windows 脚本 / 批处理 ─────────────────────────────────────────
    "bat",
    "cmd",
    "ps1",
    "psm1",
    "psd1",
    "vbs",
    "vbe",
    "js",
    "jse",
    "ws",
    "wsf",
    "wsh",
    "hta",
    "reg", // ── 跨平台脚本 ─────────────────────────────────────────────────
    "sh",
    "bash",
    "zsh",
    "csh",
    "ksh",
    "fish",
    "py",
    "pyc",
    "pyo",
    "rb",
    "pl",
    "php",
    "lua",
    "tcl", // ── macOS ──────────────────────────────────────────────────────
    "app",
    "dmg",
    "pkg",
    "mpkg",
    "scpt",
    "command",
    "workflow",
    "osx", // ── Linux / 跨平台可执行 ────────────────────────────────────
    "deb",
    "rpm",
    "apk",
    "appimage",
    "snap",
    "run",
    "desktop",
    "so",
    "class",
    "jar",
    "war",
    "ear",
    // ── 启动器 / 快捷方式 ───────────────────────────────────────────────
    "lnk",
    "url",
    "website",
];

/// 一个待发送的附件的元数据。
///
/// 只含**校验所需**的信息：文件名、大小，以及可选的 MIME 类型。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentInput {
    /// 附件文件名。可以是完整路径（会被截到最后一段）。
    pub filename: String,
    /// 附件大小（**原始字节**，未编码）。
    pub size: u64,
    /// MIME 类型。
    ///
    /// **仅供调用方参考，本模块不用它做拦截**——类型是发送方自己填的，能骗人。
    /// 保留这个字段是为了让调用方（如后端命令）不必自己另建一个结构体。
    pub mime: Option<String>,
}

impl AttachmentInput {
    /// 只带文件名和大小构造。
    pub fn new(filename: impl Into<String>, size: u64) -> Self {
        Self {
            filename: filename.into(),
            size,
            mime: None,
        }
    }

    /// 附带 MIME 类型构造。
    pub fn with_mime(mut self, mime: impl Into<String>) -> Self {
        self.mime = Some(mime.into());
        self
    }
}

/// 附件超限的具体原因。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AttachmentProblem {
    /// 附件个数超限。
    TooManyAttachments {
        /// 实际个数。
        count: usize,
        /// 个数上限。
        limit: usize,
    },
    /// 单个文件超限。
    FileTooLarge {
        /// 出问题的文件名。
        filename: String,
        /// 实际大小（字节）。
        actual: u64,
        /// 单文件上限（字节）。
        limit: u64,
    },
    /// 一封邮件的附件总大小超限。
    TotalTooLarge {
        /// 实际总大小（字节）。
        actual: u64,
        /// 总大小上限（字节）。
        limit: u64,
    },
    /// 命中扩展名黑名单。
    ForbiddenExtension {
        /// 出问题的文件名。
        filename: String,
        /// 被拦下的扩展名（不含点，小写）。
        extension: String,
    },
}

impl fmt::Display for AttachmentProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooManyAttachments { count, limit } => {
                write!(f, "附件数量过多：共 {count} 个，上限为每封 {limit} 个")
            }
            Self::FileTooLarge {
                filename,
                actual,
                limit,
            } => write!(
                f,
                "附件「{filename}」大小 {}（{actual} 字节），超过单个附件上限 {}（{limit} 字节）",
                format_bytes(*actual),
                format_bytes(*limit)
            ),
            Self::TotalTooLarge { actual, limit } => write!(
                f,
                "附件总大小 {}（{actual} 字节），超过每封邮件 {}（{limit} 字节）的总上限",
                format_bytes(*actual),
                format_bytes(*limit)
            ),
            Self::ForbiddenExtension {
                filename,
                extension,
            } => write!(
                f,
                "附件「{filename}」的扩展名 .{extension} 属于可执行文件或脚本，已阻止发送"
            ),
        }
    }
}

impl std::error::Error for AttachmentProblem {}

/// 把字节数格式化成人类可读的形式（不足 1 MiB 时用 KiB）。
fn format_bytes(bytes: u64) -> String {
    const MIB: f64 = 1024.0 * 1024.0;
    const KIB: f64 = 1024.0;
    let bytes_f = bytes as f64;
    if bytes_f >= MIB {
        format!("{:.2} MiB", bytes_f / MIB)
    } else if bytes_f >= KIB {
        format!("{:.2} KiB", bytes_f / KIB)
    } else {
        format!("{bytes} B")
    }
}

/// 检查一组附件是否满足全部上限。
///
/// 返回 `Err` 时给出**第一个**命中的问题；需要一次看全所有问题时用
/// [`check_attachment_limits_all`]。
///
/// 检查顺序（决定「先报哪个」）：
/// 1. 逐个文件：扩展名黑名单 → 单文件大小；
/// 2. 附件个数；
/// 3. 附件总大小。
///
/// 单文件问题排在最前，是因为它**指名道姓**地说出是哪个文件超了什么，
/// 比「共 6 个附件」这种笼统信息更容易照着改。
///
/// 没有附件时返回 `Ok(())`——「不带附件」是合法的。
///
/// ```
/// use xremail_core::attachment::{check_attachment_limits, AttachmentInput};
///
/// assert!(check_attachment_limits(&[]).is_ok());
///
/// let inputs = vec![AttachmentInput::new("a.pdf", 1024)];
/// assert!(check_attachment_limits(&inputs).is_ok());
///
/// let big = vec![AttachmentInput::new("big.bin", 100 * 1024 * 1024)];
/// assert!(check_attachment_limits(&big).is_err());
/// ```
pub fn check_attachment_limits(inputs: &[AttachmentInput]) -> Result<(), AttachmentProblem> {
    match check_attachment_limits_all(inputs).into_iter().next() {
        Some(problem) => Err(problem),
        None => Ok(()),
    }
}

/// 与 [`check_attachment_limits`] 同样的检查，但返回**全部**问题。
///
/// 写信面板需要这个：一次把所有超限的附件标出来，比来回试错强。
/// 顺序与 [`check_attachment_limits`] 一致（单文件问题在前）。
pub fn check_attachment_limits_all(inputs: &[AttachmentInput]) -> Vec<AttachmentProblem> {
    let mut problems = Vec::new();

    // 1) 逐个文件检查
    let mut total: u64 = 0;
    for input in inputs {
        total = total.saturating_add(input.size);

        if let Some(extension) = blocked_extension(&input.filename) {
            problems.push(AttachmentProblem::ForbiddenExtension {
                filename: input.filename.clone(),
                extension,
            });
            continue;
        }

        if input.size > MAX_ATTACHMENT_BYTES {
            problems.push(AttachmentProblem::FileTooLarge {
                filename: input.filename.clone(),
                actual: input.size,
                limit: MAX_ATTACHMENT_BYTES,
            });
        }
    }

    // 2) 个数
    if inputs.len() > MAX_ATTACHMENTS_PER_MESSAGE {
        problems.push(AttachmentProblem::TooManyAttachments {
            count: inputs.len(),
            limit: MAX_ATTACHMENTS_PER_MESSAGE,
        });
    }

    // 3) 总大小
    if total > MAX_TOTAL_ATTACHMENT_BYTES {
        problems.push(AttachmentProblem::TotalTooLarge {
            actual: total,
            limit: MAX_TOTAL_ATTACHMENT_BYTES,
        });
    }

    problems
}

/// 取出文件名里**所有**可能的扩展名并检查黑名单。
///
/// 为什么检查「所有」而不只是最后一个：
/// `发票.pdf.exe` 这种双扩展名在 Windows 上双击执行的是 `.exe` 那一段；
/// 只看最后一段（`.pdf`）就会漏掉它。这里按点号分段逐个比对，
/// 代价只是文件名里多几个点的正常文件（如 `报告.v2.pdf`）会被多比对几次。
fn blocked_extension(filename: &str) -> Option<String> {
    candidate_extensions(filename)
        .into_iter()
        .find(|extension| BLOCKED_ATTACHMENT_EXTENSIONS.contains(&extension.as_str()))
}

/// 从文件名里切出候选扩展名（统一小写、去空段）。
fn candidate_extensions(filename: &str) -> Vec<String> {
    // 只看最后一段路径：`C:\Users\me\a.exe` 与 `a.exe` 应当等价。
    let base = filename
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(filename)
        .trim();
    // Windows 会悄悄去掉文件名结尾的点和空格（`a.exe.` 实际是 `a.exe`）。
    let base = base.trim_end_matches(['.', ' ']);

    match base.split_once('.') {
        // 没有点 → 没有扩展名
        None => Vec::new(),
        // 首个点之后的每一段都是候选扩展名。
        Some((_, rest)) => rest
            .split('.')
            .map(|part| part.trim().to_ascii_lowercase())
            .filter(|part| !part.is_empty())
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(name: &str, size: u64) -> AttachmentInput {
        AttachmentInput::new(name, size)
    }

    #[test]
    fn empty_attachment_list_is_ok() {
        assert!(check_attachment_limits(&[]).is_ok());
    }

    #[test]
    fn ordinary_attachments_pass() {
        let inputs = vec![
            input("报告.pdf", 2 * 1024 * 1024),
            input("photo.png", 512 * 1024),
            input("数据.csv", 100 * 1024),
        ];
        assert!(check_attachment_limits(&inputs).is_ok());
        assert!(check_attachment_limits_all(&inputs).is_empty());
    }

    #[test]
    fn rejects_single_file_over_limit() {
        let inputs = vec![input("big.bin", MAX_ATTACHMENT_BYTES + 1)];
        let err = check_attachment_limits(&inputs).unwrap_err();
        match &err {
            AttachmentProblem::FileTooLarge {
                filename,
                actual,
                limit,
            } => {
                assert_eq!(filename, "big.bin");
                assert_eq!(*actual, MAX_ATTACHMENT_BYTES + 1);
                assert_eq!(*limit, MAX_ATTACHMENT_BYTES);
            }
            other => panic!("期望单文件超限，实际是 {other:?}"),
        }
        // 错误信息里要能看出实际值和上限值。
        let text = err.to_string();
        assert!(text.contains("big.bin"), "{text}");
        assert!(text.contains("10.00 MiB"), "{text}");
    }

    #[test]
    fn file_exactly_at_limit_is_allowed() {
        let inputs = vec![input("exactly.bin", MAX_ATTACHMENT_BYTES)];
        assert!(check_attachment_limits(&inputs).is_ok());
    }

    #[test]
    fn rejects_too_many_attachments() {
        let inputs: Vec<_> = (0..MAX_ATTACHMENTS_PER_MESSAGE + 1)
            .map(|i| input(&format!("f{i}.txt"), 1024))
            .collect();
        let err = check_attachment_limits(&inputs).unwrap_err();
        match &err {
            AttachmentProblem::TooManyAttachments { count, limit } => {
                assert_eq!(*count, MAX_ATTACHMENTS_PER_MESSAGE + 1);
                assert_eq!(*limit, MAX_ATTACHMENTS_PER_MESSAGE);
            }
            other => panic!("期望个数超限，实际是 {other:?}"),
        }
    }

    #[test]
    fn rejects_total_over_limit() {
        // 单个都合法，但加起来超了总上限。
        let inputs: Vec<_> = (0..5)
            .map(|i| input(&format!("f{i}.bin"), 5 * 1024 * 1024))
            .collect();
        let total: u64 = inputs.iter().map(|i| i.size).sum();
        assert!(total > MAX_TOTAL_ATTACHMENT_BYTES);
        let err = check_attachment_limits(&inputs).unwrap_err();
        match &err {
            AttachmentProblem::TotalTooLarge { actual, limit } => {
                assert_eq!(*actual, total);
                assert_eq!(*limit, MAX_TOTAL_ATTACHMENT_BYTES);
            }
            other => panic!("期望总量超限，实际是 {other:?}"),
        }
    }

    #[test]
    fn rejects_blocked_extensions() {
        for name in [
            "setup.exe",
            "SETUP.EXE",
            "run.BAT",
            "payload.msi",
            "a.exe",
            "script.sh",
            "macro.vbs",
            "web.js",
            "app.dmg",
            "installer.pkg",
            "shell.hta",
            "system.ps1",
        ] {
            let inputs = vec![input(name, 1024)];
            let err = check_attachment_limits(&inputs).unwrap_err();
            assert!(
                matches!(err, AttachmentProblem::ForbiddenExtension { .. }),
                "「{name}」应当被拦下，实际是 {err:?}"
            );
            assert!(err.to_string().contains(name), "{err}");
        }
    }

    #[test]
    fn rejects_blocked_extension_hidden_behind_double_extension() {
        // 最后一个扩展名是无害的，但中间那段才是双击会执行的东西。
        let inputs = vec![input("发票.pdf.exe", 1024)];
        let err = check_attachment_limits(&inputs).unwrap_err();
        match &err {
            AttachmentProblem::ForbiddenExtension { extension, .. } => {
                assert_eq!(extension, "exe")
            }
            other => panic!("应当被拦下，实际是 {other:?}"),
        }
    }

    #[test]
    fn blocks_regardless_of_path_and_trailing_dots() {
        for name in [
            r"C:\Users\me\setup.exe",
            "docs/setup.exe",
            "setup.exe.",
            "setup.exe ",
        ] {
            let inputs = vec![input(name, 1024)];
            assert!(
                check_attachment_limits(&inputs).is_err(),
                "「{name}」应当被拦下"
            );
        }
    }

    #[test]
    fn mime_type_cannot_smuggle_a_blocked_extension() {
        // MIME 是发送方自己填的，声明成 text/plain 也不该放行。
        let inputs = vec![AttachmentInput::new("setup.exe", 1024).with_mime("text/plain")];
        assert!(check_attachment_limits(&inputs).is_err());
    }

    #[test]
    fn allows_common_harmless_extensions() {
        for name in [
            "报告.pdf",
            "notes.txt",
            "archive.tar.gz",
            "photo.JPEG",
            "data.csv",
            "no-extension",
            ".gitignore",
            "第1章.笔记",
        ] {
            let inputs = vec![input(name, 1024)];
            assert!(
                check_attachment_limits(&inputs).is_ok(),
                "「{name}」不该被拦下"
            );
        }
    }

    #[test]
    fn reports_all_problems_at_once() {
        let inputs = vec![
            input("ok.txt", 1024),
            input("setup.exe", 1024),
            input("huge.bin", MAX_ATTACHMENT_BYTES + 1),
        ];
        let problems = check_attachment_limits_all(&inputs);
        assert_eq!(problems.len(), 2, "{problems:?}");
        assert!(matches!(
            problems[0],
            AttachmentProblem::ForbiddenExtension { .. }
        ));
        assert!(matches!(
            problems[1],
            AttachmentProblem::FileTooLarge { .. }
        ));
    }

    #[test]
    fn error_messages_are_chinese_and_specific() {
        let inputs = vec![input("setup.exe", 1024)];
        let text = check_attachment_limits(&inputs).unwrap_err().to_string();
        assert!(text.contains("setup.exe"), "{text}");
        assert!(text.contains(".exe"), "{text}");
        assert!(text.contains("可执行文件"), "{text}");
    }
}
