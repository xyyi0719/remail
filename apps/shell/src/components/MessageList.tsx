import { useState } from "react";
import type { Message } from "../types";
import { isStarred, isUnread } from "../types";

interface Props {
  messages: Message[];
  selectedUid: number | null;
  onSelect: (message: Message) => void;
  /**
   * 切换星标。
   *
   * 返回 Promise 是为了让列表知道请求还在路上——连点两次会把
   * 「加星 / 取消星」互相抵消，所以请求没回来之前按钮是禁用的。
   */
  onToggleStar: (message: Message) => void | Promise<void>;
}

/** 邮件列表。 */
function MessageList({
  messages,
  selectedUid,
  onSelect,
  onToggleStar,
}: Props) {
  // 正在提交星标意图的那封邮件（uid）；同一时刻只允许一个。
  const [pendingUid, setPendingUid] = useState<number | null>(null);

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

  const toggleStar = async (message: Message) => {
    setPendingUid(message.uid);
    try {
      await onToggleStar(message);
    } finally {
      setPendingUid(null);
    }
  };

  return (
    <section className="panel">
      <h2 className="panel-title">邮件（{ordered.length}）</h2>
      <ul className="message-list">
        {ordered.map((message) => {
          const starred = isStarred(message);
          return (
            <li key={message.id} className="message-row">
              <button
                type="button"
                className={
                  message.uid === selectedUid
                    ? "message-item message-item-active"
                    : "message-item"
                }
                onClick={() => onSelect(message)}
                data-testid="message-item"
              >
                <span
                  className={
                    isUnread(message)
                      ? "message-subject message-unread"
                      : "message-subject"
                  }
                >
                  {message.subject || "（无主题）"}
                </span>
                <span className="message-detail">
                  {message.from[0] || "（未知发件人）"}
                  {message.hasAttachment && " · 附件"}
                </span>
              </button>
              {/* 星标按钮放在行尾而不是行首：行首是「选哪封」的地方，
                  星标是「对这一封做什么」，分开更不容易误点。 */}
              <button
                type="button"
                className={
                  starred
                    ? "message-star-toggle message-star-toggle-on"
                    : "message-star-toggle"
                }
                onClick={() => void toggleStar(message)}
                disabled={pendingUid === message.uid}
                aria-pressed={starred}
                aria-label={starred ? "取消星标" : "加星标"}
                title={starred ? "取消星标" : "加星标"}
              >
                {starred ? (
                  <span className="message-star">★</span>
                ) : (
                  <span aria-hidden="true">☆</span>
                )}
              </button>
            </li>
          );
        })}
      </ul>
    </section>
  );
}

export default MessageList;
