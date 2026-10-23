# Changelog

本项目遵循 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.1/)，版本号遵循 [语义化版本](https://semver.org/lang/zh-CN/)。

维护者：xyyi0719

## [Unreleased]

### Added
- **P0 出关：Command → 服务 → SQLite → React 真实列表**（不再返回硬编码空列表）
  - 启动时解析数据目录（桌面 `%APPDATA%/com.xremail.app`，Android app 内部存储）、
    打开连接池并**应用迁移**；迁移失败则启动失败——**不降级**为空库或旧库
  - 服务层 `apps/shell/src-tauri/src/service/`：Command 只做「参数校验 → 调服务 → 返回 DTO」，
    业务在服务层；服务层**不认识 Tauri**，发事件是 Command 的事（**状态提交后才发**）
  - Command：`list_accounts`（真实查询）、`create_account`、`delete_account`
  - `xremail-api` 新增应用服务 DTO `NewAccountRequest`：入参校验 + 转领域对象（14 个用例）
  - 前端：账户列表 + 新增表单 + 删除；**不引新依赖**（未用 TanStack / Tailwind）
  - 事件当**刷新提示**用：收到 `account://added` 等事件后重新查询，不解析载荷
  - 服务商只作标记，**服务器地址由用户手填**：163/QQ/沃 的参数尚未实测（R01/R02），
    不能按品牌推定
- **IPC 层测试 8 个**（`tauri::test` 的 `mock_builder` + `get_ipc_response`）：
  走真实 IPC 链路，**锁死 camelCase 线格式**（`displayName` / `imapPort` / `server.imapHost`）
  - 这层测试当场抓到一个真 bug：`ServerConfig` 漏了 `rename_all`，
    嵌套对象仍是 snake_case——已修 core
- **服务层测试 10 个**：空库返回空数组、创建后可查回、校验失败不落半条记录、
  更新不换 id、**关掉连接重新打开同一个文件数据仍在**（证明真的落盘）
- **core 的过线结构体统一 camelCase**：`ServerConfig` / `Folder` / `Message` /
  `Address` / `MessageBody` / `Attachment`（`Account` 已有）。
  落库不这套命名——`xremail-store` 有自己的列映射，两边互不影响
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
- **同步引擎的拉取链路**（`xremail-sync::pull`）：服务器 → 差分 → SQLite
  - 固定流程：`EXAMINE`（拿 epoch）→ epoch 判定 → 取远端 UID 集合 → 差分 →
    拉取新增 → 清理服务器已删 → 更新游标
  - 远端 UID 集合用 `UID FETCH 1:* (UID)` 而不是 `UID SEARCH ALL`：
    **FETCH 是 IMAP 的必备命令，SEARCH 是可选扩展**
  - 拉取**只读**（`EXAMINE`），标记回写另走 `SELECT`
  - 报告区分三种情况：`first_sync`（首次）/ `epoch_changed`（旧世代作废）/ `full_fetch`
  - 用 `run_checked` 而不是 async-imap 的 `uid_fetch`——
    后者会**吞掉**服务器的 `NO`，那样会把整个文件夹的缓存当成「服务器已删」清掉
- **`xremail-store` 的邮件 DAO**（`dao/message.rs`）
  - 按 `(account_id, folder_id, uidvalidity, uid)` upsert（四元组才是身份，
    `Message-ID` 仅作线索）
  - `list_message_uids` 只取 UID，**不取正文**——差分只需要知道「有哪些」
- **`xremail-store` 的跨表同步提交**（`dao/sync.rs`）
  - 一轮文件夹同步要同时动三张表，必须**原子**：
    清旧世代 → 写新邮件 → 删服务器已删的 → 更新游标
  - 顺序很重要：先清旧世代再写新的（反过来会把刚写进来的新世代邮件也删掉）
- **`xremail-protocol-imap` 的安全 `examine_checked`**：打开文件夹并返回
  epoch / 邮件数等概况，服务器拒绝时**一定**返回错误
- **假 IMAP 服务器补齐同步链路命令**：`UID FETCH` / `UID SEARCH` / `UID STORE`
  - 假服务器有了**共享可变状态**：`STORE` 改完，后续 `FETCH` / `SEARCH` 能看到
  - `sequence` 模块：sequence set 解析（`1:10,15,20:*`）——**在服务端**，
    因为客户端（async-imap）是把 sequence set 当**原始字符串**发的
  - `search` 模块：搜索键解析与求值（标记类 / `UID` / 头字段 / 正文 / `NOT` / `OR` / 日期）
  - `fetch` 模块：取件项解析（`UID` / `FLAGS` / `RFC822.SIZE` / `BODY[HEADER.FIELDS (...)]` / `ENVELOPE`）
    与 `STORE` 参数解析（`+FLAGS` / `-FLAGS` / `.SILENT` / `UNCHANGEDSINCE`）
  - 完整实现 IMAP 状态机：未选中邮箱时 `FETCH` / `SEARCH` / `STORE` 都会被拒；
    `EXAMINE` 打开的只读邮箱拒绝 `STORE`
  - `BODY[...]`（不带 PEEK）会**置上 `\Seen`**，`BODY.PEEK[...]` 不会
  - 条件写 `UNCHANGEDSINCE (<modseq>)`：**全有或全无**——
    只要有一封的 modseq 对不上，整条命令都不改任何东西
- **`xremail-protocol-imap` 的安全命令层** `run_checked`（**重要**）
  - 自己跑命令、自己读响应、自己判定 `OK` / `NO` / `BAD`
  - 服务器回 `NO` / `BAD` 时**一定**返回错误
  - 错误信息里带上 response code（`[READ-ONLY]` / `[MODIFIED]` / …）
- **`xremail-testkit` crate（T7 前置，假 IMAP 服务器）**：协议层的测试**不再依赖真实邮箱**
  - 真邮箱只能复现**一种**情况；假服务器把各家服务商的怪癖做成**开关**，在 CI 里全部跑一遍
  - `ServerConfig`：能力集、`require_id`（163 式的“先发 ID”）、
    `reject_examine`、初始文件夹与邮件、UIDVALIDITY
  - `Fault`：登录后断连 / 对某命令不响应（客户端超时）/ 拒绝某命令 / 认证失败
  - 自签证书：真实客户端不可能信任它，测试要**显式**把那一张 CA 交给客户端
    （`TlsTrust::Roots`）；生产路径始终用系统根，**没有**跳过校验的开关
  - **认证前拒绝一切命令**（连 `LOGOUT` 都回 `BAD`）——复现 163
  - 文件夹名按 **modified UTF-7** 编码上线（`&XfJT0ZAB-` = 已发送）
- **本机测试账号库**（`%USERPROFILE%\.xremail\accounts.txt`，**仓库之外**）
  - 分段格式：`[163]` / `[qq]` / `[wo]` / …，每段 `user` + `password`
  - 配套模板 `accounts.template`（含各服务商的**开启路径与申请入口**）
  - 探测工具新增 `--list`（只列已配置的 provider，账号脱敏）与 `--provider`
  - 授权码用 `SecretValue` 包着：丢弃时清零，`Debug` 不打印内容
- **`xremail-protocol-imap` crate（P1，IMAP 协议层）**：rustls + async-imap，只读探测工具
  - `connect_and_login`：隐式 TLS（强制校验系统根，**无**明文/跳过校验开关）→ LOGIN → **ID（RFC 2971）**
  - `utf7`：IMAP 邮箱名的 **modified UTF-7** 编解码（RFC 3501 §5.1.3）——不解码就认不出「已发送」
  - `probe`：**严格只读**探测（只发 LOGIN / CAPABILITY / ID / LIST / EXAMINE / STATUS / IDLE+DONE；
    **绝不** STORE / MOVE / COPY / DELETE / EXPUNGE / APPEND，也不发 SMTP 邮件）
  - 探测输出**脱敏**：不打印授权码、完整邮箱地址、邮件内容；文件夹按特殊用途属性判角色
  - 凭据从**本机文件**读（仓库之外），不进命令行参数、不进日志
- **P1 实测：163 邮箱（2026-10-22）**——两个足以改变设计的发现
  - **必须先发 `ID`（RFC 2971）自报身份，否则进不去邮箱**：
    不发 `ID` 时 `EXAMINE` / `SELECT` 一律被回 `NO … Unsafe Login`。
    `ID NIL` 不算；内容任意（含空串）都算。已实现为「服务器宣告 `ID` 能力时登录后立即发送」
  - **没有 IDLE、没有 CONDSTORE** ⇒ 163 只能用**轮询**，且 flag 写入要走
    「读快照 + 重放本地意图 + 回读确认」路径（ARCHITECTURE §6）
  - 另：**无 UIDNEXT**（`STATUS` 接受该字段但不给值）⇒ 同步游标不能基于 UIDNEXT；
    **无 MOVE** ⇒ 移动走 `COPY` + `\Deleted` + `EXPUNGE`；
    `EXAMINE` **不报 UNSEEN**（要用 `STATUS`）；请求 `HIGHESTMODSEQ` 会得到
    `BAD Parse command error`（不要无条件请求）
  - 邮箱名在线路上是 modified UTF-7（`&XfJT0ZAB-` = 已发送）
  - UID 空间很大（实测 1792658701~1792658704，**不是** 1..N）
  - 认证后能力集、文件夹特殊用途属性、永久标记（不含 `\Recent`）等已记入
    [PROTOCOLS.md](docs/PROTOCOLS.md)；IDLE/CONDSTORE 矩阵的 163 行已从「待实测」改为实测结论
  - Rust 用例总数 194 → **212**（新增 18 个：utf7 编解码 11 + 凭据读取/脱敏 7）
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
- **`1:*` 被展开成上亿个元素**（假服务器内存爆掉，8 GiB 分配失败）：
  sequence set 的 `resolve` 是**展开**语义，而 `1:*` 在大 UID 空间下
  （163 的 UID 就是 17 亿量级）会覆盖上亿个 UID。
  现在假服务器**不展开 set**，而是拿每封邮件去问 `contains`；
  `resolve` 保留但注明「不要在大 UID 空间下用」
- **async-imap 0.12 会吞掉服务器回的 `NO` / `BAD`**（**严重**）：
  `uid_fetch` / `uid_store` / `uid_search` 在服务器拒绝时**返回 `Ok`**，
  只是结果为空。后果是同步引擎会把「服务器拒绝了」当成「写成功了」，
  本地状态与远端悄悄分叉。
  根因：库的响应解析用 `take_while(filter(...))` 读响应，而 `filter` 遇到
  **自己那个 tag** 的 `Done` 响应时返回 `false`——那个携带 `NO` 的响应
  于是**从未被解析成错误**就被丢弃。
  本项目的对策：新增 `run_checked` 安全命令层，并在测试里把这个行为钉住
- **上一批的“用例数统计脚本”有 bug**：用 `awk -F'[ ;]'` 取 `$6`，
  而 `passed; 1 failed` 之间的 `"; "` 会切出一个**空字段**，
  于是失败数**恒为 0**。因此上一批报的“六关全绿”里其实有 1 个用例是失败的
  （`xremail-testkit` 的 `first_atom_skips_quoted`）。
  对策：改用 `target/gates.sh`，它**看退出码**而不是看打印出来的数字
- **`xremail-testkit` 的 `first_atom` 是死 API**：没有任何调用点，
  且它的测试断言与文档描述相反（文档说“第一个未加引号的词”，
  测试却期望带引号时也返回它）——该测试从一开始就是失败的。直接删除
- **`MessageFixture` 的头字段替换会把头与正文之间的空行弄丢**，
  导致 `header()` 只能看到第一个头字段
- **登录失败被一律当成“密码错了”**：网络抖动导致的连接断开也是 `Auth` 错误，
  会让用户反复检查一个根本没问题的密码，也让重试策略失效。
  现在按 async-imap 的错误变体分类：`NO` → 认证失败（需用户处理）、
  `IO` / `ConnectionLost` → 传输故障（**可重试**）、`BAD` / 解析失败 → 协议层问题
- **`modified UTF-7` 编码器把 `&` 编成了 `&ACY-`**：RFC 3501 规定 `&` 必须写成 `&-`
  （移位段为空），而不是当作普通字符进移位段。用例当场捕获，已修
- **Windows 上 `tauri::test` 的测试二进制无法启动**（`STATUS_ENTRYPOINT_NOT_FOUND`）
  - 原因：测试二进制静态导入 `comctl32.dll!TaskDialogIndirect`（只有 v6 SxS 才有），
    而 `tauri-build` 只给主程序嵌入 v6 manifest
  - 解法：`apps/shell/src-tauri/build.rs` 把 comctl32 改为延迟加载
    （`/DELAYLOAD:comctl32.dll` + 链接 `delayimp.lib`）
- **Windows 上强杀 Tauri 应用会留下 `msedgewebview2.exe` 残留**，
  导致下次启动报 `WebView2 0x800700AA 请求的资源在使用中`。
  该错误发生在 Tauri 建窗口阶段（**早于用户的 setup 钩子**），
  看起来像「setup 没跑」，容易误判——详见 [ENVIRONMENT §1.9](docs/ENVIRONMENT.md)
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
