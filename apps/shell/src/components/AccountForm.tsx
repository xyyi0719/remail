import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type {
  NewAccountRequest,
  ProviderKind,
  SmtpAuth,
  SyncMode,
  TlsMode,
} from "../types";
import {
  PROVIDER_LABELS,
  SMTP_AUTH_LABELS,
  SYNC_MODE_LABELS,
  TLS_LABELS,
} from "../types";

/** 表单的初始值。 */
const EMPTY: NewAccountRequest = {
  email: "",
  provider: "netease_163",
  displayName: "",
  imapHost: "",
  imapPort: 993,
  imapTls: "implicit",
  smtpHost: "",
  smtpPort: 465,
  smtpTls: "implicit",
  smtpAuth: "login",
  username: "",
  syncMode: "poll",
  pollIntervalSecs: 300,
};

/**
 * 新增账户表单。
 *
 * **服务器地址不按品牌自动填**：各服务商的 IMAP/SMTP 参数要实测确认
 * （见 ISSUES_AND_RISKS R01/R02），在实测之前宁可让用户手填，
 * 也不能拿一个看起来合理的默认值糊弄过去。
 */
function AccountForm() {
  const [form, setForm] = useState<NewAccountRequest>(EMPTY);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [done, setDone] = useState<string | null>(null);

  const set = <K extends keyof NewAccountRequest>(
    key: K,
    value: NewAccountRequest[K],
  ) => {
    setForm((prev) => ({ ...prev, [key]: value }));
  };

  const handleSubmit = async (event: React.FormEvent) => {
    event.preventDefault();
    setBusy(true);
    setError(null);
    setDone(null);

    try {
      await invoke("create_account", { request: form });
      setForm(EMPTY);
      setDone("已添加。凭据尚未接入，暂不能收发邮件。");
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <section className="panel">
      <h2 className="panel-title">添加账户</h2>
      <p className="panel-hint">
        服务商只是标记，服务器地址需要自行填写——各服务商的参数要实测确认，
        暂不按品牌推定。
      </p>

      <form className="account-form" onSubmit={handleSubmit}>
        <label className="field">
          <span>邮箱</span>
          <input
            type="email"
            value={form.email}
            onChange={(e) => set("email", e.target.value)}
            placeholder="user@example.com"
            required
          />
        </label>

        <label className="field">
          <span>服务商</span>
          <select
            value={form.provider}
            onChange={(e) => set("provider", e.target.value as ProviderKind)}
          >
            {(Object.keys(PROVIDER_LABELS) as ProviderKind[]).map((key) => (
              <option key={key} value={key}>
                {PROVIDER_LABELS[key]}
              </option>
            ))}
          </select>
        </label>

        <label className="field">
          <span>显示名（可选）</span>
          <input
            type="text"
            value={form.displayName ?? ""}
            onChange={(e) => set("displayName", e.target.value)}
          />
        </label>

        <label className="field">
          <span>用户名</span>
          <input
            type="text"
            value={form.username}
            onChange={(e) => set("username", e.target.value)}
            placeholder="通常是完整邮箱地址"
            required
          />
        </label>

        <label className="field">
          <span>IMAP 服务器</span>
          <input
            type="text"
            value={form.imapHost}
            onChange={(e) => set("imapHost", e.target.value)}
            placeholder="imap.example.com"
            required
          />
        </label>

        <label className="field field-narrow">
          <span>IMAP 端口</span>
          <input
            type="number"
            value={form.imapPort}
            onChange={(e) => set("imapPort", Number(e.target.value))}
            min={1}
            max={65535}
            required
          />
        </label>

        <label className="field field-narrow">
          <span>IMAP 加密</span>
          <select
            value={form.imapTls}
            onChange={(e) => set("imapTls", e.target.value as TlsMode)}
          >
            {(Object.keys(TLS_LABELS) as TlsMode[]).map((key) => (
              <option key={key} value={key}>
                {TLS_LABELS[key]}
              </option>
            ))}
          </select>
        </label>

        <label className="field">
          <span>SMTP 服务器</span>
          <input
            type="text"
            value={form.smtpHost}
            onChange={(e) => set("smtpHost", e.target.value)}
            placeholder="smtp.example.com"
            required
          />
        </label>

        <label className="field field-narrow">
          <span>SMTP 端口</span>
          <input
            type="number"
            value={form.smtpPort}
            onChange={(e) => set("smtpPort", Number(e.target.value))}
            min={1}
            max={65535}
            required
          />
        </label>

        <label className="field field-narrow">
          <span>SMTP 加密</span>
          <select
            value={form.smtpTls}
            onChange={(e) => set("smtpTls", e.target.value as TlsMode)}
          >
            {(Object.keys(TLS_LABELS) as TlsMode[]).map((key) => (
              <option key={key} value={key}>
                {TLS_LABELS[key]}
              </option>
            ))}
          </select>
        </label>

        <label className="field field-narrow">
          <span>SMTP 认证</span>
          <select
            value={form.smtpAuth}
            onChange={(e) => set("smtpAuth", e.target.value as SmtpAuth)}
          >
            {(Object.keys(SMTP_AUTH_LABELS) as SmtpAuth[]).map((key) => (
              <option key={key} value={key}>
                {SMTP_AUTH_LABELS[key]}
              </option>
            ))}
          </select>
        </label>

        <label className="field field-narrow">
          <span>同步方式</span>
          <select
            value={form.syncMode}
            onChange={(e) => set("syncMode", e.target.value as SyncMode)}
          >
            {(Object.keys(SYNC_MODE_LABELS) as SyncMode[]).map((key) => (
              <option key={key} value={key}>
                {SYNC_MODE_LABELS[key]}
              </option>
            ))}
          </select>
        </label>

        <label className="field field-narrow">
          <span>轮询间隔（秒）</span>
          <input
            type="number"
            value={form.pollIntervalSecs}
            onChange={(e) => set("pollIntervalSecs", Number(e.target.value))}
            min={1}
            required
          />
        </label>

        <div className="form-actions">
          <button type="submit" disabled={busy}>
            {busy ? "保存中…" : "添加账户"}
          </button>
        </div>
      </form>

      {error && <p className="app-error">{error}</p>}
      {done && <p className="app-ok">{done}</p>}
    </section>
  );
}

export default AccountForm;
