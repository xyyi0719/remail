# ReMail · 应用外壳

Tauri 2 + React 19 + TypeScript 的前端与 Rust 外壳。**桌面与移动共用这一份工程**。

```
src/                React 前端（仅渲染，无业务逻辑）
src-tauri/          Rust 外壳：Command / Event / 平台集成
public/favicon.svg  应用图标
```

## 命令

```bash
npm install            # 首次
npm run dev            # 仅前端（Vite，端口 1420）
npm run tauri dev      # 完整应用（桌面）
npm run tauri build    # 打包桌面版
npm run typecheck      # TypeScript 类型检查
npm run android:init   # 生成 Android 工程（需 Android SDK/NDK，只做一次）
npm run android:dev    # 在 Android 设备/模拟器上运行
```

## 约束

- 前端**不直接**访问数据库、不处理协议、不碰凭据。
- 所有数据经 Tauri Command 获取，实时更新靠 `AppEvent` 事件订阅。
- HTML 邮件必须在 sandbox iframe 中渲染（见 [SECURITY §3](../../docs/SECURITY.md)）。
- `tauri.conf.json` 的 `app.security.csp` 不得设为 `null`。

详见 [架构文档](../../docs/ARCHITECTURE.md) 与 [贡献指南](../../docs/CONTRIBUTING.md)。
