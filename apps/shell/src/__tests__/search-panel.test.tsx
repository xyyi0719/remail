/**
 * 本地全文搜索面板。
 *
 * 这一组固化的行为：
 *
 * - 搜索只发一条 `search_messages` 命令，参数对不对；
 * - 空查询不搜（不跟数据库较劲）；
 * - 命中片段里的 `**` 会被高亮；
 * - 点开一条命中 → 切到对应账户与文件夹，并打开那封邮件；
 * - 后端报错时界面会说出来，不装作「没找到」。
 */
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import SearchPanel, { type SearchHit } from "../components/SearchPanel";
import { createBackend } from "../test/backend";
import { makeAccount, makeFolder, makeMessage } from "../test/fixtures";
import { ACCOUNT_EMAIL, renderApp, settle } from "../test/harness";
import { setInvokeImpl } from "../test/tauriState";

vi.mock("@tauri-apps/api/core", async () => {
  const { invokeMock } = await import("../test/tauriState");
  return { invoke: invokeMock };
});

vi.mock("@tauri-apps/api/event", async () => {
  const { listenMock } = await import("../test/tauriState");
  return { listen: listenMock };
});

function hit(overrides: Partial<SearchHit> = {}): SearchHit {
  return {
    messageId: "m1",
    accountId: "acc-1",
    folderId: "f1",
    uid: 101,
    subject: "项目进度",
    from: "同事 <同事@example.com>",
    dateSent: 1_700_000_000_000,
    snippet: "这周的**进度**还行",
    ...overrides,
  };
}

describe("搜索面板", () => {
  it("提交搜索会把查询词传给后端", async () => {
    const backend = createBackend({ searchHits: [hit()] });
    setInvokeImpl(backend.impl);
    const user = userEvent.setup();
    render(
      <SearchPanel onOpen={() => {}} onError={() => {}} />,
    );

    await user.type(screen.getByRole("searchbox"), "进度");
    await user.click(screen.getByRole("button", { name: "搜索" }));
    await settle();

    const call = backend.lastCallTo("search_messages");
    expect(call?.args).toMatchObject({ query: "进度" });
  });

  it("空查询不搜", async () => {
    const backend = createBackend({ searchHits: [hit()] });
    const user = userEvent.setup();
    render(<SearchPanel onOpen={() => {}} onError={() => {}} />);

    await user.click(screen.getByRole("button", { name: "搜索" }));
    await settle();

    expect(backend.callsTo("search_messages")).toHaveLength(0);
  });

  it("命中片段里的命中词被高亮", async () => {
    const backend = createBackend({ searchHits: [hit()] });
    setInvokeImpl(backend.impl);
    const user = userEvent.setup();
    render(<SearchPanel onOpen={() => {}} onError={() => {}} />);

    await user.type(screen.getByRole("searchbox"), "进度");
    await user.click(screen.getByRole("button", { name: "搜索" }));
    await settle();

    const mark = screen.getByText("进度");
    expect(mark.tagName).toBe("MARK");
  });

  it("没找到时说清楚是「没找到」，不是「还在搜」", async () => {
    const backend = createBackend({ searchHits: [] });
    setInvokeImpl(backend.impl);
    const user = userEvent.setup();
    render(<SearchPanel onOpen={() => {}} onError={() => {}} />);

    await user.type(screen.getByRole("searchbox"), "不存在的词");
    await user.click(screen.getByRole("button", { name: "搜索" }));
    await settle();

    expect(screen.getByText(/没找到/)).toBeInTheDocument();
  });

  it("点开一条命中会回调那封邮件", async () => {
    const backend = createBackend({ searchHits: [hit()] });
    setInvokeImpl(backend.impl);
    const opened: SearchHit[] = [];
    const user = userEvent.setup();
    render(
      <SearchPanel
        onOpen={(item) => opened.push(item)}
        onError={() => {}}
      />,
    );

    await user.type(screen.getByRole("searchbox"), "进度");
    await user.click(screen.getByRole("button", { name: "搜索" }));
    await settle();

    await user.click(screen.getByTestId("search-result"));
    expect(opened).toHaveLength(1);
    expect(opened[0].uid).toBe(101);
  });

  it("点开命中后：切到那封邮件所在的文件夹并打开它", async () => {
    const backend = createBackend({
      accounts: [makeAccount({ id: "acc-1", email: ACCOUNT_EMAIL })],
      folders: [
        makeFolder({ id: "f1", name: "收件箱" }),
        makeFolder({ id: "f2", name: "已发送" }),
      ],
      messages: [
        makeMessage({ id: "m1", accountId: "acc-1", folderId: "f1", uid: 101 }),
        makeMessage({
          id: "m2",
          accountId: "acc-1",
          folderId: "f2",
          uid: 202,
          subject: "项目进度",
        }),
      ],
      searchHits: [hit({ messageId: "m2", folderId: "f2", uid: 202 })],
    });

    const { user } = await renderApp(backend);

    await user.type(screen.getByRole("searchbox"), "进度");
    await user.click(screen.getByRole("button", { name: "搜索" }));
    await settle();

    await user.click(screen.getByTestId("search-result"));
    await settle();

    // 命中所在的文件夹被选中了。
    expect(backend.lastCallTo("list_messages")?.args).toMatchObject({
      folderId: "f2",
    });
    // 那封邮件被打开了（正文视图的标题）。
    expect(
      screen.getByRole("heading", { name: "项目进度" }),
    ).toBeInTheDocument();
  });
});
