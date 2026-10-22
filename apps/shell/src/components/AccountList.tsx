import type { Account } from "../types";
import { PROVIDER_LABELS, SYNC_MODE_LABELS, TLS_LABELS } from "../types";

interface Props {
  accounts: Account[];
  onDelete: (id: string) => void;
}

/** 账户列表。数据来自 SQLite，不是硬编码。 */
function AccountList({ accounts, onDelete }: Props) {
  if (accounts.length === 0) {
    return (
      <section className="panel">
        <h2 className="panel-title">账户</h2>
        <p className="panel-empty">还没有账户。用上面的表单添加一个。</p>
      </section>
    );
  }

  return (
    <section className="panel">
      <h2 className="panel-title">账户（{accounts.length}）</h2>
      <ul className="account-list">
        {accounts.map((account) => (
          <li key={account.id} className="account-item">
            <div className="account-main">
              <span className="account-email">{account.email}</span>
              <span className="account-tag">
                {PROVIDER_LABELS[account.provider] ?? account.provider}
              </span>
              {account.displayName && (
                <span className="account-tag account-tag-muted">
                  {account.displayName}
                </span>
              )}
            </div>
            <div className="account-detail">
              IMAP {account.server.imapHost}:{account.server.imapPort}（
              {TLS_LABELS[account.server.imapTls]}） · SMTP{" "}
              {account.server.smtpHost}:{account.server.smtpPort}（
              {TLS_LABELS[account.server.smtpTls]}） ·{" "}
              {SYNC_MODE_LABELS[account.syncMode]}
            </div>
            <div className="account-detail account-detail-muted">
              用户名 {account.username}
              {account.secretRef === null && " · 凭据未接入"}
            </div>
            <button
              type="button"
              className="account-delete"
              onClick={() => onDelete(account.id)}
            >
              删除
            </button>
          </li>
        ))}
      </ul>
    </section>
  );
}

export default AccountList;
