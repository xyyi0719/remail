# Changelog

本项目遵循 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.1/)，版本号遵循 [语义化版本](https://semver.org/lang/zh-CN/)。

维护者：xyyi0719

## [Unreleased]

### Added
- **`xremail-sync` crate（P0 / T2，同步纯函数）**：UID 差分、epoch 切换计算、bit 意图合并
  - `UidSet` / `diff_uids`：算出「要拉取的」与「服务器已删的」，升序稳定输出
    - `UidDiff::retain_local`：本地草稿与未发送内容有**独立归属**，不能因为服务器上没有就被清掉
    - `UidDiff::chunk_added`：摘要分批拉取，避免一次拉太多
  - `plan_epoch_transition`：epoch 未变 → 增量；首次同步 → 全量；epoch 变了 → **重建快照**，
    并列出绑定旧 epoch 的待提交操作（**不得**套用到新 UID 上）
    - `EpochTransition::requires_full_fetch()` 区分「全量拉取」与「重建快照」两个概念
  - `FlagIntent` / `apply_intents` / `plan_flag_update`：服务器快照与本地待提交意图**分开**保存
    - 同一 bit 的多个意图按 `seq` 排序：后者覆盖前者（`+Seen` 与 `-Seen` **确实会冲突**）
    - 只发**净变化**的位，净变化为空时**不发命令**（不整组替换 FLAGS）
    - 绑定旧 epoch 的意图进 `FlagPlan::stale`，不参与计算
    - `MAX_REPLAN_ATTEMPTS = 3`：MODIFIED 后重新读取并重算的次数上限（重试有界）
  - 本阶段**只含纯函数**：无网络、无 IO，可直接在 CI 跑；账户调度与 IMAP 会话属 P2
- **T2 用例 57 个**（单元 47 + 场景 10）：首次同步 / 增量同步 / 服务器删除保留草稿 /
  epoch 变化重建并隔离旧意图 / 跨 epoch 作差是错的 / 本地视图合并 / 点了又取消不发命令 /
  MODIFIED 后按新快照重算 / 旧意图绝不发给服务器 / 只发变化的位
  - **已用 7 个变异验证用例有效性**：不排序 seq（1 个失败）、空 delta 也发命令（6）、
    不隔离旧 epoch 操作（2）、added/removed 反转（6）、忽略 remove 位（5）、
    忽略 epoch 校验（1）、保留策略失效（2）——全部被捕获
  - Rust 用例总数：99 → **163**（含 core 新增 7 个 flags 集合代数用例）
- **`xremail-core` 的 `MessageFlags` 补齐集合代数**：`union` / `intersection` / `difference` /
  `is_empty` / `bits`——bit 意图合并需要「快照 → 最终」的净变化计算
- **`xremail-store` crate（P0 存储地基）**：SQLite 持久化层，含迁移、连接池与账户/文件夹 DAO
  - 迁移 `crates/xremail-store/migrations/0001_init.sql`：10 张表——`account` / `folder` / `message` / `message_body` / `attachment` / `mail_operation` / `send_queue` / `local_secret` / `label` / `rule`
  - **身份不变量落到 schema 层**：`message` 表的 `UNIQUE (account_id, folder_id, uidvalidity, uid)`；RFC 5322 `Message-ID` **不建唯一约束**（仅作线索）
  - 复合外键 `message(account_id, folder_id) → folder(account_id, id)`：保证邮件的账户与文件夹一致
  - 删除策略逐表声明：缓存类（文件夹/邮件/正文/附件）随上级级联；`mail_operation` / `send_queue` 对邮件只做**逻辑引用**，不随邮件删除（epoch 切换后需隔离核对）
  - `CHECK` 约束覆盖 IMAP 语义：`uid > 0`（UID 0 非法）、`uidvalidity > 0`、端口范围、`poll_interval_secs > 0`
  - `mail_operation` 与 `send_queue` **分表**，不混为一个 outbox（两者的重试语义不同）
  - 连接池：WAL、`busy_timeout`、**显式 `PRAGMA foreign_keys = ON`**（SQLite 默认不启用外键）、有界连接池
  - 错误分类：约束冲突（唯一键/外键/CHECK）映射为 `AppError::Invalid`（**不可重试**），其余 sqlx 错误映射为 `AppError::Storage`
- **T1 测试用例 30 个**（`xremail-store` 单元 6 + 集成 24）：迁移建表、迁移幂等、外键确实生效、身份唯一约束、不同 epoch 相同 UID 共存、Message-ID 非唯一、复合外键、CHECK 约束、逐表删除策略、账户/文件夹往返、upsert 语义、未知枚举不静默回退、事务整体回滚、文件库重开可读
  - Rust 用例总数：4（core）→ **34**（P0 阶段中间态，现为 99）
- **sqlx 离线查询数据 `.sqlx/`**：SQL 查询在**编译期**对 schema 校验（列名、列数、类型、可空性），CI 无需起数据库
  - 已用反例验证：写错列名 / 列数不匹配 / 类型不匹配 / 可空性不匹配，四类均在编译期报错
- **CI 新增「Verify sqlx offline data」步骤**：`cargo sqlx prepare --workspace --check` 校验 `.sqlx/` 与迁移/查询同步，过期即失败（已验证：故意改查询后返回 exit 1）
- **`docs/ENVIRONMENT.md` §2.6「Ubuntu 26.04 开发机准备清单」**：发行版基线对照表（glibc 2.43 / GCC 15.2 / LLVM 21.1 / Python 3.14 / Rust 1.93.1 / OpenJDK 25）、已核实的系统依赖包名、Rust/Node 装法、Android 组件、与 CI 的联动（`ubuntu-latest` 将于 2026-11 切到 26.04）、glibc 兼容下限对 .deb 的影响、WSL2 预演方式
- **本机补装工具**：`cargo-audit` 0.22.2、`cargo-deny` 0.20.2、`git-filter-repo`（pip --user）
- **Android 工具链就绪**（2026-10-21）：JDK 17.0.20.1、Android SDK（cmdline-tools 13.0 / platform-tools 37.0.1 / platforms android-36 与 37.0 / build-tools 36.0.0 与 37.0.0）、NDK 27.2.12479018、cargo-ndk 4.1.2、4 个 Rust Android target；均装在用户目录（本机非管理员）
  - 用户级环境变量：`JAVA_HOME`、`ANDROID_HOME`、`ANDROID_SDK_ROOT`、`NDK_HOME`
  - `tauri android init` 成功，`cargo build --target aarch64-linux-android` 产物经 `file` 确认为 ARM aarch64 ELF
  - **未完成**：APK 打包卡在 Windows 符号链接权限（非管理员且未重登录，开发者模式已写入注册表待生效）——见 [ENVIRONMENT §1.8](docs/ENVIRONMENT.md)
- 环境特性与平台差异文档 `docs/ENVIRONMENT.md`：本机 shell / 代理 / DNS / 工具链限制，各平台 WebView 与 API 差异，写脚本与 CI 前的检查清单
- 测试用例扩写计划 `docs/TESTING_STRATEGY.md §5`：T1–T10 优先级、每项的「算完成」标准、用例数作为进度指标
- CI 新增「Report test case count」步骤，输出 Rust 用例数到 job summary
- 详细技术规划文档 `docs/PLAN.md`（v1.2，历史参考），含协议核查、UI 选型对比、数据模型、同步引擎设计
- 架构基线 `docs/ARCHITECTURE.md`：身份 epoch、IMAP-aware 编排、IDLE 单一所有者、bit 级操作意图、不确定结果、FTS 投影、L2 CAS 契约
- 各服务商协议参数速查 `docs/PROTOCOLS.md`（含实测记录模板）
- 安全模型与威胁分析 `docs/SECURITY.md`
- 验收门槛路线图 `docs/ROADMAP.md`
- 贡献指南 `docs/CONTRIBUTING.md`
- 测试策略 `docs/TESTING_STRATEGY.md`：测试分层、故障注入、协议/安全/平台验收场景
- AI 开发工作规范 `docs/AI_DEVELOPMENT_GUIDE.md`：不变规则、职责边界、可复用任务模板、常见错误
- 风险登记册 `docs/ISSUES_AND_RISKS.md`：MECE 分类、纠错记录、关闭条件
- 架构决策记录：ADR-0001（UI 技术栈）、ADR-0002（同步深度）、ADR-0003（Gmail OAuth，暂缓）
- Tauri 2 + React 19 + TypeScript 工程脚手架（`apps/shell`）
- `xremail-core` crate：领域模型（账户、文件夹、邮件、Flags、地址、事件）
- `xremail-core` 错误分类单测 4 个
- `xremail-core` 的 `Folder` 新增 `uidvalidity` 字段（该文件夹的 **epoch**）：它本来就是领域概念（ARCHITECTURE §4.3 要求按 epoch 切换游标），此前模型里没有，DAO 无法往返

### Changed
- **R13 决策：release profile 的 panic 策略由 `abort` 改为 `unwind`**
  - 理由：P0 开始引入 tokio 异步任务，`abort` 下任一 panic 会终止整个进程，后台同步任务无法用 `catch_unwind` 兜底；现在改成本最低，体积代价很小
  - 保留的配套要求：后台 task 边界需 `catch_unwind`（待 `xremail-sync` 落地时实现）
  - 依据：[ARCHITECTURE §9](docs/ARCHITECTURE.md)、[ISSUES_AND_RISKS R13](docs/ISSUES_AND_RISKS.md)
- **MSRV 由 1.88 上提至 1.94**：sqlx 0.9.0 声明的 MSRV 是 1.94.0，继续声明 1.88 会成为假声明
- **领域枚举改为显式 `serde` 名称**（`ProviderKind` / `AuthKind` / `TlsMode` / `SmtpAuth` / `SyncMode`）
  - 原因：这些字符串会作为 SQLite 列值落库。若依赖 `rename_all` 自动推导，Rust 变体改名会让旧行读不出来（静默丢数据）
  - 附带修正：`ProviderKind::Netease163` 原先自动推导为 `netease163`（数字不产生分隔符），现为显式的 `netease_163`
- **项目改名：ReMail → Xremail**
  - 展示名 `Xremail`；crate / npm 包名 `xremail-core` / `xremail-shell`；Tauri identifier `com.xremail.app`
  - 原因：`remail` 在 crates.io（Rust 邮件服务器 v0.0.1）与 npm（React 邮件组件框架 v2.1.0）均已被占用；`xremail` 三个注册表均可用，GitHub 无同名仓库
  - 目录 `crates/remail-core/` → `crates/xremail-core/`；根 Cargo.toml 的 workspace 依赖键同步改名
  - 命名规范见 [CONTRIBUTING.md](docs/CONTRIBUTING.md)
- `ProviderKind` 的沃邮箱变体重命名为 `Wo`（显示名「沃邮箱」），移除服务商名称中的厂商字样；序列化值同步变更
- 文档与提交信息中不再出现厂商名称，仅保留产品名与域名
- 架构语义重写：本地 SQLite 明确为读模型；服务器快照与本地操作意图分离；不承诺 exactly-once
- 冲突仲裁不再用本地时间戳猜测远端操作顺序；有 CONDSTORE 用 UNCHANGEDSINCE，否则快照 + 意图重放
- 删除策略：无 UIDPLUS 时不使用普通 EXPUNGE，避免清除其他客户端的删除标记
- 安全：撤回凭据静默降级与沙箱弹窗逃逸建议；默认禁网；明确 AEAD 不能单独防回滚重放
- 路线图改为按验收门槛推进，撤回未经测量的耗时系数与固定交付日期
- 文档作者/负责人署名为 xyyi0719

### Fixed
- **桌面端专属插件在 Android 目标上编译失败**：`tauri-plugin-single-instance` 的 `init` 与 `tauri-plugin-autostart` 的 `init` / `MacosLauncher` 只在桌面端存在
  - `apps/shell/src-tauri/src/lib.rs`：两个插件用 `#[cfg(desktop)]` 隔离，`use tauri::Manager` 同步加门（Android 侧未用到，否则 clippy `-D warnings` 会报错）
  - 影响面：仅 Android 目标的编译；桌面端行为不变
- **`docs/ENVIRONMENT.md` 更新过时的 Android 结论**
  - §1.1 工具链表：OpenJDK 从「仅 1.8.0_502」改为 17.0.20.1，并补 Android SDK / NDK / cargo-ndk / cargo-audit / cargo-deny / git-filter-repo 行
  - 新增 §1.8「Android 工具链」：安装位置、用户级环境变量、5 条踩坑（Adoptium 跳转 GitHub 被劫持、包名实为 `platforms;android-37.0`、`sdkmanager` 需 JDK 17、Git Bash 下 `reg add` 的 `/v` 被 MSYS 改写、桌面端插件在 Android 无 API），以及当前唯一卡点（符号链接权限需重登录）
  - §2.1 平台支持表：Android 从「⛔ 本机不可构建」改为「🟡 Rust 侧可交叉编译，APK 打包待权限」
- **`docs/ENVIRONMENT.md` §1.5 不再写死本机手动设置的代理地址**
  - 删除文档里的具体代理 `host:port`（该值会随网络环境变化，且属于内网信息）；改为运行时从注册表读 `ProxyServer` / `ProxyEnable` / `ProxyOverride`
  - 实测发现文档里的旧值已与注册表当前值**不一致**，证实写死必然失效
  - 明确仓库级 `http.proxy` / `https.proxy` 属本机手动配置：落在 `.git/config`，`.git/` 不被跟踪、不随仓库分发，换机器需重设
  - `--resolve` 用的 GitHub 公网 IP 不再写死，改为经代理走 DoH 实时取（GitHub 公网 IP 会轮换）
  - 补充 Git Bash 下 `reg query ... /v` 会被 MSYS 当路径改写的坑（用 `//v` 或 `MSYS_NO_PATHCONV=1`）
- **新增“连 GitHub 前先探明”规则**（`docs/ENVIRONMENT.md` §1.5.1、`docs/AI_DEVELOPMENT_GUIDE.md` §3/§5.0、`docs/CONTRIBUTING.md`）
  - 每次访问 GitHub（推送 / 拉取 / 调 API / 开 PR）**先探明连通性，再执行操作**
  - 探针：`git ls-remote origin`（推送通道，SSH over 443）、`gh auth status` / `gh api rate_limit`（API 通道）、`curl -x <当前代理>`（HTTPS 通道）
  - 附探针结果 → 结论 → 下一步的判读表；**探不通不推**，避免反复推送产生半截状态或让 CI 空跑
  - 实测本机到 GitHub 的连接**间歇性失败**（`git ls-remote` 5 次中 3 次成功、`gh api` 5/5 成功、`curl` 过代理 5 次中 1 次成功）；**单次探针失败先重试**再判定断网
  - 补记三种通道各自的失败特征：SSH over 443 为 `Connection reset by <ip> port 443`（TCP 能连、SSH 会话被 reset）、`gh api` 为 `net/http: TLS handshake timeout`、`curl` 为 `schannel: failed to receive handshake`
  - 实测**换 GitHub IP 无效**：9 个候选里只有本地 DNS 解析到的那个接受 `ssh.github.com` 的 SSH 服务（其余 `Connection closed by <ip> port 443`），且 TCP 443 对所有候选都通 → 问题在会话层，只能重试
  - 探针脚本重试次数定为 `TRIES=6`（刻意收敛的上限：探针单次成功率约 25%，整轮全失败概率约 18%；6 次仍不通就停下报给维护者，不继续刷），并把 **push 本身也纳入同一重试循环**（探针过了 push 仍可能被 reset）
  - 补记两个判成功相关的坑：给 `git push` 接管道（如 `| tail`）会让退出码变成管道末端命令的，把失败读成成功；推送后用 `ls-remote` 核验本身也会间歇性失败，核验失败不等于没推上去，应以 `git push` 退出码为准并比对 SHA
- **改名遗漏**：`ReMail → Xremail` 后仍残留旧 crate 名
  - `apps/shell/src/App.tsx`：占位页文案显示的 `remail-store` 改为 `xremail-store`（用户可见）
  - `apps/shell/src-tauri/src/lib.rs`：注释中的 `remail-sync` 改为 `xremail-sync`
  - `apps/shell/src-tauri/src/cmd.rs`、`apps/shell/src-tauri/src/state.rs`：注释中的 `remail-store` 改为 `xremail-store`
  - `ProviderKind` 枚举**变体名与序列化值未改**（`netease163` / `netease126` / `netease_enterprise` / `tencent` / `wo` / `google` / `exchange` / `generic` 保持不变）
- **`ProviderKind` 文档注释移除厂商名**，改用产品名与域名，与 `Wo` 变体重命名时确立的规范一致
  - `NetEase enterprise mail` → `网易企业邮 mailboxes (@qiye.163.com)`
  - `QQ Mail (Tencent)` → `QQ mailboxes (@qq.com)`
  - `Wo Mail` → `沃邮箱 mailboxes (@wo.cn)`
  - `Gmail / Google Workspace` → `Gmail / Workspace mailboxes`
  - `Microsoft Exchange / Outlook 365` → `Exchange / Outlook 365 mailboxes`
  - `AuthKind::OAuth2` 的 `Google/Microsoft` → `Gmail / Exchange`
  - 同步修正 163 / 126 的注释为产品名 + 域名写法
- `AppError` 的 `ctx()` 不再把错误统一折叠成 `Other`，保留 `is_retryable()` / `requires_user_action()` 判定
- 代码注释中的文档相对路径断链（`crates/xremail-core/src/*.rs`、`apps/shell/src-tauri/src/*.rs`）
- 代码注释中过期的 `ARCHITECTURE §N` 章节号引用
- workspace `resolver` 由 `2` 改为 `3`（edition 2024 隐含 MSRV 感知解析），Cargo.lock 无变化

### Notes
- 关键技术选型已锁定：Tauri 2 + React/TS、先做 L1（IMAP 原生）同步并预留 L2 抽象
- 后续持续扩写测试用例，按 TESTING_STRATEGY §5 的 T1–T10 顺序在 P0–P3 分批推进
- Linux 编译已由 CI 验证，运行未验证；列为可选 P5-L，需求明确后再投入
- Gmail 暂缓，不再是 P1 关键路径；恢复接入前需重新核实授权与发布要求
- 弃用同步版 `imap` crate
- 尚未实现：SQLite 持久化、协议适配、收发链路、自动化业务测试
