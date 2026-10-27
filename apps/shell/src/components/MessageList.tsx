import type { Message } from "../types";
import { isStarred, isUnread } from "../types";

interface Props {
  messages: Message[];
  selectedUid: number | null;
  onSelect: (message: Message) => void;
}

/** 邮件列表。 */
function MessageList({ messages, selectedUid, onSelect }: Props) {
  if (messages.length === 0) {
    return (
      <section className="panel">
        <h2 className="panel-title">邮件</h2>
        <p className="panel-empty">这个文件夹里没有邮件。</p>
      </section>
    );
  }

  // 新的在上。
  const ordered = [...messages].sort((a, b) => b.uid - a.uid);

  return (
    <section className="panel">
      <h2 className="panel-title">邮件（{ordered.length}）</h2>
      <ul className="message-list">
        {ordered.map((message) => (
          <li key={message.id}>
            <button
              type="button"
              className={
                message.uid === selectedUid
                  ? "message-item message-item-active"
                  : "message-item"
              }
              onClick={() => onSelect(message)}
            >
              <span
                className={
                  isUnread(message)
                    ? "message-subject message-unread"
                    : "message-subject"
                }
              >
                {isStarred(message) && <span className="message-star">★</span>}
                {message.subject || "（无主题）"}
              </span>
              <span className="message-detail">
                {message.from[0] || "（未知发件人）"}
                {message.hasAttachment && " · 附件"}
              </span>
            </button>
          </li>
        ))}
      </ul>
    </section>
  );
}

export default MessageList;
