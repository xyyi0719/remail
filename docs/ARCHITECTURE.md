# 架构设计

> 当前设计基线；不代表所有模块已实现。执行范围见 [ROADMAP](ROADMAP.md)，验收见 [TESTING_STRATEGY](TESTING_STRATEGY.md)。
> 保留：Rust 核心 + Tauri 2 + React/TS、先 L1、P0 预留 L2。修订：同步语义、操作恢复、检索、安全边界。
> 维护者：xyyi0719

## 1. 原则与边界

1. Rust 是持久化写入入口；UI 发送命令并读取 DTO，不直接操作数据库或协议。
2. SQLite 是**本机读模型**，不是服务器邮件状态的全局权威；服务器是 L1 邮件/flag 的权威，SQLite 还保存本地意图与待发送操作。
3. 离线只能读取已缓存内容；未下载正文/附件不可用。离线操作排队，不承诺已送达。
4. `xremail-core` 不依赖存储/网络/UI；称为“依赖方向受控”，不使用“无依赖倒置”这一含混说法。
5. `xremail-sync` 初期明确是 **IMAP-aware** 的编排层。UIDVALIDITY、UID、MODSEQ 属于 IMAP 语义，不伪装成通用传输能力；JMAP 将另行设计适配。
6. 状态提交后才发事件；事件是刷新提示，SQLite 查询是可恢复事实。UI 重连/漏事件后重新查询。
7. 凭据、HTML 邮件及附件默认不可信；安全边界见 SECURITY，不通过失败降级削弱。
8. 先保证数据正确与失败可恢复，再做性能和跨平台扩展；所有容量与连接上限均需测量。

## 2. 模块与依赖

当前只有 `xremail-core`、`xremail-store`、`xremail-crypto`、`xremail-api`、`xremail-sync`、`xremail-protocol-imap` 与 `apps/shell/src-tauri`；以下为目标职责，按需创建，不要求一次拆齐。

| 模块 | 职责 | 边界 |
|---|---|---|
| xremail-core | 领域模型、错误、事件、身份/操作类型 | 不依赖 sqlx/tokio/tauri |
| xremail-api | 应用服务 DTO、L2 最小契约 | 不实现网络；不强迫其他模块反向依赖 UI |
| xremail-store | SQLite、迁移、事务、检索投影、操作队列 | 不发网络请求 |
| xremail-protocol-imap | IMAP 会话、能力与命令结果 | 不写库，不决定重试 |
| xremail-protocol-smtp | SMTP 提交与分阶段结果 | 不保证 exactly-once |
| xremail-sync | 账户调度、IMAP 同步、操作恢复 | 依赖 store/protocol/core，不认识 Tauri |
| xremail-message | MIME/charset、构建、HTML 清洗 | 不直接落库或加载远程资源 |
| xremail-provider | 可覆盖预设、接入诊断 | 企业邮箱需管理员配置，不能按品牌猜服务器 |
| xremail-crypto | 成熟密码库封装、版本化密文格式 | 不自行实现密码算法；不承担所有平台钥匙串职责 |
| 平台 SecretStore 适配器 | Windows/Android 等安全存储 | 与可选远端 VaultStore 分离 |
| xremail-testkit | 假服务器、合成样本、故障注入 | 测试专用 |
| shell | 组合根、Command/Event、窗口/平台生命周期 | Command 薄封装，业务在服务层 |

依赖基线：`shell → 应用服务/编排 → store、protocol、message、provider → core`。
trait 放在使用方或低层契约模块；不因“只有 trait”就允许产生环。密钥存储与 vault 同步不是同一接口。

## 3. 运行时与连接所有权

- 每账户一个 AccountActor 接收有界 mailbox，拥有账户 IMAP 会话及同步状态。
- 首版默认每账户一条 IMAP 连接；仅为保守起点，不声称服务商固定允许 3–5 条。连接数与全局预算配置化，后续按实测调优。
- 同一流只允许一个拥有者读写。IDLE 是会话状态，不是另一个 task 同时读 socket。
- 用户操作到达：请求退出 IDLE → 发送 DONE → 等 tagged completion → 执行命令 → 返回 IDLE。
- IDLE 仅监听被选中的 mailbox；其他文件夹需轮询或调度切换，不能声称监听 INBOX 覆盖全账户。
- 无 IDLE 时轮询；认证失败进入待重新授权，不用无限重连掩盖问题。
- SMTP 可独立连接；操作 worker 按账户/文件夹身份顺序执行，不能与 Actor 争用 IMAP 流。
- SQLite WAL、有界连接池、busy timeout；所有业务写入由 Rust 协调，不等于只有一个连接或一个 task。
- 退出、账户删除、网络切换均传播取消；后台任务应被监督并回收。

## 4. 身份、缓存与同步事务

### 4.1 身份不变量

远端邮件定位使用 `(account_id, folder_id, uidvalidity, uid)`。本地 message ID 独立生成。
RFC Message-ID 只能作线索，不能作为唯一键或自动跨文件夹去重依据。

### 4.2 同步主路径

1. 建立并校验 TLS；认证前后按需要获取 CAPABILITY，使用认证后能力。
2. LIST/只读选择文件夹，取得 UIDVALIDITY、可用的 MODSEQ 等信息。
3. 对比已提交游标；epoch 变化时先走 4.3。
4. 根据能力拉 UID 集合/增量 FLAGS；分批 upsert 摘要，事务提交游标后发事件。
5. 按阅读需求或缓存策略用 BODY.PEEK 拉正文/附件；正文到达与检索投影同事务更新。
6. 比对服务器已删除 UID；删缓存必须尊重本地草稿/未发送内容的独立归属。
7. 进入 IDLE 或轮询；所有请求有超时、资源上限和可取消边界。

**实现落点（2026-10-22，`crates/xremail-protocol-imap`）**：第 1 步与第 2 步的实测结论已固化——


* **服务器宣告 `ID` 能力时，登录后必须立即发一次 `ID`（RFC 2971）**。163 在不发 `ID` 时把
  `EXAMINE` / `SELECT` 一律回成 `NO … Unsafe Login`，邮箱根本进不去。内容任意（`ID NIL` 除外）。
* **能力集只能在认证后拿**：163 认证前拒绝一切 IMAP 命令，连 `LOGOUT` 都不行。
* **邮箱名在线路上是 modified UTF-7**（RFC 3501 §5.1.3），不解码就认不出「已发送」；
  实现在 `xremail-protocol-imap::utf7`。
* **游标不能一律基于 UIDNEXT**：163 的 `STATUS` 接受 `UIDNEXT` 字段但**不给值**，
  须用 `UID SEARCH` / `UID FETCH 1:*` 取最大 UID。
* **不要无条件请求 `HIGHESTMODSEQ`**：不支持 CONDSTORE 的服务器（如 163）会回
  `BAD Parse command error`。

初次同步优先摘要与最近邮件，不默认下载全部附件。性能指标必须区分摘要、正文与附件。

### 4.3 UIDVALIDITY 变化

在一个数据库事务内切换 epoch、失效旧 UID 映射与游标、隔离绑定旧 epoch 的 pending 操作。
禁止把旧操作直接套到新 UID；重新全量建立该文件夹快照。保留操作记录与冲突原因供恢复/提示，附件引用按回收策略处理，不能误删共享 blob。
PLAN 已有 `sync_state.uidvalidity`，无需重复添加同义字段。

**实现落点（2026-10-22，`crates/xremail-sync/src/epoch.rs`）**：`plan_epoch_transition` 把三种情形分开——
epoch 未变（增量）、首次同步（全量拉取）、epoch 变了（**重建快照**）。
重建时列出绑定旧 epoch 的待提交操作 ID（`quarantined`），**不**把它们套到新 UID 上。
UIDVALIDITY 不得为 0（RFC 3501），收到 0 视为协议错误。

**实现落点（2026-10-22，P0 迁移 `0001_init.sql`）**：epoch 与游标统一放在 `folder` 表
（`uidvalidity` / `last_uid` / `highest_modseq` / `last_sync_at`），与 `xremail-core` 的
`Folder` 结构一一对应，**不另建 `sync_state` 表**——那样会出现两处同义字段。
`mail_operation` / `send_queue` 对邮件只做**逻辑引用**（不设外键）：epoch 切换后旧 UID 失效，
这些操作应被隔离并保留冲突原因，而不是随邮件一起被级联删除。

## 5. 本地操作与恢复

将队列分为 **mail_operation**（flag/move/delete/append）与 **send_queue**（SMTP）。名称可在迁移时定稿，避免一个模糊 outbox 混淆语义。

- 一次本地命令在同事务内写操作意图与本地投影；提交后返回/发事件。
- UI 可立即显示带 pending 标识的本地结果，不能把它显示为服务器确认。
- 状态至少区分 queued、running、succeeded、retryable、failed、uncertain；具体枚举在实现时通过测试固定。
- 记录操作 ID、目标身份/epoch、基线版本、attempts、错误类别。进程中断的 running 必须按操作类型核对，而非一律重放。

| 操作 | 重试原则 |
|---|---|
| bit 级 +FLAGS/-FLAGS | 可核对状态后重放；不要整组替换 FLAGS |
| MOVE/COPY/APPEND | 若响应丢失，先依据 UIDPLUS/MOVE 结果或可用线索核对；无法可靠确认则 uncertain |
| 删除 | 首版优先移入垃圾箱。无 UIDPLUS 时不普通 EXPUNGE，防止清除别的客户端已标删除的邮件 |
| SMTP | 连接/提交阶段分别分类；最终 DATA 可能已接受时标 uncertain，禁止盲重发 |

Message-ID、稳定操作 ID 有助于排查，但不能保证 SMTP 去重。明确不承诺网络故障下 exactly-once。

## 6. flag 并发语义

服务器快照与本地待提交意图分开保存。`+Seen` 和 `-Seen` 可以冲突，并非“单 bit 都幂等所以没有冲突”。

- 有 CONDSTORE：基于已知 MODSEQ 用 UNCHANGEDSINCE 条件写；MODIFIED 后重新读取并按意图重算，重试有界。
- 无 CONDSTORE：读取快照、重放本地 bit 级意图、发送后回读确认；接受服务器执行顺序带来的竞态窗口。
- 本地时间戳/HLC 不能排序普通 IMAP 客户端的远端操作；MODSEQ 也不是跨 mailbox 的全局时间。
- 移动后原 UID 无效；删除/移动冲突应重新核对存在性，无法定位时失败或交给用户，不按时间戳猜测目标。
- L2 自建同步未来可用显式版本/CAS；不能把它的 HLC/LWW 规则套进 IMAP。

**实现落点（2026-10-22，`crates/xremail-sync/src/intent.rs`）**：`FlagIntent` 记录一个 bit 级意图
（uid、**epoch**、add/remove、`seq` 顺序号、基线 MODSEQ）。`apply_intents` 按 `seq` 排序作用于快照，
后者覆盖前者——`+Seen` 与 `-Seen` 的冲突就是这样定序的。
`plan_flag_update` 只产出**净变化**的位：净变化为空时返回 `None`（不发命令），
其余只含真正变化的位，**不整组替换 FLAGS**。绑定旧 epoch 的意图进 `FlagPlan::stale`，不参与计算。
服务器回 `MODIFIED` 后，客户端用新快照重调 `plan_flag_update` 即可重算，
次数上限见 `MAX_REPLAN_ATTEMPTS`。

## 7. 检索模型

推荐先做显式 `search_document` 投影，聚合摘要与已解码正文；使用普通存内容 FTS5 表并在 DAO 事务内维护索引。
这是待迁移验证的设计，不沿用 PLAN 原来的 `content=''` 示例。若空间测量后改外部内容表，必须另行定义列对应、删除协议、触发器与重建。

元数据变化、正文晚到/替换、删除、UID epoch 重建都要更新投影。提供可重复 rebuild 与一致性检查。
trigram 对不足三个字符的检索有限制，中文短词采用有界 LIKE 回退或明确 UI 限制，不能假装具备完整中文分词。

## 8. L2 预留的最小范围

P0 按既定决策保留接口与 schema，但不建设远端服务：

- `SecretStore`：平台凭据读写删，与普通 SQLite 存储分离；失败语义明确。
- `VaultStore`：本地版本化密文封装与元数据持久化，不等于 OS 钥匙串。
- `SyncBackend`：读取远端密文/revision；以 expected revision 条件提交，原子返回新 revision 或冲突。
- 内存 mock 测试首次写、往返、双设备 CAS 竞争、错误返回；禁止独立 check_conflict 后无条件 push 的 TOCTOU 设计。
- label/rule 等表预留稳定 ID、schema version 与必要生命周期字段；不提前冻结复杂 CRDT/同步协议。

**实现落点（2026-10-22，`crates/xremail-api`）**：三个契约已建，内存 mock 可运行。
`SyncBackend` **只提供原子的 `compare_and_swap`**，刻意不提供「先查冲突再推送」的组合方法——
检查与推送之间状态可能已变，那会静默丢失更新（TOCTOU）。冲突错误 `SyncError::Conflict { current }`
携带远端当前修订号，调用方据此重新读取并合并。`xremail-crypto` 提供版本化密文封装，
其中 `to_bytes`/`parse` 是**自包含**字节格式，可作为不透明载荷在设备间搬运。

P6 前重新审查身份认证、密钥恢复、轮换、回滚/重放防护与元数据泄露。接口预留降低耦合，不保证零返工。

## 9. 平台与错误边界

Windows 先打通完整链路，Android 保留目标并尽早做小型生命周期 PoC。普通轮询不保证 Doze 下准时执行；前台服务也不是永久保活承诺。
Linux/macOS/iOS 不属于当前验收平台，不因 Tauri 支持就视为产品可用。

错误按可恢复原因分类：传输临时失败退避；认证需用户动作；磁盘满/迁移失败不能当普通 Storage 错误无限重试；协议错误不原样输出可能含凭据/正文的响应。
事件枚举以 `xremail-core/src/event.rs` 为准，flag 更新使用 MessageUpserted，当前没有 FlagsChanged。
**约束冲突不是可重试错误**：`xremail-store` 把唯一键/外键/CHECK 违反映射为 `AppError::Invalid`
（不可重试），其余 sqlx 错误映射为 `AppError::Storage`（可重试）。重试同一份输入不可能成功，
因此不能一律归入 Storage。

错误附加上下文时必须保留错误变体，否则 `is_retryable()` / `requires_user_action()` 判定失效。
`AppError::with_context` 按变体逐个包裹，`Context::ctx` 基于它实现，已有单测锁定该行为。
`panic = "abort"` 意味着后台任务无法用 `catch_unwind` 兜底，单个同步 task 的 panic 会终止整个应用；取舍见 [ISSUES_AND_RISKS R13](ISSUES_AND_RISKS.md)。

## 10. 性能预算与复审

旧版启动、内存、5000 封 3 分钟、30 秒双端一致等数字都是未测目标，不是门槛保证。
先测基线，记录设备/网络/数据规模与缓存状态，再设回归阈值；安全和数据正确性不因性能而让步。

架构变更需写明不变量、迁移与失败路径；改变 UI 栈、同步层次或 Android 范围须用户确认。测试覆盖见 TESTING_STRATEGY。

相关：[风险](ISSUES_AND_RISKS.md) · [安全](SECURITY.md) · [测试](TESTING_STRATEGY.md)
