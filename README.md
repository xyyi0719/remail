# Xremail

**多端、多账号、隐私优先的邮件客户端。** 核心用 Rust 写，界面用 Web 技术，Windows / Android 优先，iOS / macOS / Linux 随后。
> 维护者：xyyi0719

计划先接入 163、QQ 与沃邮箱，保留通用 IMAP/SMTP 手动配置；Gmail 暂缓，其他服务商后续评估。目前尚不能实际收发邮件。
**L1 邮件状态由 IMAP 服务器同步**，时效受网络、服务商和平台后台限制；本地设置等 L2 数据暂不跨端。

---

## 当前状态

> **v0.1.0 · P0 脚手架阶段**（2026-10）
>
> 工程骨架已生成，架构持续复核，协议与持久化业务尚未实现。当前设计见 [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)。
> 2026-10-23 验证：`cargo fmt` / `cargo clippy -D warnings` / `cargo test`（381 个用例）/ `npm typecheck` / `npm build` 均通过。
> 存储层（`xremail-store`）、密文封装（`xremail-crypto`）、L2 契约（`xremail-api`）、同步纯函数（`xremail-sync`）、IMAP 协议层（`xremail-protocol-imap`）、内容解析（`xremail-message`）与假服务器（`xremail-testkit`）已落地，同步引擎的**拉取与回写**链路已打通；**尚未接入应用**（`list_accounts` 仍返回硬编码空列表）。
> 想看进度：[docs/ROADMAP.md](docs/ROADMAP.md)

| 状态 |
|---|---|
| P0 骨架与地基 | 🟡 进行中 |
| P1 账户接入与收信 | ⬜ |
| P2 同步引擎与阅读 | ⬜ |
| P3 发信与草稿 | ⬜ |
| P4 平台化与实时推送 | ⬜ |
| P5 增值功能 | ⬜ |
| P6 跨端账号信息同步（可选） | ⬜ |

> 路线图以验收门槛描述，不承诺固定日期（个人研究项目）。

---

## 架构

```
┌────────────────────────────────────────────────────────┐
│  React 19 + TypeScript          仅渲染，无业务逻辑          │
└───────────────────▲────────────────────────────────────┘
                    │ Tauri Command（请求/响应）
                    │ Tauri Event（增量推送）
┌───────────────────┴────────────────────────────────────┐
│  src-tauri (Rust)                 托盘 / Android 前台服务   │
├────────────────────────────────────────────────────────┤
│  crates/                          全部业务逻辑在 Rust 侧    │
│  xremail-core       领域模型       xremail-sync   同步引擎    │
│  xremail-store     SQLite + FTS   xremail-crypto 加密/密钥派生│
│  xremail-protocol-* IMAP/SMTP/OAuth                     │
│  xremail-api       领域服务接口    xremail-provider 厂商预设  │
└────────────────────────────────────────────────────────┘
```

**目标架构：UI 经 Rust 服务查询 SQLite，持久化写入只走 Rust。** 事件用于刷新提示，漏事件后可重新查询；上图中除 core/store/crypto/api/shell 外的模块仍为规划。

---

## 技术栈

| 层 | 选型 | 版本 |
|---|---|---|
| 外壳 | Tauri | 2.12 |
| 前端 | React + TypeScript + Vite | 19 / 6.x / 8.x |
| 状态 | TanStack Query + TanStack Router + Zustand | 5.x |
| UI 基座 | Tailwind CSS + shadcn/ui (Radix) | 4.x |
| 长列表 | TanStack Virtual | 3.x |
| 富文本 | TipTap (ProseMirror) | 2.x |
| 运行时 | tokio | 1.54 |
| IMAP | async-imap + imap-proto | 0.12 / 0.17 |
| SMTP | lettre | 0.11 |
| 存储 | SQLite (sqlx, WAL) + FTS5 | 0.9 |
| 密钥库 | 平台 SecretStore（后端待验证） | 待定 |
| 密码学 | argon2 + chacha20poly1305 | 0.6 / 0.11 |

> 此表混合已使用与计划依赖；实际版本以 Cargo.lock/package-lock.json 为准。TanStack、编辑器、协议/存储/密码学库尚未接入，不代表已验证兼容性；依赖维护状态应在引入时复核。

---

## 目录结构

```
xremail/
├─ crates/                 Rust 库：core / store / crypto / api / provider / protocol-*
├─ apps/
│  ├─ shell/               Tauri 应用（桌面 + 移动）
│  │  ├─ src/              React 前端
│  │  └─ src-tauri/        Rust 外壳 + Command + Event
│  └─ sync-server/         可选：自建 E2EE 同步服务（P6）
├─ docs/                   本文档集
└─ .github/workflows/      CI
```

---

## 开发环境

| 依赖 | 版本 | 说明 |
|---|---|---|
| Rust | ≥ 1.88 | `rustup update stable` |
| Node.js | ≥ 20 | 建议用 fnm / nvm |
| Android 端 | Android Studio + NDK + JDK 17 | 仅编译 Android 需要 |
| 平台工具 | Tauri 2 的 [前置依赖](https://v2.tauri.app/start/prerequisites/) | Windows 需 WebView2 + MSVC Build Tools |

```bash
# 首次
npm install --prefix apps/shell

# 开发（桌面）
npm run dev --prefix apps/shell        # 仅前端
npm run tauri dev --prefix apps/shell  # 完整应用

# Android
npm run tauri android init --prefix apps/shell   # 只需执行一次
npm run tauri android dev --prefix apps/shell

# Rust 侧
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

---

## 文档

| 文档 | 内容 |
|---|---|
| [docs/PLAN.md](docs/PLAN.md) | 原始详细规划（历史参考，当前执行范围以 ROADMAP 为准） |
| [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) | 模块职责、线程模型、事件流、数据流时序、扩展指南 |
| [docs/PROTOCOLS.md](docs/PROTOCOLS.md) | 各服务商 IMAP/SMTP 实测参数与坑（**持续更新，实测即记录**） |
| [docs/SECURITY.md](docs/SECURITY.md) | 威胁模型、凭据处理、HTML 邮件沙箱规范、密钥派生 |
| [docs/ROADMAP.md](docs/ROADMAP.md) | 里程碑看板、每阶段完成定义、本周任务 |
| [docs/CONTRIBUTING.md](docs/CONTRIBUTING.md) | 开发约定、提交规范 |
| [docs/TESTING_STRATEGY.md](docs/TESTING_STRATEGY.md) | 测试分层、故障注入、协议/安全/平台验收 |
| [docs/AI_DEVELOPMENT_GUIDE.md](docs/AI_DEVELOPMENT_GUIDE.md) | AI 编码流程、任务模板、证据要求 |
| [docs/ISSUES_AND_RISKS.md](docs/ISSUES_AND_RISKS.md) | 风险登记册、旧审查纠错、关闭条件 |
| [docs/ENVIRONMENT.md](docs/ENVIRONMENT.md) | 本机 shell / 代理 / DNS / 工具链限制，各平台 WebView 与 API 差异 |
| [docs/adr/](docs/adr/) | 架构决策记录（ADR）：为什么选 Tauri、为什么先做 L1、Gmail 怎么处理 |
| [CHANGELOG.md](CHANGELOG.md) | 变更日志 |

---

## 三个必须先知道的现实约束

1. **国内邮箱优先，Gmail 暂缓**。个人研究项目按验收门槛推进，不承诺固定工期。目标服务商为 163、QQ 与沃邮箱（`wo.cn`）；恢复 Gmail 时重新核实 OAuth。当前范围见 [ROADMAP](docs/ROADMAP.md)。
2. **"同步所有账号信息"分三层**。邮件与已读/星标/移动/删除 → IMAP 原生跨端同步，零服务器；本地标签/规则/签名/联系人 → v1.0 每设备独立；凭据 → 存 OS 钥匙串，绝不明文同步。详见 [ADR-0002](docs/adr/0002-sync-depth-l1-first.md)。
3. **协议能力与后台时效需要实测**。服务商连接限制、IDLE/CONDSTORE 支持度不能按品牌推定；不支持时保守降级。Android 前台服务不保证永久后台同步，须真机验证。

---

## 贡献

见 [docs/CONTRIBUTING.md](docs/CONTRIBUTING.md)。特别是：**只要你实测了某个服务商的协议参数，请把结果写进 [docs/PROTOCOLS.md](docs/PROTOCOLS.md)** —— 这份文档的价值全靠真实数据积累。

## 许可

MIT OR Apache-2.0
