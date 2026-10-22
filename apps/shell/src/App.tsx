import AccountForm from "./components/AccountForm";
import AccountList from "./components/AccountList";
import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { Account, AppInfo } from "./types";
import "./App.css";

/**
 * 账户相关的事件名。
 *
 * 事件是**刷新提示**，不是数据来源（见 ARCHITECTURE §1.6）：
 * 收到后重新查询，漏事件也能靠重查恢复。
 */
const ACCOUNT_EVENTS = [
  "account://added",
  "account://updated",
  "account://removed",
] as const;

function App() {
  const [accounts, setAccounts] = useState<Account[] | null>(null);
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      setAccounts(await invoke<Account[]>("list_accounts"));
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  }, []);

  useEffect(() => {
    void refresh();
    void invoke<AppInfo>("app_info")
      .then(setInfo)
      .catch((e) => setError(String(e)));
  }, [refresh]);

  useEffect(() => {
    let disposed = false;
    const unlisteners: UnlistenFn[] = [];

    for (const name of ACCOUNT_EVENTS) {
      void listen(name, () => {
        void refresh();
      }).then((unlisten) => {
        if (disposed) {
          unlisten();
        } else {
          unlisteners.push(unlisten);
        }
      });
    }

    return () => {
      disposed = true;
      for (const unlisten of unlisteners) {
        unlisten();
      }
    };
  }, [refresh]);

  const handleDelete = useCallback(
    async (id: string) => {
      try {
        await invoke<boolean>("delete_account", { id });
        // 事件会触发刷新；这里也直接刷一次，事件丢了也不影响结果。
        await refresh();
      } catch (e) {
        setError(String(e));
      }
    },
    [refresh],
  );

  return (
    <main className="app-shell">
      <header className="app-header">
        <div>
          <h1>Xremail</h1>
          <p className="app-subtitle">多端多账号邮件客户端 · P0 骨架</p>
        </div>
        {info && (
          <dl className="app-info">
            <dt>版本</dt>
            <dd>{info.version}</dd>
            <dt>平台</dt>
            <dd>{info.platform}</dd>
            <dt>数据目录</dt>
            <dd className="app-info-path" title={info.dataDir ?? ""}>
              {info.dataDir ?? "—"}
            </dd>
          </dl>
        )}
      </header>

      {error && <p className="app-error">操作失败：{error}</p>}

      <AccountForm />

      {accounts === null ? (
        <p className="app-loading">加载中…</p>
      ) : (
        <AccountList accounts={accounts} onDelete={handleDelete} />
      )}
    </main>
  );
}

export default App;
