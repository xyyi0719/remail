/**
 * 邮件正文的渲染行为。
 *
 * 这一组用例固化的是「正文怎么显示」的三条约定：
 *
 * - **HTML 按 HTML 显示**（不是把源码打印出来）；
 * - **纯文本按纯文本显示**；
 * - **正文里的链接不让应用自己跳**——邮件内容不可信，
 *   让 webview 自己跳会把整个应用窗口换成那个网页。
 *
 * 渲染的 HTML 是**后端消毒过的**那份（`htmlSafe`），
 * 消毒本身在后端（`xremail-core::html`），用例在那边。
 */
import { screen } from "@testing-library/react";
import { createBackend } from "../test/backend";
import { makeAccount, makeFolder, makeMessage } from "../test/fixtures";
import { ACCOUNT_EMAIL, renderApp, settle } from "../test/harness";
import type { MessageBody } from "../types";

vi.mock("@tauri-apps/api/core", async () => {
  const { invokeMock } = await import("../test/tauriState");
  return { invoke: invokeMock };
});

vi.mock("@tauri-apps/api/event", async () => {
  const { listenMock } = await import("../test/tauriState");
  return { listen: listenMock };
});

const openUrlMock = vi.fn(async (_url: string) => {});
vi.mock("@tauri-apps/plugin-opener", () => ({
  openUrl: (url: string) => openUrlMock(url),
}));

function backendWithBody(body: MessageBody) {
  return createBackend({
    accounts: [makeAccount({ id: "acc-1", email: ACCOUNT_EMAIL })],
    folders: [makeFolder({ id: "f1", name: "收件箱" })],
    messages: [
      makeMessage({
        id: "m1",
        accountId: "acc-1",
        folderId: "f1",
        uid: 101,
      }),
    ],
    messageBody: body,
  });
}

/** 打开文件夹并点开那封邮件。 */
async function openMail(backend: ReturnType<typeof createBackend>) {
  const { user, openFolder } = await renderApp(backend);
  await openFolder();
  await user.click(screen.getByRole("button", { name: /测试邮件/ }));
  await settle();
  return user;
}

describe("邮件正文渲染", () => {
  it("后端给了 HTML 就按 HTML 显示，不当成源码打印", async () => {
    const backend = backendWithBody({
      messageId: "<m1@example.com>",
      textPlain: null,
      htmlSafe: "<p>正文里的<strong>重点</strong></p>",
      charset: "utf-8",
    });
    await openMail(backend);

    // 标签是真的元素，不是被转义的文本。
    const strong = screen.getByText("重点");
    expect(strong.tagName).toBe("STRONG");
    expect(strong.closest("p")).not.toBeNull();
    // 不该把源码当文字显示出来。
    expect(screen.queryByText(/<strong>/)).toBeNull();
  });

  it("只有纯文本时按纯文本显示", async () => {
    const backend = backendWithBody({
      messageId: "<m1@example.com>",
      textPlain: "一封纯文本邮件",
      htmlSafe: null,
      charset: "utf-8",
    });
    await openMail(backend);

    expect(screen.getByText("一封纯文本邮件")).toBeTruthy();
  });

  it("正文里的链接交给系统浏览器，应用自己不跳", async () => {
    const backend = backendWithBody({
      messageId: "<m1@example.com>",
      textPlain: null,
      htmlSafe: '<p>看看 <a href="https://example.com/x">这个链接</a></p>',
      charset: "utf-8",
    });
    const user = await openMail(backend);

    // 记录点击被处理完之后默认行为有没有被阻止。
    let prevented: boolean | null = null;
    const onDocumentClick = (event: MouseEvent) => {
      prevented = event.defaultPrevented;
    };
    document.addEventListener("click", onDocumentClick);

    await user.click(screen.getByRole("link", { name: "这个链接" }));
    await settle();

    document.removeEventListener("click", onDocumentClick);

    expect(openUrlMock).toHaveBeenCalledWith("https://example.com/x");
    // 默认行为必须被阻止——不然应用窗口会被那个网页顶掉。
    expect(prevented).toBe(true);
  });

  it("危险协议的链接不会被交给浏览器", async () => {
    const backend = backendWithBody({
      messageId: "<m1@example.com>",
      textPlain: null,
      // 后端消毒器本就不会放行这种链接；
      // 这里绕过后端，锁住前端这一层的兜底。
      htmlSafe: '<a href="javascript:alert(1)">点我</a>',
      charset: "utf-8",
    });
    const user = await openMail(backend);

    await user.click(screen.getByRole("link", { name: "点我" }));
    await settle();

    expect(openUrlMock).not.toHaveBeenCalled();
  });
});
