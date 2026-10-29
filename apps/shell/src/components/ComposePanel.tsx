import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { DraftDto, DraftInput, SendResult } from "../types";

interface Props {
  /** 用哪个账户发信（写进草稿的 accountId）。 */
  accountId: string;
  /** 账户邮箱，只用来显示给用户看。 */
  accountEmail: string;
  /** 请求关闭面板（父组件负责卸载）。 */
  onClose: () => void;
}

/** 一个附件（后端按 UTF-8 字节数组收，再转 base64 存库）。 */
interface Attachment {
  filename: string;
  contentType: string;
  bytes: number[];
}

/** 面板里维护的表单内容。 */
interface ComposeForm {
  /** 收件人（To）。 */
  to: string;
  /** 抄送（Cc）。 */
  cc: string;
  /** 密送（Bcc）。 */
  bcc: string;
  subject: string;
  bodyText: string;
  attachments: Attachment[];
}

/** 发送结果的界面说法。 */
interface SendOutcome {
  kind: "sent" | "uncertain" | "failed";
  /** 主文案。 */
  text: string;
  /** 后端给的补充说明。 */
  detail: string | null;
  /** 服务器给的 Message-ID。 */
  messageId: string | null;
}

/** 自动存草稿的防抖间隔（毫秒）。 */
const AUTOSAVE_DELAY_MS = 800;

/** 空表单。 */
const EMPTY_FORM: ComposeForm = {
  to: "",
  cc: "",
  bcc: "",
  subject: "",
  bodyText: "",
  attachments: [],
};

/**
 * 收件人分隔符。
 *
 * 逗号（中英文）、分号（中英文）、空白都能分隔多个地址——
 * 邮箱地址本身不含这些字符。
 */
const ADDRESS_SPLIT = /[,;，；\s]+/;

/** 把一行收件人文本拆成地址数组（去掉空项）。 */
function parseAddresses(text: string): string[] {
  return text
    .split(ADDRESS_SPLIT)
    .map((one) => one.trim())
    .filter((one) => one !== "");
}

/** 表单是不是完全空的（空草稿不值得往库里写）。 */
function isEmptyForm(form: ComposeForm): boolean {
  return (
    form.to.trim() === "" &&
    form.cc.trim() === "" &&
    form.bcc.trim() === "" &&
    form.subject.trim() === "" &&
    form.bodyText.trim() === "" &&
    form.attachments.length === 0
  );
}

/** 按契约拼出发给后端的草稿结构。 */
function buildDraft(
  accountId: string,
  form: ComposeForm,
  id: string | null,
): DraftInput {
  return {
    accountId,
    id,
    to: parseAddresses(form.to),
    cc: parseAddresses(form.cc),
    bcc: parseAddresses(form.bcc),
    subject: form.subject,
    bodyText: form.bodyText,
    // 前端还没有富文本编辑器，HTML 正文暂时是空的。
    bodyHtml: null,
    attachments: form.attachments,
  };
}

/**
 * 把发送结果翻译成界面上的说法。
 *
 * 三种状态必须分开说：`uncertain`（服务器没确认）**不是**「失败」——
 * 重发一封可能已经发出去的邮件，就是给用户发了两封信。
 */
function describeSendResult(result: SendResult): SendOutcome {
  switch (result.state) {
    case "sent":
      return {
        kind: "sent",
        text: "已发送",
        detail: result.detail || null,
        messageId: result.messageId,
      };
    case "uncertain":
      return {
        kind: "uncertain",
        text: "服务器没有确认结果，这封信可能已经发出去了，不要重复发送",
        detail: result.detail || null,
        messageId: result.messageId,
      };
    case "failed":
      return {
        kind: "failed",
        text: "发送失败",
        detail: result.detail || null,
        messageId: result.messageId,
      };
    default:
      // 后端加了新状态而前端没跟上：别静默吞掉。
      return {
        kind: "failed",
        text: "发送失败：后端返回了不认识的状态",
        detail: null,
        messageId: null,
      };
  }
}

/** 时间戳格式化成 HH:MM:SS。 */
function formatTime(ts: number): string {
  const date = new Date(ts);
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${pad(date.getHours())}:${pad(date.getMinutes())}:${pad(date.getSeconds())}`;
}

/**
 * 读一个附件：文件名 + 类型 + UTF-8 字节数组（后端再转 base64）。
 *
 * 用 `FileReader` 而不是 `file.arrayBuffer()`：后者在部分运行环境
 * （包括测试用的 jsdom）里没有。
 */
function readAttachment(file: File): Promise<Attachment> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => {
      const buffer = reader.result as ArrayBuffer;
      resolve({
        filename: file.name,
        contentType: file.type || "application/octet-stream",
        bytes: Array.from(new Uint8Array(buffer)),
      });
    };
    reader.onerror = () => {
      reject(reader.error ?? new Error(`读取附件失败：${file.name}`));
    };
    reader.readAsArrayBuffer(file);
  });
}

/**
 * 写信面板。
 *
 * ## 自动存草稿
 *
 * 改动后等 800ms 再存一次（防抖）：每敲一个字就写一次库既吵又慢，
 * 而完全不等又会丢掉用户刚写的半句话。
 *
 * ## 三种发送结果
 *
 * `sent` / `uncertain` / `failed` 必须在界面上分开说，
 * 理由见 `types.ts` 里 `SendResult` 的说明——尤其是 `uncertain`：
 * 那是「不知道有没有发出去」，自动重发会让用户收到两封信。
 */
function ComposePanel({ accountId, accountEmail, onClose }: Props) {
  const [form, setForm] = useState<ComposeForm>(EMPTY_FORM);
  /** 密送栏默认收起来（它平时不该被看见）。 */
  const [showBcc, setShowBcc] = useState(false);
  /** 已经发出去了就不再自动存草稿。 */
  const [sent, setSent] = useState(false);
  const [savedAt, setSavedAt] = useState<string | null>(null);
  const [savingDraft, setSavingDraft] = useState(false);
  const [sending, setSending] = useState(false);
  const [outcome, setOutcome] = useState<SendOutcome | null>(null);
  const [formError, setFormError] = useState<string | null>(null);

  /**
   * 已经存过的草稿 id。
   *
   * 用 ref 而不是 state：它只是「下次存草稿要带上 id」，
   * 界面上不显示它，没必要为它重渲染一次。
   */
  const draftIdRef = useRef<string | null>(null);

  /**
   * 存一次草稿。
   *
   * 成功后把后端给的 id 记下来，后面的存草稿就变成「更新同一份草稿」
   * 而不是每次新建一份。
   */
  const persistDraft = useCallback(
    async (snapshot: ComposeForm) => {
      setSavingDraft(true);
      try {
        const dto = await invoke<DraftDto>("save_draft", {
          draft: buildDraft(accountId, snapshot, draftIdRef.current),
        });
        draftIdRef.current = dto.id;
        setSavedAt(formatTime(Date.now()));
        setFormError(null);
      } catch (e) {
        setFormError(`自动保存草稿失败：${String(e)}`);
      } finally {
        setSavingDraft(false);
      }
    },
    [accountId],
  );

  /**
   * 自动存草稿：改动后等 800ms。
   *
   * 空的表单不存（没什么可存的），已经发出去的也不存。
   * 依赖 `form` 的**引用**：每改一个字段就是新对象，
   * 于是计时器被重置——这正是防抖要的。
   */
  useEffect(() => {
    if (sent || isEmptyForm(form)) {
      return;
    }
    const timer = window.setTimeout(() => {
      void persistDraft(form);
    }, AUTOSAVE_DELAY_MS);
    return () => window.clearTimeout(timer);
  }, [form, sent, persistDraft]);

  const update = useCallback(
    <K extends keyof ComposeForm>(key: K, value: ComposeForm[K]) => {
      setForm((prev) => ({ ...prev, [key]: value }));
    },
    [],
  );

  /** 读文件系统的文件，加进附件列表。 */
  const handleFiles = useCallback(async (input: HTMLInputElement) => {
    const files = input.files;
    if (!files || files.length === 0) {
      return;
    }
    const loaded = await Promise.all(Array.from(files).map(readAttachment));
    setForm((prev) => ({ ...prev, attachments: [...prev.attachments, ...loaded] }));
    // 清空一下，好让用户能再次选择同一个文件。
    input.value = "";
  }, []);

  /** 移除单个附件。 */
  const removeAttachment = useCallback((index: number) => {
    setForm((prev) => ({
      ...prev,
      attachments: prev.attachments.filter((_, i) => i !== index),
    }));
  }, []);

  /**
   * 发送。
   *
   * 没写收件人就在前端拦下来：发出去也是白跑一趟，
   * 还会在服务器上留下一封没人收的信。
   */
  const handleSend = async () => {
    const draft = buildDraft(accountId, form, draftIdRef.current);
    if (draft.to.length + draft.cc.length + draft.bcc.length === 0) {
      setFormError("至少要一个收件人");
      setOutcome(null);
      return;
    }

    setSending(true);
    setFormError(null);
    setOutcome(null);
    try {
      const result = await invoke<SendResult>("send_message", { draft });
      setOutcome(describeSendResult(result));
      if (result.state === "sent") {
        // 发出去了就不再自动存草稿，也不用再提醒「没发送」。
        setSent(true);
        draftIdRef.current = null;
      }
      // `uncertain` 什么都不做：**不自动重发**。
      // 服务器没确认的时候重发，很可能让用户收到两封一样的信。
    } catch (e) {
      // 命令直接失败 = 明确没发出去，允许改完重试。
      setOutcome({
        kind: "failed",
        text: "发送失败",
        detail: String(e),
        messageId: null,
      });
    } finally {
      setSending(false);
    }
  };

  /** 还有没发出去的改动吗？ */
  const dirty = !isEmptyForm(form) && !sent;

  /** 关闭面板：没发出去的改动要先问一句。 */
  const handleClose = () => {
    if (dirty) {
      const ok = window.confirm("这封信还没有发送，确定要关闭吗？");
      if (!ok) {
        return;
      }
      // 临走前把当前内容存一次，别让用户白填。
      void persistDraft(form);
    }
    onClose();
  };

  /**
   * 发送按钮什么时候不让点。
   *
   * - 正在发：不许并发发第二封；
   * - 已经发出去了：不许再发一封一样的；
   * - 服务器没确认（`uncertain`）：**停用**。
   *   这封信可能已经发出去了，这时候点一下就是给用户发两封。
   */
  const sendBlocked = sending || sent || outcome?.kind === "uncertain";

  return (
    <section className="panel compose-panel" aria-label="写信面板">
      <div className="compose-head">
        <h2 className="panel-title">写邮件</h2>
        <button type="button" className="compose-close" onClick={handleClose}>
          关闭
        </button>
      </div>

      <p className="panel-hint">
        发信账户：{accountEmail}。改动会自动存草稿，正文是纯文本。
      </p>

      <div className="compose-fields">
        <label className="field">
          <span>收件人</span>
          <input
            type="text"
            value={form.to}
            onChange={(e) => update("to", e.target.value)}
            placeholder="多个地址用逗号分开"
          />
        </label>

        <label className="field">
          <span>抄送</span>
          <input
            type="text"
            value={form.cc}
            onChange={(e) => update("cc", e.target.value)}
            placeholder="多个地址用逗号分开"
          />
        </label>

        <button
          type="button"
          className="compose-bcc-toggle"
          aria-expanded={showBcc}
          onClick={() => setShowBcc((prev) => !prev)}
        >
          {showBcc ? "隐藏密送" : "显示密送"}
        </button>

        {showBcc && (
          <label className="field">
            <span>密送</span>
            <input
              type="text"
              value={form.bcc}
              onChange={(e) => update("bcc", e.target.value)}
              placeholder="多个地址用逗号分开"
            />
          </label>
        )}
      </div>

      <p className="compose-hint">
        密送（Bcc）收件人不会出现在收件人列表里——只有邮件头里能看到。
      </p>

      <div className="compose-fields">
        <label className="field">
          <span>主题</span>
          <input
            type="text"
            value={form.subject}
            onChange={(e) => update("subject", e.target.value)}
          />
        </label>

        <label className="field">
          <span>正文</span>
          <textarea
            className="compose-body"
            rows={8}
            value={form.bodyText}
            onChange={(e) => update("bodyText", e.target.value)}
          />
        </label>

        <label className="field">
          <span>附件</span>
          <input
            type="file"
            multiple
            className="compose-file"
            onChange={(e) => void handleFiles(e.target)}
          />
        </label>
      </div>

      {form.attachments.length > 0 && (
        <ul className="compose-attachments">
          {form.attachments.map((attachment, index) => (
            <li key={`${attachment.filename}-${index}`} className="compose-attachment">
              <span className="compose-attachment-name">{attachment.filename}</span>
              <button
                type="button"
                className="compose-attachment-remove"
                aria-label={`移除附件 ${attachment.filename}`}
                onClick={() => removeAttachment(index)}
              >
                移除
              </button>
            </li>
          ))}
        </ul>
      )}

      <div className="compose-actions">
        <span className="compose-saved" data-testid="compose-saved">
          {savedAt !== null
            ? `草稿已保存 ${savedAt}`
            : savingDraft
              ? "正在保存草稿…"
              : "改动后会自动存草稿"}
        </span>
        <button
          type="button"
          className="compose-send"
          disabled={sendBlocked}
          onClick={() => void handleSend()}
        >
          {sending ? "发送中…" : "发送"}
        </button>
      </div>

      {formError && (
        <p className="compose-error" role="alert" data-testid="compose-error">
          {formError}
        </p>
      )}

      {outcome && (
        <div
          className={`compose-result compose-result-${outcome.kind}`}
          role="status"
          data-testid="compose-send-result"
        >
          <strong>{outcome.text}</strong>
          {outcome.detail && (
            <span className="compose-result-detail">{outcome.detail}</span>
          )}
          {outcome.kind === "sent" && outcome.messageId && (
            <span className="compose-result-detail">
              Message-ID：{outcome.messageId}
            </span>
          )}
          {outcome.kind === "uncertain" && (
            <span className="compose-result-detail">
              发送按钮已停用：确认服务器上确实没有这封信之后，再重新写一封。
            </span>
          )}
          {outcome.kind === "failed" && (
            <span className="compose-result-detail">
              改完可以直接再点「发送」重试。
            </span>
          )}
        </div>
      )}
    </section>
  );
}

export default ComposePanel;
