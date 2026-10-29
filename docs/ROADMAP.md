# 路线图

> 执行范围入口。个人研究项目，AI 编码，允许一年以上探索；不使用未经测量的 AI 耗时系数或固定交付日期。
> 本轮只更新文档，未实现下表中的业务能力。已确认：国内邮箱优先、Gmail 暂缓、先 L1 且 P0 预留 L2。
> 维护者：xyyi0719

## 1. MECE 工作分区

按主要交付物划分；安全/测试是各工作区的验收条件，不再额外重复计入工期。

| 区域 | 唯一主责 | 不负责 | 对应阶段 |
|---|---|---|---|
| A 外部接入 | 配置、认证、协议能力证据 | 缓存/业务 UI | P1 |
| B 数据与同步 | 身份、存储、增量、操作恢复 | HTML 展示/SMTP 提交 | P0/P2 |
| C 邮件交互 | 阅读、检索、编写、发送 | 系统生命周期 | P2/P3 |
| D 平台宿主 | Windows/Android 生命周期、通知、安全存储适配 | 邮箱业务规则 | P4 |
| E 可选扩展 | 标签/规则等本地能力、L2 服务、额外服务商 | 核心收发门槛 | P5/P6 |
| F 工程交付 | 构建、测试工具、诊断、个人安装包 | 业务实现 | P0/P7 |

**测试用例扩写** 横跨 A–C，按 [TESTING_STRATEGY §5](TESTING_STRATEGY.md) 的 T1–T10 顺序在各阶段推进，不单列为一条串行工作线。

阶段编号用于保持原文档关联，不表示严格串行：Android PoC、威胁验证与协议探测可提前。

## 2. 当前仓库事实

- [x] Git 仓库、Tauri/React 脚手架、Cargo workspace、xremail-core。
- [x] 文档与 CI workflow 文件；测试策略/AI 工作规范已定义。
- [x] `xremail-core` 错误分类单测 4 个（2026-10-20）。
- [x] `xremail-store`：迁移 + 账户/文件夹 DAO + 身份约束（2026-10-22）；T1 用例 30 个。
- [x] `xremail-crypto`：argon2id + ChaCha20-Poly1305 版本化密文封装（2026-10-22）；T3 用例 40 个。
- [x] `xremail-api`：SecretStore / VaultStore / SyncBackend 契约 + 内存 mock（2026-10-22）；T4 用例 25 个。
- [x] `xremail-sync`：UID 差分 / epoch 切换 / bit 意图合并三个纯函数模块（2026-10-22）；T2 用例 57 个。
  - 2026-10-23 升级为**带 IO 的同步引擎**：拉取链路（差分 → epoch 切换 → 落库）
    与回写链路（意图 → `UID STORE` → **回读确认**）
- [x] `xremail-protocol-imap`：IMAP 协议层 + 只读 probe + **安全命令层** `run_checked`（2026-10-22/23）。
  - **三家实测完成**：163 / QQ / 沃（各自的协议事实见 [PROTOCOLS.md](PROTOCOLS.md)）
  - `run_checked` 自己判定 `OK`/`NO`/`BAD`——async-imap 会**吞掉**服务器的拒绝（R15）
- [x] `xremail-message`：MIME 结构、charset 解码、RFC 5322 头（2026-10-23）；T5 用例 40 个。
  - **GB2312 / GBK / GB18030 三种都要支持**；30 封真实邮件解码零错误
- [x] `xremail-testkit`：假 IMAP 服务器 + 故障注入（2026-10-22）；T7 前置已就位。
  - 能复现 163 的「必须先发 `ID`」「认证前拒绝一切命令」等怪癖
- [x] `xremail-diagnostics`：诊断工具（字符集抽样，只输出编码名与计数）
- [x] `xremail-api`：新增应用服务 DTO `NewAccountRequest`（校验 + 转领域对象，14 个用例）。
- [x] shell：服务层 + 真实持久化链路 + React 账户列表（2026-10-22）；服务测试 10 + IPC 测试 8。
  - **实测账本**：Rust 用例 4 → **524**（新增 520 个）
- [x] **同步引擎接进应用**（2026-10-26）：UI 里能看到**真实邮箱**里的邮件
  - 服务层 `MessageService` / `SyncService`；Command 8 个；前端：账户 → 同步 → 文件夹 → 邮件 → 正文
  - 验收：三家真实账号各跑通一次（163 / QQ / 沃，6 个文件夹全部同步，正文解码零错误）
- [x] 凭据录入：授权码**能在应用里填**了，也**能落盘**（Windows 凭据管理器）；
  关掉应用不用重填（R19：Windows 已处理，其它平台仍需重填）。落盘的平台适配见 P4。
- [x] **平台钥匙串**（2026-10-26）：授权码存进 **Windows 凭据管理器**，数据库里只留不透明引用
  - `secrets/windows_store.rs`（`CredWriteW`/`CredReadW`/`CredDeleteW`），条目名 `xremail:account:<account_id>`
  - 非 Windows 是**桩**：不落盘、不静默降级；平台不支持时**如实报错**
  - 用例 6 个，**真跑** Windows 凭据管理器（存 → 读 → 改 → 删），不靠 mock
- [x] **写操作真正回写服务器**（2026-10-26）：标记已读 / 星标从「只改本地」变成 `UID STORE` 回写
  - 意图先落库（`queued`）→ 提交 → **回读确认** → 六态回报前端（`applied` / `state`）
  - 三家真实邮箱实测：`UID STORE` 全 OK、**回读确认生效**、重复发同一标记也回 OK（见 `docs/PROTOCOLS.md`）
  - epoch 变了 → 意图作废（`failed` / `epoch_changed`），不会把旧 UID 套到另一封信上
- [ ] 调度器（AccountActor）：什么时候同步、同步哪些文件夹、重连退避。
- [ ] 收发链路（P3）：SMTP **提交路径**一个字没写（只读探测已做完，见 P3 小节）。
- [ ] 真实邮箱 **CI 验收**：现在的真实邮箱证据全部来自本机手工运行。

> **仓库已推送**（2026-10-26，代码批次 `975ffda`、文档批次 `f6f7406`），CI **四个 job 全绿**
> （含 Windows 凭据管理器的真实往返用例）。R17 已处理。

## 3. 分阶段门槛

### P0：可测试的最小地基

- [x] xremail-store：迁移、账户/文件夹 DAO、身份约束（`crates/xremail-store`，2026-10-22）
  - 迁移 `0001_init.sql`（10 张表）；身份不变量 `UNIQUE (account_id, folder_id, uidvalidity, uid)` 落到 schema
  - DAO：账户 upsert/list/get/delete，文件夹 upsert/list/get/delete
- [ ] xremail-store：跨表事务封装 → **已完成**（`crates/xremail-store/src/dao/sync.rs` 的 `commit_folder_sync`）
- [x] xremail-api：SecretStore、VaultStore、SyncBackend 最小契约与可运行 mock；CAS 冲突测试（`crates/xremail-api`，2026-10-22）
  - `SyncBackend` 只提供原子的 `compare_and_swap`，刻意不提供「先查后写」组合（TOCTOU）
  - 双设备 / 三方 CAS 竞争：恰好一个赢家；另有用例证明 API 形状本身防住 TOCTOU
- [x] L2 schema 预留：`local_secret` 只存版本化密文、`label` / `rule` 预留稳定 ID 与 schema version（迁移已就位）
- [x] 最小密码封装测试（`xremail-crypto`：argon2id + ChaCha20-Poly1305，40 个用例）
  - 口令错误与密文篡改返回**同一个**错误（分开就成了 oracle）
  - 头与 nonce 作为 AAD：元数据被改也会被发现
  - `rekey()` 提升 KDF 成本，旧封装仍能解开；`to_bytes`/`parse` 自包含字节格式供 L2 搬运
- [x] Command → 服务 → SQLite → React 真实列表，不以硬编码空列表验收（2026-10-22）
  - `list_accounts` 改为真实查询；新增 `create_account` / `delete_account`
  - 启动时解析数据目录（桌面 `%APPDATA%/com.xremail.app`，Android app 内部存储）、开库、**应用迁移**；
    迁移失败则启动失败，不降级为空库
  - 服务层 `apps/shell/src-tauri/src/service/`：校验 → 落库；**不认识 Tauri**，
    发事件是 Command 的事（状态提交后才发）
  - 前端：账户列表 + 新增表单 + 删除；不引新依赖（不用 TanStack/Tailwind）
  - 事件作为**刷新提示**使用：收到 `account://added` 等事件后重新查询，不解析载荷
  - 服务商只作标记，**服务器地址由用户手填**——163/QQ/沃 的参数尚未实测（R01/R02），
    不能按品牌推定
  - IPC 层测试 8 个：锁死 camelCase 线格式、参数校验、增删的完整链路
- [x] 新库 / 迁移幂等重跑 / 事务整体回滚测试（**升级路径**已补：第二个迁移 `0002_operation_seq.sql` 已就位）
- [x] 跨表事务封装（`crates/xremail-store/src/dao/sync.rs`：`commit_folder_sync` 一个事务内完成
  清旧世代 → 写新邮件 → 删服务器已删的 → 更新游标）
- [x] 实际执行 fmt/clippy/test/typecheck/build（2026-10-26 本地全绿；
  **CI 四个 job 全绿**（Rust ubuntu / Rust windows / Frontend / Docs sanity），
  仓库已推送，见 ISSUES_AND_RISKS R17）
- [x] **测试用例扩写 T1–T5**：T1（迁移与事务）、T2（同步纯函数）、T3（密文封装）、
  T4（L2 契约）、T5（MIME / charset）均已完成；Rust 用例 4 → **387**。
  见 [TESTING_STRATEGY §5](TESTING_STRATEGY.md)。

**出口**：真实持久化链路与 mock 契约均有测试，明确哪些模块尚未创建。
*当前状态：T1–T5 全部完成，真实账户列表已打通（Rust 用例 4 → 387）。**P0 已出关。*
剩余：无（P0 清单已全部勾选；后续工作归 P1/P2）。*

### P1：国内邮箱只读接入

- [x] **163：只读 probe 实测**（2026-10-22，`crates/xremail-protocol-imap`）
  - 预认证：TLSv1.3、证书由公开 CA（DigiCert）签发、**系统根即可校验通过**
  - 认证前**拒绝一切 IMAP 命令**（连 `LOGOUT` 都回 `BAD command not support`）
  - **必须先发 `ID`（RFC 2971）**，否则 `EXAMINE` / `SELECT` 被回 `NO … Unsafe Login`
  - **无 IDLE / 无 CONDSTORE / 无 MOVE / 无 UIDNEXT** ⇒ 只能轮询 + 「快照+意图重放」
  - 邮箱名是 **modified UTF-7**，需解码（已实现 `xremail-protocol-imap::utf7`）
  - 详细记录见 [PROTOCOLS.md 163 章](PROTOCOLS.md)
- [x] **沃邮箱：只读 probe 实测**（2026-10-23）
  - `imap.wo.cn:993`，TLSv1.3，**授权码登录成功**
  - 服务器自称 `Coremail System IMap Server Ready(wo.cn[...])` ⇒ **与 163 同源的 Coremail 体系**
    （先前文档推测的 Coxremail 已被推翻）
  - **无 IDLE / 无 MOVE / 无 NAMESPACE / 无 CONDSTORE**；**有 UIDPLUS**；
    LIST **带** RFC 6154 角色属性；文件夹名是英文（`Sent Items` / `Drafts` / `Trash` / `Junk E-mail`）
  - **不需要先发 `ID`**（与 163 相反）⇒ `ID` 是 163 独有的要求
- [x] **QQ：只读 probe 实测**（2026-10-23）
  - `imap.qq.com:993`，TLSv1.3，授权码登录成功
  - **三家里唯一支持 IDLE**（服务器接受 `+ idling`），也**有 MOVE**、有 NAMESPACE
  - **无 CONDSTORE / 无 QRESYNC** ⇒ 同样只能走「快照+意图重放」
  - **LIST 不给 RFC 6154 角色属性**（只给 `\NoSelect` / `\HasChildren`）⇒
    文件夹角色**必须**按名字兼底（已实现并有单测）
  - **UIDVALIDITY 是大数**（1792993698）而 UIDNEXT 很小（105）⇒ epoch 切换是常态路径
- [x] 凭据适配（`%USERPROFILE%\.xremail\accounts.txt`，**仓库之外**；支持 `domain =`；QQ 只写 QQ 号时自动补 `@qq.com`）
- [x] MIME / 中文字符集：实测三家 charset 并实现 `crates/xremail-message`
  - **GB2312 / GBK / GB18030 三种都要支持**（QQ 用 gb2312 *和* gb18030，沃用 gbk）
  - **只看顶层 `Content-Type` 会看错**——三家顶层几乎都是 multipart，正文 charset 在子部分里
  - 30 封真实邮件解析，**解码零错误**
- [x] 按需正文拉取（`BODY.PEEK[]` → `message_body` 表；正文不随同步全量拉）
- [x] **测试用例扩写 T5**（MIME / charset 解析，含 GBK/GB18030 样本）——`xremail-message` 40 个用例
- [x] 假 IMAP 服务器（T7 前置）：先有假服务器，再上真服务器
  - [x] **已建** `crates/xremail-testkit`（2026-10-22）：可配置的假 IMAP 服务器
    （能力集、要求先发 ID、拒绝 EXAMINE、认证前拒绝一切命令、故障注入）+ 自签证书
  - [x] 协议层端到端测试 10 个（假服务器 × 真客户端）
  - [x] 补齐同步链路命令：UID FETCH / UID SEARCH / UID STORE（2026-10-23）
  - [ ] 补齐：COPY / MOVE（垃圾箱移动）、APPEND（发信）——P3 需要

**出口**：三种真实账户各有实测记录；没有账号或配置时保留阻塞，不宣称完成。
*当前状态（2026-10-26）：**三家均已只读实测**（各自的协议事实记在 [PROTOCOLS.md](PROTOCOLS.md)），
只读链路已在 crate 层打通并**接进应用**（UI 里能看到真实邮件）。
**P1 出关**——凭据落盘也已补上（2026-10-26，Windows 凭据管理器），
**P1 的出口条件已达成**（三种真实账户各有实测记录 + 只读链路接进应用 + 凭据可从应用录入并落盘）。*

### P2：可靠同步与安全阅读

- [x] UIDVALIDITY 事务切换、删除差分。
  - epoch 切换在**一个事务**内完成（清旧世代 → 写新邮件 → 删服务器已删的 → 更新游标）
  - 绑定旧 epoch 的待提交操作**隔离**（标记 `failed`），绝不套用到新 UID 上
  - 用例：`epoch_change_triggers_a_rebuild`、`operations_bound_to_the_old_epoch_are_quarantined`
- [ ] 断线恢复（崩溃后重启对账）。
- [ ] IDLE 状态切换与轮询兜底；其他文件夹调度；**调度器本身尚未存在**。
  - 已具备条件：三家能力已实测（QQ 有 IDLE，163/沃 无），
    能力判定逻辑在 `xremail-protocol-imap::extension_support`
- [x] flag bit 级意图、**回读确认**与 `uncertain` 状态。
  - 「读快照 → 重放意图 → **回读确认**」（ARCHITECTURE §6）
  - 三种失败分清：服务器回 `NO` → `retryable`；`NO [MODIFIED]` → `retryable`（有界重算）；
    连接断了 → **`uncertain`**（不知道服务器执行了没，**不能盲目重试**）
  - 只发**变化的位**，不整组替换 `FLAGS`
  - 用例：11 个（含「服务器回 OK 却不真改 → 回读发现」）
- [ ] 移动 / 删除的保守策略（P3 一并做，依赖假服务器的 COPY / MOVE）
- [x] 按需正文（`BODY.PEEK[]` → `message_body` 表）；头随拉取一起落库。
- [ ] 附件下载、离线缓存状态。
- [ ] **HTML 默认禁网且不可访问宿主**（R06）。
  - 当前状态是安全的：`xremail-message` 只产出**未经清洗的原始 HTML**，没有任何渲染路径
- [ ] FTS 投影、重建与中文短词策略。
- [ ] **测试用例扩写 T6–T8**（HTML 清洗攻击样本、假 IMAP 同步链路、故障注入）。
  - T7 的**假服务器已就位**（含故障注入），同步链路的端到端测试已覆盖拉取与回写
  - T6（HTML 攻击样本）与 T8（故障注入矩阵）仍缺

**出口**：两客户端并发、崩溃恢复、恶意邮件与检索一致性测试通过；不承诺任意网络下固定收敛秒数。
*当前状态（2026-10-23）：同步引擎的**拉取**与**回写**两条链路已在 crate 层完成并有端到端测试；
缺调度器、断线恢复、移动/删除、HTML 安全门与 FTS。*

### P3：发送与草稿

- [ ] **先实测三家的 SMTP 参数**（R18）：服务器地址、`STARTTLS` 语义、`8BITMIME`、
  授权码认证、发信后的 `APPEND` 行为。
  - 预认证阶段已测过 163 的 SMTP 问候与 `EHLO` 能力（无 SIZE / 无 PIPELINUX，有 8BITMIME 与 STARTTLS），
    但**没有发过一封信**
  - [x] **只读探测阶段已实测**（2026-10-28，工具 \`xremail-smtp-probe\`，用本项目自己的 Rust 客户端）：
    - **163** \`smtp.163.com:465\`：TLS 1.3，证书校验通过，\`AUTH LOGIN\` 认证成功（\`235\`）；
      有 \`8BITMIME\`、**无** \`SMTPUTF8\`。
    - **QQ** \`smtp.qq.com:465\`：TLS 1.2，证书校验通过，\`AUTH LOGIN\` 认证成功（\`235\`）；
      有 \`8BITMIME\` / \`SMTPUTF8\` / \`PIPELINING\`，\`SIZE 73400320\`。
    - **沃** \`smtp.wo.cn:465\`：❌ **连不上**——服务器只接受**静态 RSA** 密钥交换套件
      （\`openssl s_client\` 协商出 \`TLS_RSA_WITH_AES_128_GCM_SHA256\` 可证），
      而 rustls 只支持 ECDHE，**换 TLS 版本无效**。⇒ 沃的发信需要另一个 TLS 后端，
      P3 首发只做 163 / QQ。
    - **126**：本机凭据文件里没有 126 账号，**待用户补授权码**后再测。
    - 原始报告见 [PROTOCOLS.md「SMTP 实测」](PROTOCOLS.md#smtp-实测2026-10-28-起)。
  - 仍缺：**发信后的 \`APPEND\` 行为**（服务器是否存已发送副本）——必须用专用测试账户实测。
- [x] SMTP 提交、草稿、附件与签名；服务器已发送副本策略先实测，避免重复 APPEND。
  - [x] **提交路径 + 草稿 + 附件**（2026-10-28）：协议层 `submit_message`
        （MAIL FROM / RCPT TO / DATA / QUIT + 分阶段结果）、存储层 `send_queue` 表与 DAO、
        服务层 `ComposeService`（落草稿 → **冻结字节** → 抢占 → 提交 → 记结局 → 回读）、
        前端写信面板（800ms 防抖自动存草稿）
  - [ ] **签名（DKIM / 域签名）未做**——属于 MIME / 发送链路的上游，不在协议层
  - [ ] **已发送副本策略未实测**（服务器是否 `APPEND` 到「已发送」）——
        在实测之前**不自动**往「已发送」里塞副本，免得两边都存一份
  - [x] **已发送副本行为已实测**（2026-10-29，真发信）：**163 服务器自己存**
        （已发送里找得到），**QQ 不存**——所以不能一律自己存。
        → 待做：协议层 `APPEND` + 「发完去已发送找标记，找不到才补一份」的自适应逻辑
- [x] envelope recipients 与显示头分离，Bcc 不泄漏且不破坏 To/Cc。
  - Bcc **只进信封**（`RCPT TO`），不进邮件头；信封收件人 = To ∪ Cc ∪ Bcc 去重
  - 用例：`bcc_recipient_is_only_in_the_envelope`
- [x] DATA 最终应答丢失测试与用户可见 uncertain 状态；不盲重发。
  - 状态机已接上发信路径：`SendState::Uncertain` **不可自动重试**（服务器可能已收下它）
  - 前端 `SendResult.state` 三态分开显示，`uncertain` 时**禁用发送按钮**
  - 用例：`missing_server_reply_is_uncertain_not_retryable`
- [x] **测试用例扩写 T8 续**（SMTP 提交阶段分类、重复投递防护）。
  - 提交阶段分类：`NotSent`（前置失败，可重试）/ `Rejected`（服务器拒收）/ `Uncertain`（DATA 后丢应答）
  - 重复投递防护：不确定的信不自动重发 + 冻结字节后不可改内容
  - 用例：服务层 8 条、存储层 18 条、IPC 2 条、协议层 12 条（**不依赖真实邮箱**）

**出口**：专用测试账户发信成功与失败恢复都有证据，收件人/附件/Bcc 正确。

### P4：Windows 与 Android 平台验收

- [ ] Windows 托盘、通知、单实例、退出/恢复。
  - [x] **安全存储**（2026-10-26）：Windows 凭据管理器适配已落地（`secrets/windows_store.rs`），
        不静默降级；托盘 / 通知 / 单实例 / 退出恢复仍未做。
- [ ] **早期可并行** Android PoC：构建、Rust 调用、Keystore 适配路线、前台/后台/息屏/进程恢复。
- [ ] 根据 PoC 决定移动端前台同步与后台策略；必要时请求用户选择时效/功耗取舍。
- [ ] Android 阅读/发信/本地缓存验收，背景任务限制如实展示。

**出口**：Windows 与 Android 分别记录能力矩阵；若 Android 未验收，只能称 Windows alpha，不能称双端 MVP。
当前未获授权取消 Android 或承诺始终后台实时同步。

### P5：本地增值功能（按需求选择）

标签、规则、联系人、稍后处理逐项交付。每设备独立的事实必须在 UI 明示。
Gmail 将来有需求再开独立决策：授权流程、scope、个人使用与公开发布要求均重新核实，不预设 Device Code 可用。

### P5-L：Linux 支持（可选，不承诺）

**当前事实**：Linux 编译已被 CI 验证（ubuntu job 编译了 tauri 应用本体）；**运行未验证**。
详见 [ENVIRONMENT.md §2](ENVIRONMENT.md)。

若要支持 Linux，需额外完成：

- [ ] WebKitGTK 沙箱验证：sandbox iframe、CSP、`srcdoc`、远程图片阻断的行为与 Chromium 有差异，
      必须重跑 [TESTING_STRATEGY §2.4](TESTING_STRATEGY.md) 的隔离测试，不能直接沿用 Windows 结论
- [ ] Secret Service 凭据后端（与 Windows Credential Manager / Android Keystore 分离实现）
- [ ] 托盘（libayatana-appindicator）、通知（D-Bus）、开机自启（XDG autostart .desktop）
- [ ] 打包链路验证（AppImage / deb / rpm）
- [ ] 至少 2 个主流发行版的真机运行验证（Ubuntu / Fedora 系）

> 优先级低于 P4 的 Windows/Android 验收。**在用户明确提出需求前不投入。**

### P6：可选 L2 加密同步

先做威胁模型、账户认证、CAS、密钥恢复、回滚保护，再实现服务与双设备测试。凭据同步默认关闭，不能因已有 trait 宣称无需返工。

### P7：个人安装与长期维护

可在 Windows alpha 后提前做个人安装包、备份/恢复、迁移和排错文档。不把公开上架、商业签名、社区增长当作当前前提。
额外平台与公开分发有需求时再确定验收范围。

## 4. 下一批任务（不附虚构工期）

按依赖顺序排列；**每完成一个门槛再选下一批**，不把建议当作用户授权。

1. ~~**把同步引擎接进应用**（P1 出口条件的最后一节）。~~ **已完成（2026-10-26）**。
   - 服务层：文件夹与邮件列表的查询；Command 接线；前端加一个邮件列表
   - 前提已全部具备：crate 层的拉取/回写链路已通，`xremail-store` 有 message / message_body DAO
   - 验收：UI 里能看到**真实邮箱**里的邮件（中文标题、正文）——**已达**（见下）
2. ~~**凭据录入**~~ **已完成（2026-10-26）**：授权码进应用，并**落平台钥匙串**
   （Windows 凭据管理器；数据库里只有不透明 `secret_ref`）。
   - 与 [SECURITY §2.1](SECURITY.md) 一致：数据库里只有不透明的 `secret_ref` ✓
   - 平台不支持时**如实报错**，不静默降级成内存 / 文件
   - 遗留：Android 侧钥匙串适配（Keystore）还没做
3. **调度器**（AccountActor）：什么时候同步、同步哪些文件夹、重连退避。
   - 能力判定已有（QQ 有 IDLE，163/沃 无 ⇒ 退化为轮询）
   - R13 的 `catch_unwind` 边界在这一步落地
   - **前置已就位**：凭据可从平台钥匙串取用（不用每次问用户要授权码），
     写操作队列已有六态与回读确认（失败时调度器只管重试 `retryable` 的那些）
4. ~~**同步引擎接进应用的同时跑一次真邮箱验收**~~ **已完成（2026-10-26）**：
   三个测试账号各跑一次完整同步（连接 → 登录 → 列文件夹 → 拉取 → 落库 → 读正文），
   脱敏证据：163 = 6 文件夹 / 4 封 / utf-8；QQ = 6 文件夹 / 71 封 / gb2312；
   沃 = 6 文件夹 / 6 封 / gbk；正文解码**零替换字符**。
   工具：`xremail-sync-smoke`（`crates/xremail-diagnostics`）。
5. **SMTP**（P3）：先实测三家的 SMTP 参数（R18），再写提交与 `uncertain` 恢复。
6. 提前做有限 Android 生命周期验证，不等待完整 UI。

前 4 项完成时，P1 的出口条件（「三种真实账户各有实测记录」）才真正达成。

**已完成（2026-10-26）**：同步引擎已接进应用（UI 能看到真实邮件），三家真实账号
各跑通一次完整同步（见上）；**凭据落平台钥匙串**（Windows 凭据管理器）；
**写操作真正回写服务器**（标记已读 / 星标 → `UID STORE` → 回读确认 → 六态回报），
三家真实邮箱的 `UID STORE` 实测记录在 [PROTOCOLS.md](PROTOCOLS.md)。

**下一批建议（2026-10-26，未获授权不动手）**：**调度器（AccountActor）**——
凭据不再需要人工输入，且写操作有了可靠的失败分类，调度器才有东西可调；
其次是 **HTML 正文渲染（R06）**（P2 的阅读体验缺口）。
**SMTP（R18）**排在后面：它要先实测三家的 SMTP 参数，风险与不可逆性都更高。

**已完成（2026-10-23）**：三家测试账号已准备并完成只读实测（凭据仅在本机配置）；
只读 probe、假服务器、同步纯函数、同步引擎（拉取 + 回写 + 按需正文）、
MIME/charset 解析均已落地。

相关：[架构](ARCHITECTURE.md) · [风险](ISSUES_AND_RISKS.md) · [测试](TESTING_STRATEGY.md) · [AI 规范](AI_DEVELOPMENT_GUIDE.md)
