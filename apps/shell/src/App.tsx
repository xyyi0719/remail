import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import "./App.css";

interface AppInfo {
  name: string;
  version: string;
  platform: string;
  dataDir: string | null;
}

/**
 * P0 占位页：验证 Rust ↔ 前端链路是否打通。
 * 真正的邮件列表界面在 P1/P2 实现。
 */
function App() {
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    invoke<AppInfo>("app_info")
      .then(setInfo)
      .catch((e) => setError(String(e)));
  }, []);

  return (
    <main className="app-shell">
      <h1>Xremail</h1>
      <p className="app-subtitle">多端多账号邮件客户端 · P0 骨架</p>

      {error && <p className="app-error">无法连接 Rust 后端：{error}</p>}

      {info ? (
        <dl className="app-info">
          <dt>产品</dt>
          <dd>{info.name}</dd>
          <dt>版本</dt>
          <dd>{info.version}</dd>
          <dt>平台</dt>
          <dd>{info.platform}</dd>
        </dl>
      ) : (
        !error && <p className="app-loading">加载中…</p>
      )}

      <p className="app-note">
        下一阶段：接入 <code>remail-store</code> 与 IMAP 协议层。
      </p>
    </main>
  );
}

export default App;
