/**
 * 地址的**轻量**校验。
 *
 * 前端只提示、不拦人：真正的校验在后端（那边会返回逐条问题）。
 * 这里要保证的是「提示够不够当场发现笔误」，以及**别把用户拦下来**。
 */
import { screen } from "@testing-library/react";
import { renderApp, settle } from "../test/harness";
import { backendWithAccount, openCompose } from "./compose-support";

vi.mock("@tauri-apps/api/core", async () => {
  const { invokeMock } = await import("../test/tauriState");
  return { invoke: invokeMock };
});

vi.mock("@tauri-apps/api/event", async () => {
  const { listenMock } = await import("../test/tauriState");
  return { listen: listenMock };
});

describe("地址轻量校验", () => {
  it("少了 @ 会提示", async () => {
    const backend = backendWithAccount();
    const { user } = await renderApp(backend);
    await openCompose(user);

    await user.type(screen.getByLabelText("收件人"), "abc");

    const warning = screen.getByTestId("compose-address-warning");
    expect(warning).toHaveTextContent("缺少 @");
  });

  it("地址之间有空白会提示", async () => {
    const backend = backendWithAccount();
    const { user } = await renderApp(backend);
    await openCompose(user);

    await user.type(screen.getByLabelText("收件人"), "a@example.com b@example.com");

    expect(screen.getByTestId("compose-address-warning")).toHaveTextContent(
      "空白",
    );
  });

  it("合法的地址不提示", async () => {
    const backend = backendWithAccount();
    const { user } = await renderApp(backend);
    await openCompose(user);

    await user.type(screen.getByLabelText("收件人"), "to@example.com");

    expect(screen.queryByTestId("compose-address-warning")).not.toBeInTheDocument();
  });

  it("只是提示，不会拦住用户发送", async () => {
    const backend = backendWithAccount();
    const { user } = await renderApp(backend);
    await openCompose(user);

    await user.type(screen.getByLabelText("收件人"), "abc");
    expect(screen.getByTestId("compose-address-warning")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "发送" }));
    await settle();

    // 拦人的是后端（它会返回逐条问题），前端只是提示一下。
    expect(backend.callsTo("send_message")).toHaveLength(1);
  });
});
