# Changelog

本项目遵循 [Keep a Changelog](https://keepachangelog.com/zh-CN/1.1.1/)，版本号遵循 [语义化版本](https://semver.org/lang/zh-CN/)。

维护者：xyyi0719

## [Unreleased]

### Added
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

### Changed
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
  - 探针脚本从 `TRIES=3` 改为 `TRIES=12`，并把 **push 本身也纳入同一重试循环**（探针过了 push 仍可能被 reset）；实测一次真实推送在第 4 次尝试才通过
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
