# Changelog

本项目遵循 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.1/)，版本号遵循 [语义化版本](https://semver.org/lang/zh-CN/)。

维护者：xyyi0719

## [Unreleased]

### Added
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
- `remail-core` crate：领域模型（账户、文件夹、邮件、Flags、地址、事件）
- `remail-core` 错误分类单测 4 个

### Changed
- `ProviderKind` 的沃邮箱变体重命名为 `Wo`（显示名「沃邮箱」），移除服务商名称中的厂商字样；序列化值同步变更
- 文档与提交信息中不再出现厂商名称，仅保留产品名与域名
- 架构语义重写：本地 SQLite 明确为读模型；服务器快照与本地操作意图分离；不承诺 exactly-once
- 冲突仲裁不再用本地时间戳猜测远端操作顺序；有 CONDSTORE 用 UNCHANGEDSINCE，否则快照 + 意图重放
- 删除策略：无 UIDPLUS 时不使用普通 EXPUNGE，避免清除其他客户端的删除标记
- 安全：撤回凭据静默降级与沙箱弹窗逃逸建议；默认禁网；明确 AEAD 不能单独防回滚重放
- 路线图改为按验收门槛推进，撤回未经测量的耗时系数与固定交付日期
- 文档作者/负责人署名为 xyyi0719

### Fixed
- `AppError` 的 `ctx()` 不再把错误统一折叠成 `Other`，保留 `is_retryable()` / `requires_user_action()` 判定
- 代码注释中的文档相对路径断链（`crates/remail-core/src/*.rs`、`apps/shell/src-tauri/src/*.rs`）
- 代码注释中过期的 `ARCHITECTURE §N` 章节号引用
- workspace `resolver` 由 `2` 改为 `3`（edition 2024 隐含 MSRV 感知解析），Cargo.lock 无变化

### Notes
- 关键技术选型已锁定：Tauri 2 + React/TS、先做 L1（IMAP 原生）同步并预留 L2 抽象
- 后续持续扩写测试用例，按 TESTING_STRATEGY §5 的 T1–T10 顺序在 P0–P3 分批推进
- Gmail 暂缓，不再是 P1 关键路径；恢复接入前需重新核实授权与发布要求
- 弃用同步版 `imap` crate
- 尚未实现：SQLite 持久化、协议适配、收发链路、自动化业务测试
