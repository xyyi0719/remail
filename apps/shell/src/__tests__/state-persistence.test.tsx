/**
 * 重新拉取列表之后，状态不能丢。
 *
 * 前端不在本地「猜」标记位：写完标记会重新拉一次列表，用后端
 * （服务器快照 + 未提交意图账本）返回的状态当准。这组用例盯的就是
 * 这条链路——标记完再拉一次，星标 / 已读的高亮还得在。
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

function backendWithMessage() {
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
  });
}

describe("重新拉取列表后状态不丢", () => {
  it("标记后会重新拉一次列表，星标高亮仍在", async () => {
    const backend = backendWithMessage();
    const { user, openFolder } = await renderApp(backend);
    await openFolder();

    const before = backend.callsTo("list_messages").length;

    await user.click(screen.getByRole("button", { name: "加星标" }));
    await settle();

    // 写完之后确实重新拉了一次列表（状态来自后端，不是前端自己改的）。
    expect(backend.callsTo("list_messages").length).toBeGreaterThan(before);
    // 重新拉回来的那一份，星标高亮还在。
    const starred = screen.getByRole("button", { name: "取消星标" });
    expect(starred).toHaveAttribute("aria-pressed", "true");
  });

  it("重新同步之后，已读与星标都还在", async () => {
    const backend = backendWithMessage();
    const { user, openFolder } = await renderApp(backend);
    await openFolder();

    // 加星标 + 标记已读。
    await user.click(screen.getByRole("button", { name: "加星标" }));
    await settle();
    await user.click(screen.getByRole("button", { name: /测试邮件/ }));
    await settle();
    await user.click(screen.getByRole("button", { name: "标记为已读" }));
    await settle();

    expect(screen.getByRole("button", { name: "取消星标" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    expect(screen.getByTestId("message-item")).toHaveTextContent("测试邮件");
    expect(
      screen.getByTestId("message-item").querySelector(".message-unread"),
    ).toBeNull();

    // 再同步一次：同步会触发应用重新拉文件夹与邮件列表。
    const input = screen.getByLabelText(/IMAP 授权码/);
    await user.type(input, "sync-credential");
    await user.click(screen.getByRole("button", { name: "同步" }));
    await settle();

    expect(backend.callsTo("sync_account")).toHaveLength(1);
    expect(screen.getByRole("button", { name: "取消星标" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    expect(
      screen.getByTestId("message-item").querySelector(".message-unread"),
    ).toBeNull();
  });
});
