import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { Account, Notice, SyncResult } from "../types";

interface Props {
  account: Account;
  onSynced: (result: SyncResult) => void;
  onError: (message: string) => void;
  onNotice: (notice: Notice) => void;
  /** 账户信息变了（凭据存进 / 删掉）之后重新拉一遍账户列表。 */
  onChanged: () => void;
}

/**
 * 同步面板：填授权码 → 同步一次。
 *
 * ## 凭据默认不落盘
 *
 * 授权码默认只在**这次同步**里用，用完即弃（只驻留内存）。
 * 但让用户每次开应用都重敲 16 位授权码太折磨人，所以给一个**显式**的
 * 入口：把它交给**操作系统的钥匙串**（Windows 的凭据管理器），
 * 以后就不用再输了。
 *
 * 默认仍然不存——把密钥写进应用自己的数据库是另一回事（SECURITY §2.1）。
 */
function SyncPanel({
  account,
  onSynced,
  onError,
  onNotice,
  onChanged,
}: Props) {
  const [credential, setCredential] = useState("");
  /**
   * 刚用过的那次授权码。
   *
   * 同步成功后输入框会被清空，但用户可能想把**刚才那个**码存进系统凭据库，
   * 所以另留一份在内存里；只有用户明确点了「保存」才会落到系统钥匙串。
   */
  const [lastCredential, setLastCredential] = useState("");
  const [busy, setBusy] = useState(false);
  const [saving, setSaving] = useState(false);

  const run = async () => {
    setBusy(true);
    try {
      const result = await invoke<SyncResult>("sync_account", {
        accountId: account.id,
        credential,
      });
      setLastCredential(credential);
      setCredential("");
      onSynced(result);
    } catch (e) {
      onError(String(e));
    } finally {
      setBusy(false);
    }
  };

  /**
   * 把授权码存进系统凭据库。
   *
   * 失败要把原因**如实**说出来（比如非 Windows 平台会报
   * 「当前平台暂不支持保存凭据」）——假装存好了，用户下次还得重填，
   * 还会以为是应用 bug。
   */
  const save = async () => {
    const value = credential.trim() !== "" ? credential.trim() : lastCredential;
    if (value === "") {
      return;
    }
    setSaving(true);
    try {
      await invoke<void>("save_credential", {
        accountId: account.id,
        credential: value,
      });
      // 存完了就别让明文继续在内存里待着。
      setCredential("");
      setLastCredential("");
      onNotice({
        kind: "ok",
        text: "授权码已保存到系统凭据库，以后不用再输",
      });
      onChanged();
    } catch (e) {
      onNotice({ kind: "error", text: `保存授权码失败：${String(e)}` });
    } finally {
      setSaving(false);
    }
  };

  const canSave = credential.trim() !== "" || lastCredential !== "";

  return (
    <div className="sync-panel">
      <label className="sync-label" htmlFor={`cred-${account.id}`}>
        IMAP 授权码（默认只用于这次同步，不落盘）
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
      <div className="sync-actions">
        <button
          type="button"
          className="account-sync"
          disabled={busy || credential.length === 0}
          onClick={() => void run()}
        >
          {busy ? "同步中…" : "同步"}
        </button>
        <button
          type="button"
          className="account-save-credential"
          disabled={busy || saving || !canSave}
          onClick={() => void save()}
        >
          {saving
            ? "保存中…"
            : "保存授权码到系统凭据库（以后不用再输）"}
        </button>
      </div>
      {lastCredential !== "" && credential.trim() === "" && (
        <span className="sync-hint">
          刚用这个授权码同步成功——要存进系统凭据库吗？存了以后就不用再输。
        </span>
      )}
    </div>
  );
}

export default SyncPanel;
