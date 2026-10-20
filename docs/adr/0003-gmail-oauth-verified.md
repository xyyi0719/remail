# ADR-0003：Gmail 走正式 OAuth 2.0 验证流程

- **状态**：执行暂缓（用户最新决策：国内邮箱优先，暂时跳过 Gmail）
- **复核说明**：下文保留历史方案，不是当前实现规范。恢复接入时重新核实平台授权流程、scope、Testing/Production/verification 区别与发布要求；不预设 Device Code 可用于普通桌面/Android 邮件客户端，也不承诺固定审核周期。
- **日期**：2026-10-18
- **决策者**：xyyi0719

---

## 背景

Gmail 是本项目四家目标服务商中**唯一有特殊协议要求**的一家。

已核实的官方政策：

| 生效时间 | 政策 | 来源 |
|---|---|---|
| 2025-03-14 | Google Workspace 全面禁止第三方应用使用密码基础认证（Basic Auth） | [Workspace 帮助](https://support.google.com/a/answer/14114704) |
| 2025-05-01 | Workspace 账号不再支持 "less secure apps"，只能用 OAuth | [Google 帮助](https://support.google.com/mail/answer/12103) |
| 持续 | IMAP/POP/SMTP **协议本身未下线**，但认证必须用 `AUTH XOAUTH2` | [Gmail 开发者文档](https://developers.google.com/workspace/gmail/imap/imap-smtp) |

也就是说：**Gmail 的 IMAP 还能用，但认证方式被锁死了。** 而 163/QQ/沃邮箱只需要一个
16 位授权码，难度天差地别。

Gmail 的 OAuth 又有额外约束：

- 每个平台必须**单独注册**一个 OAuth Client（Windows 安装版 / Android / iOS / Web）。
- 桌面应用必须走 **Loopback** 或 **Device Code** 流程。
- 应用处于 **Testing**（未验证）状态时：refresh token **会过期**（官方文档为 7 天），
  且**限 100 个测试用户** → 未验证的应用无法长期稳定使用。
- 敏感 scope（`https://mail.google.com/`）需提交**验证审核**：
  要求域名网站、隐私政策页、同意屏品牌化，审核周期 1~3 周。

这意味着：**Gmail 支持的可用性不由代码决定，而由外部审核流程决定。**

---

## 决策

**走正式 OAuth 2.0 验证流程（Device Code + XOAUTH2），且在 P1 的第 1 周就启动申请。**

配套措施：

1. **P1 第 1 周**：注册 Google Cloud 项目 → 创建三类 OAuth 客户端 → 提交验证审核。
2. **开发期兜底**：开启两步验证的个人账号用「应用专用密码」走基础认证，不阻塞开发。
3. **产品内兜底**：设置里提供"使用自己的 OAuth Client ID"入口。
4. **Client ID/Secret 存放**：放 `oauth/clients.json`，**不入 Git**，走环境变量 / CI Secret；
   正式发布时内置进构建产物。
5. **Token 过期处理**：refresh token 过期时前端弹"重新授权"，走 Device Code 流程
   （可在手机上完成，无需在电脑上做本地回调）。

---

## 理由

- **只有正式验证能长期使用**：Testing 状态的 7 天 token 过期 + 100 用户上限，
  不满足"正式发布"的要求。
- **审核周期是外部依赖，越早启动越好**：1~3 周的等待必须与开发并行，不能串行。
- **Device Code 流程最适合多端**：Windows/Android 都能在手机浏览器上完成授权，
  不需要为每个平台调试本地 HTTP 回调端口。
- **兜底路径让开发不被卡住**：应用专用密码足以支撑 P1 的开发与自测。

---

## 后果

### 正面

- Gmail 是长期可用的正规通道，不依赖灰色手段。
- 面向普通用户发布时体验合规（用户看到的是品牌化的同意屏）。
- 审核通过后 refresh token 不再短期过期。

### 负面 / 代价

- **Google Cloud 项目 + 验证审核是一次性投入**：域名、隐私政策页、同意屏文案、
  演示视频（若要求）都要准备。
- **多平台要注册多个 Client**，构建流程需要按平台注入不同的 Client ID。
- **OAuth 状态机是额外的复杂度**：access token 1 小时过期、refresh token 可能被用户
  在 Google 账号设置里撤销、需要处理 `invalid_grant` / `unauthorized_client` 等错误。
- **"应用专用密码"兜底不是正式方案**，必须在 UI 里说明仅供高级用户。

### 风险与缓解

| 风险 | 缓解 |
|---|---|
| 验证审核被拒（品牌/网站不达标） | 提前准备：真实域名、完整隐私政策、清楚的用途说明、录屏演示；预留 2 轮被拒的时间 |
| 审核周期超出预期 | P1 期间用应用专用密码开发 Gmail 相关功能，审核通过后仅换认证方式 |
| refresh token 被用户撤销 | 明确错误提示 + 一键重新授权（Device Code） |
| 多平台 Client ID 混淆 | 构建脚本按 target 注入；`oauth/clients.json` 用 target 段区分 |
| 依赖单一 provider 的政策变化 | 抽象 `AuthProvider` trait，JMAP/Gmail API 未来可替换 |

---

## 备选方案

### A. 只支持应用专用密码

- **优点**：零审核、开发最快。
- **否决理由**：仅限开启了 2FA 的**个人**账号，Workspace 完全不可用；
  且 Google 随时可能进一步收紧。**只能作为开发期兜底，不能作为正式方案。**

### B. 改用 Gmail API（REST）而非 IMAP

- **优点**：纯 HTTP，无需 OAuth2 SASL 扩展；功能更全（标签、线程、发送草稿）。
- **否决理由**：只解决 Gmail 一家，163/QQ/沃邮箱 仍必须走 IMAP → **两套协议引擎**，
  工作量翻倍，且同步模型需要抽象出第三种（JMAP/REST）形态。得不偿失。
  > 若未来要做 JMAP 支持，Google 也会提供 JMAP 端点，届时再评估。

### C. 只让用户自带 Client ID

- **优点**：完全合规、零审核负担、零密钥管理。
- **否决理由**：普通用户开箱即用的体验差（要自己去 Google Cloud 建项目），
  与"多端邮件客户端"的消费级定位不符。可作为**高级选项**保留。

### D. 用 Gmail API 但只做收发，不做同步

- **否决理由**：同 B，仍然是第二套协议引擎。

---

## 复审条件

- Google 宣布 IMAP/XOAUTH2 支持终止 → 立即启动 Gmail API / JMAP 方案评估。
- 验证审核连续两轮被拒且申诉无望 → 降级为"仅支持自带 Client ID + 应用专用密码"，
  并在产品内明示 Gmail 支持的局限。
- 用户量超过 100 人且应用进入 Production 状态 → 重新核对 quota 与敏感 scope 要求。
