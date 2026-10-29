/**
 * 邮件标记（星标 / 已读）的回写行为。
 *
 * 这一组用例固化的正是上一批改动里「点一下到底有没有写回服务器」
 * 的那些一次性手工验证：
 *
 * - `set_message_flag` 的参数对不对；
 * - 界面的无障碍状态跟不跟得上；
 * - 后端报错时界面会不会说谎（说成功了 / 把状态改了）。
 */
import { screen } from "@testing-library/react";
import { createBackend } from "../test/backend";
import { makeAccount, makeFolder, makeMessage } from "../test/fixtures";
import { ACCOUNT_EMAIL, renderApp, settle } from "../test/harness";

vi.mock("@tauri-apps/api/core", async () => {
  const { invokeMock } = await import("../test/tauriState");
  return { invoke: invokeMock };
});

vi.mock("@tauri-apps/api/event", async () => {
  const { listenMock } = await import("../test/tauriState");
  return { listen: listenMock };
});

function backendWithMessage(flags = 0) {
  return createBackend({
    accounts: [makeAccount({ id: "acc-1", email: ACCOUNT_EMAIL })],
    folders: [makeFolder({ id: "f1", name: "收件箱" })],
    messages: [
      makeMessage({
        id: "m1",
        accountId: "acc-1",
        folderId: "f1",
        uid: 101,
        flags,
      }),
    ],
  });
}

describe("邮件标记回写", () => {
  it("点星标会以正确的参数调用 set_message_flag", async () => {
    const backend = backendWithMessage();
    const { user, openFolder } = await renderApp(backend);
    await openFolder();

    await user.click(screen.getByRole("button", { name: "加星标" }));
    await settle();

    const calls = backend.callsTo("set_message_flag");
    expect(calls).toHaveLength(1);
    expect(calls[0].args).toEqual({
      accountId: "acc-1",
      folderId: "f1",
      uid: 101,
      flag: "flagged",
      value: true,
    });
  });

  it("星标的无障碍状态（aria-pressed）跟着星标状态变", async () => {
    const backend = backendWithMessage();
    const { user, openFolder } = await renderApp(backend);
    await openFolder();

    const before = screen.getByRole("button", { name: "加星标" });
    expect(before).toHaveAttribute("aria-pressed", "false");

    await user.click(before);
    await settle();

    const after = screen.getByRole("button", { name: "取消星标" });
    expect(after).toHaveAttribute("aria-pressed", "true");
  });

  it("后端报错时界面保持原状，并显示错误", async () => {
    const backend = backendWithMessage();
    backend.flagError = new Error("服务器拒绝了这次标记");
    const { user, openFolder } = await renderApp(backend);
    await openFolder();

    await user.click(screen.getByRole("button", { name: "加星标" }));
    await settle();

    // 错误要说清楚是哪里错了。
    expect(screen.getByTestId("app-error")).toHaveTextContent(
      "服务器拒绝了这次标记",
    );
    // 界面不能被「骗」：没生效就是没生效，不能显示成已生效。
    expect(screen.queryByText("已生效")).toBeNull();
    expect(screen.getByRole("button", { name: "加星标" })).toHaveAttribute(
      "aria-pressed",
      "false",
    );
  });

  it("正文面板的「标记为已读」会以 seen 参数回写", async () => {
    const backend = backendWithMessage();
    const { user, openFolder } = await renderApp(backend);
    await openFolder();

    await user.click(screen.getByRole("button", { name: /测试邮件/ }));
    await settle();

    await user.click(screen.getByRole("button", { name: "标记为已读" }));
    await settle();

    const calls = backend.callsTo("set_message_flag");
    expect(calls).toHaveLength(1);
    expect(calls[0].args).toEqual({
      accountId: "acc-1",
      folderId: "f1",
      uid: 101,
      flag: "seen",
      value: true,
    });
  });
});
