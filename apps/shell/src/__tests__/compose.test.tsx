/**
 * 写信界面（写一封 → 存草稿 / 发送）。
 *
 * 这一组用例固化的是「写信」这条链路上最容易出错、也最难靠肉眼发现的点：
 *
 * - 发出去的参数对不对（收件人、抄送、密送、正文、附件都要原样到后端）；
 * - 改动会不会自动存草稿（而且别每敲一个字就存一次）；
 * - `uncertain`（服务器没确认）会不会被当成「失败」从而被自动重发——
 *   **重发一封可能已经发出去的邮件，就是给用户发了两封信**；
 * - 没写收件人时会不会白跑一趟。
 */
import { act, screen } from "@testing-library/react";
import { createBackend, fakeFilePicker } from "../test/backend";
import { makeAccount } from "../test/fixtures";
import type { AppHarness } from "../test/harness";
import { ACCOUNT_EMAIL, renderApp, settle } from "../test/harness";
import type { DraftInput } from "../types";

vi.mock("@tauri-apps/api/core", async () => {
  const { invokeMock } = await import("../test/tauriState");
  return { invoke: invokeMock };
});

vi.mock("@tauri-apps/api/event", async () => {
  const { listenMock } = await import("../test/tauriState");
  return { listen: listenMock };
});

/** 一个只含账户的假后端。 */
function backendWithAccount() {
  return createBackend({
    accounts: [makeAccount({ id: "acc-1", email: ACCOUNT_EMAIL })],
  });
}

/** 选中账户 → 打开写信面板。 */
async function openCompose(user: AppHarness["user"]): Promise<void> {
  await user.click(screen.getByRole("button", { name: ACCOUNT_EMAIL }));
  await settle();
  await user.click(screen.getByRole("button", { name: "写邮件" }));
  await settle();
}

/** 等过自动存草稿的防抖窗口（800ms），再把异步刷新排空。 */
async function waitForAutosave(): Promise<void> {
  await act(async () => {
    await new Promise((resolve) => {
      setTimeout(resolve, 900);
    });
  });
  await settle();
}

/** 从一次调用里取出草稿参数。 */
function draftOf(call: { args: { [key: string]: unknown } }): DraftInput {
  return call.args.draft as DraftInput;
}

describe("写信界面", () => {
  it("填好收件人和正文后点发送，会带着完整内容调用 send_message", async () => {
    const backend = backendWithAccount();
    const { user } = await renderApp(backend);
    await openCompose(user);

    await user.type(screen.getByLabelText("收件人"), "to@example.com");
    await user.type(screen.getByLabelText("主题"), "周末一起吃饭？");
    await user.type(screen.getByLabelText("正文"), "老地方见。");
    await user.click(screen.getByRole("button", { name: "发送" }));
    await settle();

    const calls = backend.callsTo("send_message");
    expect(calls).toHaveLength(1);
    const draft = draftOf(calls[0]);
    expect(draft.accountId).toBe("acc-1");
    expect(draft.to).toEqual(["to@example.com"]);
    expect(draft.cc).toEqual([]);
    expect(draft.bcc).toEqual([]);
    expect(draft.subject).toBe("周末一起吃饭？");
    expect(draft.bodyText).toBe("老地方见。");
    expect(draft.attachments).toEqual([]);
  });

  it("发送成功时显示「已发送」", async () => {
    const backend = backendWithAccount();
    const { user } = await renderApp(backend);
    await openCompose(user);

    await user.type(screen.getByLabelText("收件人"), "to@example.com");
    await user.click(screen.getByRole("button", { name: "发送" }));
    await settle();

    expect(screen.getByTestId("compose-send-result")).toHaveTextContent("已发送");
  });

  it("改动之后会自动存草稿，并显示保存时间", async () => {
    const backend = backendWithAccount();
    const { user } = await renderApp(backend);
    await openCompose(user);

    await user.type(screen.getByLabelText("收件人"), "to@example.com");
    await user.type(screen.getByLabelText("主题"), "草稿标题");

    await waitForAutosave();

    const calls = backend.callsTo("save_draft");
    expect(calls.length).toBeGreaterThanOrEqual(1);
    const draft = draftOf(calls[calls.length - 1]);
    expect(draft.accountId).toBe("acc-1");
    expect(draft.to).toEqual(["to@example.com"]);
    expect(draft.subject).toBe("草稿标题");
    // 保存时间要显示出来（HH:MM:SS）。
    expect(screen.getByTestId("compose-saved")).toHaveTextContent(
      /草稿已保存 \d{2}:\d{2}:\d{2}/,
    );
  });

  it("服务器没确认结果时给出明确警告，而且不会自动再发一次", async () => {
    const backend = backendWithAccount();
    backend.sendState = "uncertain";
    const { user } = await renderApp(backend);
    await openCompose(user);

    await user.type(screen.getByLabelText("收件人"), "to@example.com");
    await user.type(screen.getByLabelText("正文"), "正文");
    await user.click(screen.getByRole("button", { name: "发送" }));
    await settle();

    const result = screen.getByTestId("compose-send-result");
    expect(result).toHaveTextContent("可能已经发出去了");
    expect(result).toHaveTextContent("不要重复发送");

    // 关键：**不能**自动重发。多等一会儿也不该多出一次调用。
    await act(async () => {
      await new Promise((resolve) => {
        setTimeout(resolve, 300);
      });
    });
    await settle();
    expect(backend.callsTo("send_message")).toHaveLength(1);
  });

  it("没写收件人时拦在前端，不去调用 send_message", async () => {
    const backend = backendWithAccount();
    const { user } = await renderApp(backend);
    await openCompose(user);

    await user.type(screen.getByLabelText("主题"), "忘了写收件人");
    await user.click(screen.getByRole("button", { name: "发送" }));
    await settle();

    expect(backend.callsTo("send_message")).toHaveLength(0);
    expect(screen.getByTestId("compose-error")).toHaveTextContent(
      "至少要一个收件人",
    );
  });

  it("密送地址照常提交给后端（隐藏只是界面行为）", async () => {
    const backend = backendWithAccount();
    const { user } = await renderApp(backend);
    await openCompose(user);

    await user.type(screen.getByLabelText("收件人"), "to@example.com");
    await user.click(screen.getByRole("button", { name: "显示密送" }));
    await user.type(screen.getByLabelText("密送"), "secret@example.com");
    await user.click(screen.getByRole("button", { name: "发送" }));
    await settle();

    const calls = backend.callsTo("send_message");
    expect(calls).toHaveLength(1);
    const draft = draftOf(calls[0]);
    expect(draft.bcc).toEqual(["secret@example.com"]);
    expect(draft.to).toEqual(["to@example.com"]);
  });

  it("界面上一直说明密送不会显示在收件人列表里", async () => {
    const backend = backendWithAccount();
    const { user } = await renderApp(backend);
    await openCompose(user);

    expect(
      screen.getByText(/密送（Bcc）收件人不会出现在收件人列表里/),
    ).toBeInTheDocument();
  });

  it("选中的附件会把路径和文件名带给后端", async () => {
    const backend = backendWithAccount();
    const { user } = await renderApp(backend);
    await openCompose(user);

    // 附件改成走**路径**（后端自己读文件），所以这里 mock 的是
    // Tauri 的文件选择框，不是 `<input type="file">`。
    fakeFilePicker(["/tmp/报告.txt"]);
    await user.click(screen.getByRole("button", { name: "添加附件" }));
    await settle();

    expect(screen.getByText("报告.txt")).toBeInTheDocument();

    await user.type(screen.getByLabelText("收件人"), "to@example.com");
    await user.click(screen.getByRole("button", { name: "发送" }));
    await settle();

    const calls = backend.callsTo("send_message");
    expect(calls).toHaveLength(1);
    const draft = draftOf(calls[0]);
    expect(draft.attachments).toHaveLength(1);
    expect(draft.attachments[0].name).toBe("报告.txt");
    expect(draft.attachments[0].path).toBe("/tmp/报告.txt");
    // 前端拿不到真实大小（`open()` 只给路径），先填 0，
    // 真实大小由后端读文件后回填。
    expect(draft.attachments[0].size).toBe(0);
  });

  it("明确失败时给出错误提示，而且还能再发一次", async () => {
    const backend = backendWithAccount();
    backend.sendState = "failed";
    backend.sendDetail = "服务器拒绝了这次投递";
    const { user } = await renderApp(backend);
    await openCompose(user);

    await user.type(screen.getByLabelText("收件人"), "to@example.com");
    await user.click(screen.getByRole("button", { name: "发送" }));
    await settle();

    expect(screen.getByTestId("compose-send-result")).toHaveTextContent("发送失败");
    expect(screen.getByTestId("compose-send-result")).toHaveTextContent(
      "服务器拒绝了这次投递",
    );
    // 失败是「明确没发出去」，可以重试。
    const sendButton = screen.getByRole("button", { name: "发送" });
    expect(sendButton).toBeEnabled();

    await user.click(sendButton);
    await settle();
    expect(backend.callsTo("send_message")).toHaveLength(2);
  });

  it("还有没发出去的改动时，关闭面板要先确认", async () => {
    const backend = backendWithAccount();
    const { user } = await renderApp(backend);
    await openCompose(user);

    const confirmSpy = vi.spyOn(window, "confirm").mockReturnValue(false);
    await user.type(screen.getByLabelText("收件人"), "to@example.com");
    await user.click(screen.getByRole("button", { name: "关闭" }));
    await settle();

    expect(confirmSpy).toHaveBeenCalled();
    // 用户说「不关」→ 面板还在。
    expect(screen.getByLabelText("收件人")).toBeInTheDocument();

    confirmSpy.mockReturnValue(true);
    await user.click(screen.getByRole("button", { name: "关闭" }));
    await settle();
    expect(screen.queryByLabelText("收件人")).toBeNull();

    confirmSpy.mockRestore();
  });
});
