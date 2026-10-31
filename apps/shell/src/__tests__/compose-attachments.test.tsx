/**
 * 附件：走**路径**（不是字节）+ 上限提示。
 *
 * 附件改成传路径之后，`open()` 只给得出路径——大小与 MIME 前端拿不到，
 * 所以：
 *
 * - 上限检查只能拦「个数」与「已知大小」的附件；
 * - 真实大小由后端读文件后回填，前端先乐观显示「大小待后端校验」，
 *   后端回填后再更新。
 */
import { screen } from "@testing-library/react";
import { fakeFilePicker } from "../test/backend";
import type { Backend } from "../test/backend";
import { renderApp, settle } from "../test/harness";
import type { DraftAttachment, DraftDto } from "../types";
import {
  backendWithAccount,
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

/**
 * 塞一份「带超大附件」的草稿进去。
 *
 * 直接写假后端的存储：假后端的 `save_draft` 也会拦超限的附件，
 * 所以没法通过正常流程造出这种草稿——但真实后端的库里是能有这种数据的。
 */
function seedOversizedDraft(backend: Backend, size: number): void {
  const dto: DraftDto = {
    id: "draft-big",
    accountId: "acc-1",
    subject: "带大附件的草稿",
    to: ["to@example.com"],
    state: "draft",
    updatedAt: Date.now(),
  };
  backend.drafts = [...backend.drafts, dto];
  const attachment: DraftAttachment = {
    name: "大文件.bin",
    mime: "application/octet-stream",
    size,
    path: "",
  };
  backend.draftDetails.set(dto.id, {
    ...dto,
    cc: [],
    bcc: [],
    bodyText: "",
    bodyHtml: null,
    attachments: [attachment],
  });
}

describe("附件", () => {
  it("选完文件能挂上附件（只有名字，大小等后端回填）", async () => {
    const backend = backendWithAccount();
    const { user } = await renderApp(backend);
    await openCompose(user);

    fakeFilePicker(["/tmp/报告.txt"]);
    await user.click(screen.getByRole("button", { name: "添加附件" }));
    await settle();

    expect(screen.getByText("报告.txt")).toBeInTheDocument();
    // 前端拿不到真实大小（`open()` 只给路径）。
    expect(screen.getByTestId("compose-attachment-size")).toHaveTextContent(
      "大小待后端校验",
    );
  });

  it("保存之后后端回填真实大小，界面跟着更新", async () => {
    const backend = backendWithAccount();
    // 假后端「读文件」得到的大小。
    backend.fileSizes["/tmp/报告.txt"] = 2048;
    const { user } = await renderApp(backend);
    await openCompose(user);

    fakeFilePicker(["/tmp/报告.txt"]);
    await user.click(screen.getByRole("button", { name: "添加附件" }));
    await settle();

    await waitForAutosave();

    expect(screen.getByTestId("compose-attachment-size")).toHaveTextContent(
      "2.0 KB",
    );
  });

  it("可执行文件与脚本不让加", async () => {
    const backend = backendWithAccount();
    const { user } = await renderApp(backend);
    await openCompose(user);

    fakeFilePicker(["/tmp/工具.exe", "/tmp/脚本.sh"]);
    await user.click(screen.getByRole("button", { name: "添加附件" }));
    await settle();

    const notice = screen.getByTestId("compose-attachment-notice");
    expect(notice).toHaveTextContent("不能添加可执行文件或脚本");
    expect(notice).toHaveTextContent("工具.exe");
    // 没挂上。
    expect(screen.queryByText("工具.exe")).not.toBeInTheDocument();
  });

  it("一封信最多 5 个附件", async () => {
    const backend = backendWithAccount();
    const { user } = await renderApp(backend);
    await openCompose(user);

    fakeFilePicker([
      "/tmp/a.txt",
      "/tmp/b.txt",
      "/tmp/c.txt",
      "/tmp/d.txt",
      "/tmp/e.txt",
      "/tmp/f.txt",
    ]);
    await user.click(screen.getByRole("button", { name: "添加附件" }));
    await settle();

    expect(screen.getByTestId("compose-attachment-notice")).toHaveTextContent(
      "最多 5 个附件",
    );
    expect(screen.queryByText("a.txt")).not.toBeInTheDocument();
  });

  it("已知大小超过上限时会被拦下", async () => {
    const backend = backendWithAccount();
    // 30 MiB 的单个附件：超过 10 MiB 上限。
    seedOversizedDraft(backend, 30 * 1024 * 1024);
    const { user } = await renderApp(backend);
    await openCompose(user);

    await user.click(screen.getByRole("button", { name: "打开草稿 带大附件的草稿" }));
    await settle();

    fakeFilePicker(["/tmp/正常.txt"]);
    await user.click(screen.getByRole("button", { name: "添加附件" }));
    await settle();

    expect(screen.getByTestId("compose-attachment-notice")).toHaveTextContent(
      "超过单个附件上限",
    );
    expect(screen.queryByText("正常.txt")).not.toBeInTheDocument();
  });

  it("能移除附件", async () => {
    const backend = backendWithAccount();
    const { user } = await renderApp(backend);
    await openCompose(user);

    fakeFilePicker(["/tmp/报告.txt"]);
    await user.click(screen.getByRole("button", { name: "添加附件" }));
    await settle();
    expect(screen.getByText("报告.txt")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "移除附件 报告.txt" }));
    await settle();

    expect(screen.queryByText("报告.txt")).not.toBeInTheDocument();
  });
});
