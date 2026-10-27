import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { Account, SyncResult } from "../types";

interface Props {
  account: Account;
  onSynced: (result: SyncResult) => void;
  onError: (message: string) => void;
}

/**
 * 同步面板：填授权码 → 同步一次。
 *
 * ## 凭据为什么不落盘
 *
 * 授权码只在**这次同步**里用，用完即弃（只驻留内存）。
 * 落盘凭据要靠平台钥匙串，那是 P4 的事（见 SECURITY §2.1）。
 * 所以关掉应用就得重填——这是有意的取舍。
 */
function SyncPanel({ account, onSynced, onError }: Props) {
  const [credential, setCredential] = useState("");
  const [busy, setBusy] = useState(false);

  const run = async () => {
    setBusy(true);
    try {
      const result = await invoke<SyncResult>("sync_account", {
        accountId: account.id,
        credential,
      });
      setCredential("");
      onSynced(result);
    } catch (e) {
      onError(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="sync-panel">
      <label className="sync-label" htmlFor={`cred-${account.id}`}>
        IMAP 授权码（只用于这次同步，不落盘）
      </label>
      <input
        id={`cred-${account.id}`}
        type="password"
        className="sync-input"
        value={credential}
        autoComplete="off"
        onChange={(e) => setCredential(e.target.value)}
        placeholder="16 位客户端授权码"
      />
      <button
        type="button"
        className="account-sync"
        disabled={busy || credential.length === 0}
        onClick={() => void run()}
      >
        {busy ? "同步中…" : "同步"}
      </button>
    </div>
  );
}

export default SyncPanel;
