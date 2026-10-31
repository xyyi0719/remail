/**
 * 富文本编辑器。
 *
 * 这一组用例固化的是三件事：
 *
 * - 纯文本 ↔ 富文本来回切**不能丢内容**（用户最不能接受的就是这个）；
 * - 富文本模式下发给后端的**既有 HTML 也有纯文本**
 *   （有些客户端只读纯文本）；
 * - 产出的 HTML 过一遍白名单：`img`、`style`、`on*`、`javascript:` 都不能有。
 */
import { fireEvent, screen } from "@testing-library/react";
import {
  htmlToPlainText,
  sanitizeHtml,
  textToHtml,
} from "../components/RichTextEditor";
import { renderApp, settle } from "../test/harness";
import {
  backendWithAccount,
  draftOf,
  openCompose,
} from "./compose-support";

vi.mock("@tauri-apps/api/core", async () => {
  const { invokeMock } = await import("../test/tauriState");
  return { invoke: invokeMock };
});

vi.mock("@tauri-apps/api/event", async () => {
  const { listenMock } = await import("../test/tauriState");
  return { listen: listenMock };
});

/**
 * 往富文本编辑器里写一段 HTML。
 *
 * 直接改 DOM 再触发 `input`：jsdom 里 `contentEditable` 的选区行为
 * 和真浏览器不一样，用 `user.type` 不可靠。
 */
async function writeRich(html: string): Promise<void> {
  const editor = screen.getByLabelText("正文");
  editor.innerHTML = html;
  fireEvent.input(editor);
  await settle();
}

describe("富文本编辑器", () => {
  it("工具栏按钮都有 aria-label（加粗/斜体/下划线/列表/链接）", async () => {
    const backend = backendWithAccount();
    const { user } = await renderApp(backend);
    await openCompose(user);

    await user.click(screen.getByRole("button", { name: "富文本模式" }));
    await settle();

    for (const name of ["加粗", "斜体", "下划线", "无序列表", "有序列表", "插入链接"]) {
      expect(screen.getByRole("button", { name })).toBeInTheDocument();
    }
  });

  it("纯文本切到富文本再切回来，内容不会丢", async () => {
    const backend = backendWithAccount();
    const { user } = await renderApp(backend);
    await openCompose(user);

    await user.type(screen.getByLabelText("正文"), "第一段");
    await user.click(screen.getByRole("button", { name: "富文本模式" }));
    await settle();

    // 富文本模式下编辑区是 contentEditable 的。
    const editor = screen.getByLabelText("正文");
    expect(editor).toHaveAttribute("contenteditable", "true");

    await user.click(screen.getByRole("button", { name: "纯文本模式" }));
    await settle();

    expect(screen.getByLabelText("正文")).toHaveValue("第一段");
  });

  it("富文本模式下会同时送出 HTML 与纯文本", async () => {
    const backend = backendWithAccount();
    const { user } = await renderApp(backend);
    await openCompose(user);

    await user.type(screen.getByLabelText("收件人"), "to@example.com");
    await user.click(screen.getByRole("button", { name: "富文本模式" }));
    await settle();

    await writeRich("<p>你好 <b>世界</b></p>");

    await user.click(screen.getByRole("button", { name: "发送" }));
    await settle();

    const calls = backend.callsTo("send_message");
    expect(calls).toHaveLength(1);
    const draft = draftOf(calls[0]);
    // HTML 给富文本客户端。
    expect(draft.bodyHtml).toBe("<p>你好 <b>世界</b></p>");
    // 纯文本给只读 text/plain 的客户端——**必须**同时给。
    expect(draft.bodyText).toBe("你好 世界");
  });

  it("编辑器里粘进来的危险 HTML 会被洗掉", async () => {
    const backend = backendWithAccount();
    const { user } = await renderApp(backend);
    await openCompose(user);

    await user.type(screen.getByLabelText("收件人"), "to@example.com");
    await user.click(screen.getByRole("button", { name: "富文本模式" }));
    await settle();

    await writeRich('<p>正文</p><img src="x" onerror="alert(1)"><script>alert(2)</script>');

    await user.click(screen.getByRole("button", { name: "发送" }));
    await settle();

    const draft = draftOf(backend.callsTo("send_message")[0]);
    expect(draft.bodyHtml).not.toContain("img");
    expect(draft.bodyHtml).not.toContain("onerror");
    expect(draft.bodyHtml).not.toContain("script");
    expect(draft.bodyHtml).toContain("正文");
  });
});

describe("HTML 白名单（纯函数）", () => {
  it("去掉脚本、样式、事件属性与图片", () => {
    expect(sanitizeHtml('<p onclick="x()">你好</p>')).toBe("<p>你好</p>");
    expect(sanitizeHtml('<img src="x" onerror="alert(1)">')).toBe("");
    expect(sanitizeHtml("<script>alert(1)</script>")).toBe("");
    expect(sanitizeHtml("<style>p{}</style><p>x</p>")).toBe("<p>x</p>");
    expect(sanitizeHtml('<span style="color:red">红</span>')).toBe("<span>红</span>");
  });

  it("链接只放行 http/https/mailto", () => {
    expect(sanitizeHtml('<a href="https://example.com">点</a>')).toBe(
      '<a href="https://example.com">点</a>',
    );
    expect(sanitizeHtml('<a href="mailto:a@b.com">点</a>')).toBe(
      '<a href="mailto:a@b.com">点</a>',
    );
    expect(sanitizeHtml('<a href="javascript:alert(1)">点</a>')).toBe("点");
    expect(sanitizeHtml('<a href="data:text/html,x">点</a>')).toBe("点");
  });

  it("HTML 与纯文本能互相转回来", () => {
    expect(textToHtml("第一行\n第二行")).toBe("<p>第一行</p><p>第二行</p>");
    expect(htmlToPlainText("<p>第一行</p><p>第二行</p>")).toBe("第一行\n第二行");
    // 列表要能看出层级。
    expect(htmlToPlainText("<ul><li>甲</li><li>乙</li></ul>")).toBe("• 甲\n• 乙");
    expect(htmlToPlainText("<ol><li>甲</li><li>乙</li></ol>")).toBe("1. 甲\n2. 乙");
  });
});
