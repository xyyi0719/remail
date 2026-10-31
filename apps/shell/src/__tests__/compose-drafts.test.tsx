/**
 * 草稿箱：列出来 + 「接着上次那封改」。
 *
 * 这一组用例固化的是写信面板最容易出问题的一处：
 * 面板关掉再打开时，**不该新建一份草稿**，而要把上次那封填回编辑区，
 * 之后保存 / 发送的还得是**同一份**草稿（id 带回去）。
 */
import { screen } from "@testing-library/react";
import type { Backend } from "../test/backend";
import { renderApp, settle } from "../test/harness";
import type { DraftDto, DraftInput } from "../types";
import {
  backendWithAccount,
  draftOf,
  openCompose,
  waitForAutosave,
} from "./compose-support";

vi.mock("@tauri-apps/api/core", async () => {
  const { invokeMock } = await import("../test/tauriState");
  return { invoke: invokeMock };
});

vi.mock("@tauri-apps/api/event", async () => {
  const { listenMock } = await import("../test/tauriState");
  return { listen: listenMock };
});

/** 造一份草稿存进假后端（相当于用户上次写了一半没发）。 */
async function seedDraft(
  backend: Backend,
  overrides: Partial<DraftInput> = {},
): Promise<DraftDto> {
  const draft: DraftInput = {
    accountId: "acc-1",
    to: ["to@example.com"],
    cc: [],
    bcc: [],
    subject: "上次的草稿",
    bodyText: "上次写的内容",
    bodyHtml: null,
    attachments: [],
    ...overrides,
  };
  return (await backend.impl("save_draft", { draft })) as DraftDto;
}

describe("草稿箱", () => {
  it("没有草稿时给出明确提示，不是空白一片", async () => {
    const backend = backendWithAccount();
    const { user } = await renderApp(backend);
    await openCompose(user);

    expect(screen.getByTestId("compose-drafts-empty")).toHaveTextContent(
      "还没有草稿",
    );
  });

  it("草稿列表里能看到主题、收件人和更新时间", async () => {
    const backend = backendWithAccount();
    await seedDraft(backend);

    const { user } = await renderApp(backend);
    await openCompose(user);

    const item = screen.getByRole("button", { name: "打开草稿 上次的草稿" });
    expect(item).toHaveTextContent("上次的草稿");
    expect(item).toHaveTextContent("to@example.com");
    expect(item).toHaveTextContent(/\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}/);
  });

  it("点开草稿会把内容填回表单", async () => {
    const backend = backendWithAccount();
    await seedDraft(backend, {
      subject: "上次的草稿",
      bodyText: "上次写的内容",
      cc: ["cc@example.com"],
    });

    const { user } = await renderApp(backend);
    await openCompose(user);

    await user.click(screen.getByRole("button", { name: "打开草稿 上次的草稿" }));
    await settle();

    expect(screen.getByLabelText("收件人")).toHaveValue("to@example.com");
    expect(screen.getByLabelText("抄送")).toHaveValue("cc@example.com");
    expect(screen.getByLabelText("主题")).toHaveValue("上次的草稿");
    expect(screen.getByLabelText("正文")).toHaveValue("上次写的内容");
  });

  it("接着改之后保存的还是同一份草稿（id 带回去）", async () => {
    const backend = backendWithAccount();
    const saved = await seedDraft(backend);

    const { user } = await renderApp(backend);
    await openCompose(user);

    await user.click(screen.getByRole("button", { name: "打开草稿 上次的草稿" }));
    await settle();

    // 改一点东西 → 自动保存。
    await user.type(screen.getByLabelText("主题"), "（改过）");
    await waitForAutosave();

    const calls = backend.callsTo("save_draft");
    expect(calls.length).toBeGreaterThanOrEqual(1);
    const last = draftOf(calls[calls.length - 1]);
    expect(last.id).toBe(saved.id);
    expect(last.subject).toBe("上次的草稿（改过）");
    // 列表里还是那一份（没有多出第二份草稿）。
    expect(backend.drafts.filter((one) => one.accountId === "acc-1")).toHaveLength(1);
  });

  it("打开草稿会把附件一起填回来", async () => {
    const backend = backendWithAccount();
    await seedDraft(backend, {
      attachments: [
        { name: "报告.txt", mime: "text/plain", size: 1234, path: "" },
      ],
    });

    const { user } = await renderApp(backend);
    await openCompose(user);

    await user.click(screen.getByRole("button", { name: "打开草稿 上次的草稿" }));
    await settle();

    expect(screen.getByText("报告.txt")).toBeInTheDocument();
    // 大小是人能读的形式。
    expect(screen.getByTestId("compose-attachment-size")).toHaveTextContent(
      "1.2 KB",
    );
  });

  it("「新建一封」会清空当前内容，回到空白草稿", async () => {
    const backend = backendWithAccount();
    await seedDraft(backend);

    const { user } = await renderApp(backend);
    await openCompose(user);

    await user.click(screen.getByRole("button", { name: "打开草稿 上次的草稿" }));
    await settle();
    expect(screen.getByLabelText("主题")).toHaveValue("上次的草稿");

    await user.click(screen.getByRole("button", { name: "新建一封" }));
    await settle();

    expect(screen.getByLabelText("主题")).toHaveValue("");
    expect(screen.getByLabelText("正文")).toHaveValue("");
  });
});
