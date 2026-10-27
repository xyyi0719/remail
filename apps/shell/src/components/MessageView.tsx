import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { Message, MessageBody } from "../types";

interface Props {
  accountId: string;
  folderId: string;
  message: Message;
  onError: (message: string) => void;
}

/**
 * 邮件正文。
 *
 * ## 为什么 HTML 只显示源码
 *
 * 邮件的 HTML **未经清洗**（ARCHITECTURE §7 规定的清洗器还没做，
 * 见 ISSUES_AND_RISKS R06）。直接渲染等于给邮件内容一个 XSS 口子，
 * 所以这里**不渲染**，只把源码显示出来。
 *
 * 真实情况：国内服务商的邮件**大多是 HTML-only**（没有 text/plain），
 * 实测 163 / QQ / 沃三家的抽样邮件纯文本都是 0 字。所以这条路径不是边角情况。
 */
function MessageView({ accountId, folderId, message, onError }: Props) {
  const [body, setBody] = useState<MessageBody | null>(null);
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    let disposed = false;
    setBody(null);
    setLoading(true);

    void invoke<MessageBody | null>("open_message", {
      accountId,
      folderId,
      uid: message.uid,
    })
      .then((result) => {
        if (!disposed) {
          setBody(result);
        }
      })
      .catch((e) => {
        if (!disposed) {
          onError(String(e));
        }
      })
      .finally(() => {
        if (!disposed) {
          setLoading(false);
        }
      });

    return () => {
      disposed = true;
    };
  }, [accountId, folderId, message.uid, onError]);

  const plain = body?.textPlain ?? null;
  const html = body?.htmlRaw ?? null;

  return (
    <section className="panel">
      <h2 className="panel-title">{message.subject || "（无主题）"}</h2>
      <p className="message-detail">
        {message.from.join(", ") || "（未知发件人）"}
        {message.to.length > 0 && ` → ${message.to.join(", ")}`}
      </p>

      {loading && <p className="panel-empty">正在取正文…</p>}

      {!loading && plain && <pre className="message-body">{plain}</pre>}

      {!loading && !plain && html && (
        <>
          <p className="panel-empty">
            这封邮件只有 HTML 版本（纯文本为空）。HTML 清洗器还没做，
            所以下面显示的是**源码**，不是渲染结果。
          </p>
          <pre className="message-body">{html}</pre>
        </>
      )}

      {!loading && !plain && !html && (
        <p className="panel-empty">这封邮件没有正文。</p>
      )}
    </section>
  );
}

export default MessageView;
