/**
 * 极简富文本编辑器。
 *
 * ## 为什么自己写而不用现成的库
 *
 * 项目里**不许**引入新的前端依赖（除对话框插件外），而富文本库动辄
 * 几十 KB、还带一堆样式注入。所以这里用原生 `contentEditable` 加一条
 * 极简工具栏实现：加粗 / 斜体 / 下划线 / 无序列表 / 有序列表 / 链接。
 *
 * ## 安全边界在哪
 *
 * 后端会消毒 HTML（白名单：不允许 `img`、不允许 `style` / `on*`，
 * 链接只允许 `http/https/mailto`），但前端**也不能**当 sanitizer 用：
 *
 * - 编辑器里的内容会直接进 DOM，所以这里在**写入与读取两端**都过一遍
 *   `sanitizeHtml`；
 * - 粘贴一律按**纯文本**插入（`onPaste` 里 `preventDefault`），
 *   免得粘贴进来的 `onerror=` 之类的属性在编辑器里就执行了；
 * - 前端**不**用 `dangerouslySetInnerHTML` 渲染后端给的 HTML 做预览——
 *   预览只显示抽出来的纯文本。后端消毒过 ≠ 前端可以放心 innerHTML：
 *   一旦后端的白名单漏了一项，前端就会变成执行点。
 */
import { useEffect, useRef, useState } from "react";

/** 允许出现在正文里的标签。 */
const ALLOWED_TAGS = new Set([
  "P",
  "BR",
  "B",
  "STRONG",
  "I",
  "EM",
  "U",
  "S",
  "STRIKE",
  "UL",
  "OL",
  "LI",
  "DIV",
  "SPAN",
  "A",
]);

/** 连内容一起丢掉的标签：里面的东西不是正文，是脚本或样式。 */
const DROPPED_TAGS = new Set([
  "SCRIPT",
  "STYLE",
  "IFRAME",
  "OBJECT",
  "EMBED",
  "TEMPLATE",
  "NOSCRIPT",
]);

/** HTML 文本转义。 */
function escapeHtml(text: string): string {
  return text
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

/**
 * 链接地址是否安全。
 *
 * 只放行 `http/https/mailto`：`javascript:` 与 `data:` 会让收件人点一下
 * 就执行代码——这正是后端白名单要挡的东西，前端不能自己放宽。
 */
function safeHref(url: string): string | null {
  const trimmed = url.trim();
  if (trimmed === "") {
    return null;
  }
  return /^(https?:|mailto:)/i.test(trimmed) ? trimmed : null;
}

/**
 * 用户填的链接补上协议。
 *
 * 「www.example.com」这种没写协议的，补成 `https://`——
 * 否则会被当成相对路径，收件人点了打不开。
 */
function normalizeUrl(raw: string): string | null {
  const trimmed = raw.trim();
  if (trimmed === "") {
    return null;
  }
  const withScheme = /^[a-z][a-z0-9+.-]*:/i.test(trimmed)
    ? trimmed
    : `https://${trimmed}`;
  return safeHref(withScheme);
}

/** 把一个节点洗成白名单里的 HTML。 */
function sanitizeNode(node: Node): string {
  if (node.nodeType === 3 /* TEXT_NODE */) {
    return escapeHtml(node.textContent ?? "");
  }
  if (node.nodeType !== 1 /* ELEMENT_NODE */) {
    return "";
  }
  const element = node as Element;
  const tag = element.tagName.toUpperCase();
  if (DROPPED_TAGS.has(tag)) {
    return "";
  }
  const children = Array.from(element.childNodes)
    .map(sanitizeNode)
    .join("");
  if (!ALLOWED_TAGS.has(tag)) {
    // 不认识的标签：丢掉标签本身，留住里面的文字。
    return children;
  }
  if (tag === "BR") {
    return "<br>";
  }
  if (tag === "A") {
    const href = safeHref(element.getAttribute("href") ?? "");
    if (href === null) {
      return children;
    }
    return `<a href="${escapeHtml(href)}">${children}</a>`;
  }
  const lower = tag.toLowerCase();
  return `<${lower}>${children}</${lower}>`;
}

/**
 * 把一段 HTML 洗成白名单里的 HTML。
 *
 * 用 `DOMParser` 解析（解析出来的文档是**惰性**的：不会执行脚本、
 * 不会加载图片），再手工拼回字符串——所以不需要 `innerHTML` 去读。
 */
export function sanitizeHtml(html: string): string {
  if (html === "") {
    return "";
  }
  if (typeof DOMParser === "undefined") {
    // 没有 DOMParser 的环境（理论上不该有）：退化成纯文本。
    return escapeHtml(html);
  }
  const doc = new DOMParser().parseFromString(html, "text/html");
  return Array.from(doc.body.childNodes).map(sanitizeNode).join("");
}

/**
 * 从 HTML 里抽出纯文本。
 *
 * 为什么必须始终有一份纯文本：有些客户端（以及大多数网关）只读
 * `text/plain`，没有它正文就是空的。
 */
export function htmlToPlainText(html: string): string {
  if (html === "") {
    return "";
  }
  if (typeof DOMParser === "undefined") {
    return html;
  }
  const doc = new DOMParser().parseFromString(html, "text/html");
  const lines: string[] = [];
  let line = "";

  const flush = () => {
    const trimmed = line.trim();
    if (trimmed !== "") {
      lines.push(trimmed);
    }
    line = "";
  };

  const walk = (node: Node) => {
    for (const child of Array.from(node.childNodes)) {
      if (child.nodeType === 3 /* TEXT_NODE */) {
        line += child.textContent ?? "";
        continue;
      }
      if (child.nodeType !== 1 /* ELEMENT_NODE */) {
        continue;
      }
      const element = child as Element;
      const tag = element.tagName.toUpperCase();
      if (tag === "BR") {
        flush();
        continue;
      }
      if (tag === "UL" || tag === "OL") {
        const items = Array.from(element.children).filter(
          (child) => child.tagName.toUpperCase() === "LI",
        );
        items.forEach((item, index) => {
          line += tag === "OL" ? `${index + 1}. ` : "• ";
          walk(item);
          flush();
        });
        continue;
      }
      if (tag === "P" || tag === "DIV") {
        if (line.trim() !== "") {
          flush();
        }
        walk(element);
        if (line.trim() !== "") {
          flush();
        }
        continue;
      }
      walk(element);
    }
  };

  walk(doc.body);
  flush();
  return lines.join("\n");
}

/** 纯文本转成 HTML（每行一段）。 */
export function textToHtml(text: string): string {
  if (text === "") {
    return "";
  }
  return text
    .split("\n")
    .map((line) => `<p>${escapeHtml(line) === "" ? "<br>" : escapeHtml(line)}</p>`)
    .join("");
}

interface RichTextEditorProps {
  /** 编辑器里的 HTML（受控）。 */
  value: string;
  /** 内容变了（传出去的已经是消毒过的 HTML）。 */
  onChange: (html: string) => void;
}

/**
 * 富文本编辑器。
 *
 * ## React 与 contentEditable 的配合
 *
 * `contentEditable` 的内容**不能**交给 React 管：每次 setState 都会重写
 * DOM，光标就跳回开头，用户根本没法正常打字。
 *
 * 所以这里的做法是：
 *
 * - React 只管一个 `value` 字符串，DOM 由我们自己写；
 * - 只在「外面的值确实变了」时才写 DOM——自己刚写出去的值（`emittedRef`）
 *   直接跳过，这就是光标不跳的关键；
 * - 组件**不渲染任何子节点**（否则 React 会接管 children 并报警）。
 */
export default function RichTextEditor({ value, onChange }: RichTextEditorProps) {
  const editorRef = useRef<HTMLDivElement | null>(null);
  /** 自己最后一次交出去的 HTML——用来分辨「外面的值变了」与「只是我自己的回声」。 */
  const emittedRef = useRef<string | null>(null);
  const [linkOpen, setLinkOpen] = useState(false);
  const [linkUrl, setLinkUrl] = useState("");
  const [linkError, setLinkError] = useState<string | null>(null);

  /**
   * 把外面的值写进 DOM。
   *
   * 只在「不是自己刚写出去的」时写——否则每敲一个字 React 都会把
   * innerHTML 重写一遍，光标就跳了。
   */
  useEffect(() => {
    const node = editorRef.current;
    if (!node) {
      return;
    }
    if (emittedRef.current === value) {
      return;
    }
    const safe = sanitizeHtml(value);
    if (node.innerHTML !== safe) {
      node.innerHTML = safe;
    }
    emittedRef.current = safe;
  }, [value]);

  /** 读出编辑器里的 HTML（消毒后交给上层）。 */
  const emit = () => {
    const node = editorRef.current;
    if (!node) {
      return;
    }
    const html = sanitizeHtml(node.innerHTML);
    emittedRef.current = html;
    onChange(html);
  };

  /**
   * 执行一条编辑命令。
   *
   * `document.execCommand` 已经是废弃 API，但它是**唯一**不引入依赖
   * 就能做富文本编辑的办法：自己实现选区操作要处理跨节点选区、
   * undo 栈等一堆边界情况。
   */
  const runCommand = (command: string, argument?: string) => {
    const node = editorRef.current;
    if (!node) {
      return;
    }
    node.focus();
    if (typeof document.execCommand !== "function") {
      // 环境不支持（比如 jsdom）：内容不变，也不报错。
      return;
    }
    try {
      document.execCommand(command, false, argument);
    } catch {
      return;
    }
    emit();
  };

  /** 粘贴一律按纯文本插入。 */
  const handlePaste = (event: React.ClipboardEvent<HTMLDivElement>) => {
    const text = event.clipboardData?.getData("text/plain") ?? "";
    if (text === "") {
      return;
    }
    event.preventDefault();
    runCommand("insertText", text);
  };

  /** 拖放也拦掉：不然能拖进来一整张带 `onerror` 的图片。 */
  const handleDrop = (event: React.DragEvent<HTMLDivElement>) => {
    event.preventDefault();
  };

  const applyLink = () => {
    const url = normalizeUrl(linkUrl);
    if (url === null) {
      setLinkError("链接只支持 http、https 或 mailto 开头的地址");
      return;
    }
    runCommand("createLink", url);
    setLinkOpen(false);
    setLinkUrl("");
    setLinkError(null);
  };

  /**
   * 工具栏按钮的 `onMouseDown` 要吃掉默认行为：
   * 不然鼠标按下就把焦点从编辑区挪走，选区丢了，加粗就没作用了。
   */
  const keepSelection = (event: React.MouseEvent) => {
    event.preventDefault();
  };

  return (
    <div className="compose-rich">
      <div className="compose-rich-toolbar" role="toolbar" aria-label="富文本工具栏">
        <button
          type="button"
          className="compose-rich-tool"
          aria-label="加粗"
          onMouseDown={keepSelection}
          onClick={() => runCommand("bold")}
        >
          B
        </button>
        <button
          type="button"
          className="compose-rich-tool"
          aria-label="斜体"
          onMouseDown={keepSelection}
          onClick={() => runCommand("italic")}
        >
          I
        </button>
        <button
          type="button"
          className="compose-rich-tool"
          aria-label="下划线"
          onMouseDown={keepSelection}
          onClick={() => runCommand("underline")}
        >
          U
        </button>
        <button
          type="button"
          className="compose-rich-tool"
          aria-label="无序列表"
          onMouseDown={keepSelection}
          onClick={() => runCommand("insertUnorderedList")}
        >
          •
        </button>
        <button
          type="button"
          className="compose-rich-tool"
          aria-label="有序列表"
          onMouseDown={keepSelection}
          onClick={() => runCommand("insertOrderedList")}
        >
          1.
        </button>
        <button
          type="button"
          className="compose-rich-tool"
          aria-label="插入链接"
          onMouseDown={keepSelection}
          onClick={() => {
            setLinkOpen((prev) => !prev);
            setLinkError(null);
          }}
        >
          链接
        </button>
      </div>

      {linkOpen && (
        <div className="compose-rich-link">
          <input
            type="text"
            aria-label="链接地址"
            value={linkUrl}
            placeholder="https://example.com"
            onChange={(e) => setLinkUrl(e.target.value)}
          />
          <button type="button" aria-label="应用链接" onClick={applyLink}>
            应用
          </button>
          <button
            type="button"
            aria-label="取消链接"
            onClick={() => {
              setLinkOpen(false);
              setLinkUrl("");
              setLinkError(null);
            }}
          >
            取消
          </button>
          {linkError && (
            <span className="compose-rich-link-error" role="alert">
              {linkError}
            </span>
          )}
        </div>
      )}

      <div
        ref={editorRef}
        className="compose-rich-body"
        contentEditable
        suppressContentEditableWarning
        role="textbox"
        aria-multiline="true"
        aria-label="正文"
        onInput={emit}
        onPaste={handlePaste}
        onDrop={handleDrop}
      />
    </div>
  );
}
