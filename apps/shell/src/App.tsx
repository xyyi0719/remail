import AccountForm from "./components/AccountForm";
import AccountList from "./components/AccountList";
import FolderList from "./components/FolderList";
import MessageList from "./components/MessageList";
import MessageView from "./components/MessageView";
import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  Account,
  AppInfo,
  Folder,
  Message,
  SyncResult,
} from "./types";
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

  // 当前浏览的账户 / 文件夹 / 邮件。
  const [accountId, setAccountId] = useState<string | null>(null);
  const [folders, setFolders] = useState<Folder[]>([]);
  const [folderId, setFolderId] = useState<string | null>(null);
  const [messages, setMessages] = useState<Message[]>([]);
  const [message, setMessage] = useState<Message | null>(null);

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
      })
        .then((unlisten) => {
          if (disposed) {
            unlisten();
          } else {
            unlisteners.push(unlisten);
          }
        })
        .catch((e) => setError(String(e)));
    }

    return () => {
      disposed = true;
      for (const unlisten of unlisteners) {
        unlisten();
      }
    };
  }, [refresh]);

  // 切换账户 → 重新拉文件夹。
  useEffect(() => {
    if (!accountId) {
      setFolders([]);
      setFolderId(null);
      setMessages([]);
      setMessage(null);
      return;
    }
    void invoke<Folder[]>("list_folders", { accountId })
      .then(setFolders)
      .catch((e) => setError(String(e)));
  }, [accountId]);

  // 切换文件夹 → 重新拉邮件。
  useEffect(() => {
    if (!accountId || !folderId) {
      setMessages([]);
      setMessage(null);
      return;
    }
    void invoke<Message[]>("list_messages", { accountId, folderId })
      .then(setMessages)
      .catch((e) => setError(String(e)));
  }, [accountId, folderId]);

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

  const handleSynced = useCallback(
    async (_result: SyncResult) => {
      // 同步完成后刷新文件夹与邮件。
      if (accountId) {
        const next = await invoke<Folder[]>("list_folders", { accountId });
        setFolders(next);
        if (folderId) {
          setMessages(await invoke<Message[]>("list_messages", { accountId, folderId }));
        }
      }
    },
    [accountId, folderId],
  );

  const handleError = useCallback((message: string) => setError(message), []);

  return (
    <main className="app-shell">
      <header className="app-header">
        <div>
          <h1 className="app-title">Xremail</h1>
          <p className="app-subtitle">本地邮件客户端</p>
        </div>
        {info && (
          <dl className="app-info">
            <dt>版本</dt>
            <dd>
              {info.name} {info.version}（{info.platform}）
            </dd>
            <dt>数据目录</dt>
            <dd className="app-info-path">{info.dataDir ?? "—"}</dd>
          </dl>
        )}
      </header>

      {error && <p className="app-error">操作失败：{error}</p>}

      <AccountForm />

      {accounts === null ? (
        <p className="app-loading">加载中…</p>
      ) : (
        <>
          <AccountList
            accounts={accounts}
            onSelect={(account) => setAccountId(account.id)}
            onDelete={handleDelete}
            onSynced={handleSynced}
            onError={handleError}
          />

          {accountId && (
            <>
              <FolderList
                folders={folders}
                selectedId={folderId}
                onSelect={(folder) => setFolderId(folder.id)}
              />

              {folderId && (
                <>
                  <MessageList
                    messages={messages}
                    selectedUid={message?.uid ?? null}
                    onSelect={setMessage}
                  />

                  {message && (
                    <MessageView
                      accountId={accountId}
                      folderId={folderId}
                      message={message}
                      onError={handleError}
                    />
                  )}
                </>
              )}
            </>
          )}
        </>
      )}
    </main>
  );
}

export default App;
