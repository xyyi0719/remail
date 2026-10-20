# 贡献指南

> 维护者：xyyi0719

## 环境

| 依赖 | 版本 | 安装 |
|---|---|---|
| Rust | ≥ 1.88 | `rustup update stable` |
| Node.js | ≥ 20 | `fnm install 20` / `nvm use 20` |
| pnpm / npm | 任意 | 仓库用 npm（`package-lock.json` 已提交） |
| Android SDK + NDK + JDK 17 | 仅 Android | Android Studio |
| Windows: WebView2 + MSVC Build Tools | 仅 Windows 桌面 | [Tauri 前置依赖](https://v2.tauri.app/start/prerequisites/) |

> 2026-10-20 本机实测：Rust 1.99.1、Node 22.21.0 可用；**仅装有 JDK 8 且无 Android SDK**，
> 因此本机暂时无法构建 Android 产物，P4 前需先补 JDK 17 + SDK/NDK。

```bash
npm install --prefix apps/shell
cargo check --workspace
```

---

## 目录与职责

| 路径 | 放什么 | 不放什么 |
|---|---|---|
| `crates/remail-core` | 领域模型、错误、事件 | 任何 `sqlx` / `tokio` / `tauri` 引用 |
| `crates/remail-store` | SQL、迁移、DAO | 网络调用 |
| `crates/remail-sync` | 同步状态机、调度、outbox | UI 相关的一切 |
| `crates/remail-protocol-*` | 协议细节 | 数据库、业务规则 |
| `crates/remail-api` | trait 与 DTO | 实现（实现放各领域 crate） |
| `apps/shell/src-tauri` | Command、Event、平台集成 | 业务规则（应在 crate 里） |
| `apps/shell/src` | React 组件、路由、样式 | 协议细节、SQL、凭据处理 |

**依赖方向铁律**：`remail-core` ← 一切；`remail-api` ← 一切；UI → `remail-api`；`remail-sync` 不认识 UI。

---

## 代码风格

### Rust

- `cargo fmt`（默认 rustfmt，不改配置）
- `cargo clippy --workspace --all-targets -- -D warnings` **必须零告警**
- 公开 API 写 `///` 文档注释；crate 顶部写 `//!` 说明
- 错误类型用 `thiserror`；应用层错误用 `anyhow`；**库 crate 不要用 `anyhow`**
- 禁止 `unwrap()` / `expect()` 于可能失败的运行时路径（测试代码除外）
- 禁止 `panic!` 作为错误传播手段
- 公开结构体加 `#[derive(Debug, Clone, PartialEq, Eq)]`（便于断言）
- 不使用 `unsafe`（如确需，单独 PR 说明理由）

### TypeScript / React

- `strict: true`，禁止 `any`（实在不行用 `unknown` + 收窄）
- 组件用函数式 + Hooks；**不写 class 组件**
- 数据获取统一用 TanStack Query；UI 局部状态用 Zustand
- **禁止在 React 里直接调 `invoke` 之外的方式取数据**；所有数据经 `remail-api` 的 Tauri Command
- 样式用 Tailwind 原子类；组件样式用 `shadcn/ui` 变体
- 列表一律虚拟化（TanStack Virtual）
- 可访问性：交互元素必须有 `aria-label` 或可见文本

### 命名

| 类型 | 约定 | 例 |
|---|---|---|
| Rust 结构体/枚举 | `PascalCase` | `MessageFlags` |
| Rust 函数/变量 | `snake_case` | `list_messages` |
| TS 组件 | `PascalCase` | `MessageList.tsx` |
| TS 函数/变量 | `camelCase` | `useAccounts()` |
| DB 表 | `snake_case` 复数 | `message` / `outbox` |
| 事件名 | `PascalCase` 变体 | `AppEvent::MessageUpserted` |
| 迁移文件 | `NNNN_snake_case.sql` | `0001_init.sql` |

---

## 提交规范

[Conventional Commits](https://www.conventionalcommits.org/)：

```
<type>(<scope>): <subject>

<body>

<footer>
```

**type**

| type | 用途 |
|---|---|
| `feat` | 新功能 |
| `fix` | 修 bug |
| `docs` | 只改文档 |
| `refactor` | 不改行为的重构 |
| `perf` | 性能 |
| `test` | 测试 |
| `build` | 构建 / 依赖 |
| `ci` | CI 配置 |
| `chore` | 杂项 |

**scope**（尽量用 crate / 模块名）：`core` `store` `sync` `imap` `smtp` `crypto` `provider` `shell` `ui` `docs`

示例：

```
feat(sync): 支持 CONDSTORE 增量拉取
fix(imap): UIDVALIDITY 变更时清空本地缓存并重拉
docs(protocols): 补充 163 的 IDLE 实测结果
feat(ui): 邮件列表支持按账户筛选
```

---

## 测试策略

以 [TESTING_STRATEGY.md](TESTING_STRATEGY.md) 为当前验收规范。下表是目标设施，不能据此宣称 Vitest/Playwright/Mock 服务器已安装或已通过；真实 Tauri/WebView 与 Android 另需宿主验证。
AI 工作流与审查模板见 [AI_DEVELOPMENT_GUIDE.md](AI_DEVELOPMENT_GUIDE.md)。

| 层 | 工具 | 要求 |
|---|---|---|
| Rust 单元测试 | `cargo test` | 状态机转换、编解码、charset 嗅探、冲突仲裁必须有单测 |
| Rust 集成测试 | `cargo test` + `remail-testkit` | Mock IMAP/SMTP 服务器跑通完整同步流程 |
| SQL 迁移 | `sqlx` | 迁移可前滚；破坏性变更需写 `docs/adr/` |
| 前端单测 | Vitest + Testing Library | 关键组件与 hook |
| 端到端 | Playwright | 账户添加 → 收信 → 阅读 → 已读回写 的主链路 |
| 回归样本 | `crates/remail-testkit/src/samples/` | 各家真实邮件的**脱敏**样本（含 GBK 样本） |
| 性能 | 待建基准 | 见 [ARCHITECTURE §10](ARCHITECTURE.md)，先测基线再设阈值 |

**门槛**：新增协议解析 / 状态机 / 冲突处理代码，必须同时带测试。

用例扩写的优先级、阶段归属与「算完成」标准见 [TESTING_STRATEGY §5](TESTING_STRATEGY.md)。

---

## 文档规则

| 什么时候 | 改哪份 |
|---|---|
| 实测了某服务商的参数或行为 | [PROTOCOLS.md](PROTOCOLS.md) —— **最重要的贡献** |
| 改了架构或模块职责 | [ARCHITECTURE.md](ARCHITECTURE.md) |
| 做了不可逆的技术决策 | 新增 `docs/adr/NNNN-*.md`（模板见 ADR-0001） |
| 改了范围 / 里程碑 / 风险 | [PLAN.md](PLAN.md) + [ROADMAP.md](ROADMAP.md) |
| 改了安全策略 | [SECURITY.md](SECURITY.md) |
| 用户可见的行为变化 | [CHANGELOG.md](../CHANGELOG.md) |

**ADR 模板**

```markdown
# ADR-NNNN：<标题>

- 状态：提议 / 已接受 / 已废弃 / 被 ADR-XXXX 取代
- 日期：YYYY-MM-DD
- 决策者：<姓名>

## 背景
<为什么现在要做这个决定>

## 决策
<决定了什么>

## 理由
<为什么选它>

## 后果
### 正面
### 负面 / 代价
### 风险与缓解

## 备选方案
<考虑过但没选的，及为什么>

## 复审条件
<什么情况下重新评估这个决定>
```

---

## PR 流程

1. 从 `main` 切分支：`feat/xxx` / `fix/xxx` / `docs/xxx`
2. 本地过一遍：`cargo fmt && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace` + `npm run build --prefix apps/shell`
3. PR 描述写清：**改了什么 / 为什么 / 怎么验证 / 截图（UI 变更）**
4. CI 必须绿
5. 至少一位 reviewer 批准（可自己 review 别人的）

**分支保护**：`main` 不允许直接 push。

---

## 敏感信息红线

以下**永远不提交**（已在 `.gitignore` 中，但也要自觉）：

- 邮箱授权码 / 密码 / OAuth token
- `oauth/clients.json`（OAuth Client ID/Secret）
- 真实邮箱地址（测试代码里用 `test@example.com`）
- 真实邮件正文样本（**必须脱敏**后再进 `remail-testkit`）

发现泄漏：**立即重置该凭据**，然后 `git filter-repo` 清理历史并通知所有克隆者。
