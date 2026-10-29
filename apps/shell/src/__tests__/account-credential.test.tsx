/**
 * 账户列表与授权码的行为。
 *
 * 覆盖上一批改动里「授权码落平台钥匙串」那部分的断言：
 *
 * - 账户条目里不能有嵌套的 `<button>`（无障碍硬要求）；
 * - 「忘记凭据」真的会调 `forget_credential`；
 * - 失败时要说清楚哪里错了，不能静默失败；
 * - 保存时调 `save_credential`，且**口令不出现在任何渲染输出里**。
 */
import { screen, within } from "@testing-library/react";
import { createBackend } from "../test/backend";
import { makeAccount } from "../test/fixtures";
import { ACCOUNT_EMAIL, renderApp, settle } from "../test/harness";

vi.mock("@tauri-apps/api/core", async () => {
  const { invokeMock } = await import("../test/tauriState");
  return { invoke: invokeMock };
});

vi.mock("@tauri-apps/api/event", async () => {
  const { listenMock } = await import("../test/tauriState");
  return { listen: listenMock };
});

const CREDENTIAL = "client-secret-1234";

function backendWithAccount() {
  return createBackend({
    accounts: [makeAccount({ id: "acc-1", email: ACCOUNT_EMAIL })],
  });
}

describe("账户列表", () => {
  it("账户条目里没有嵌套的 button", async () => {
    const backend = backendWithAccount();
    await renderApp(backend);

    const item = await screen.findByTestId("account-item");
    const buttons = within(item).getAllByRole("button");
    expect(buttons.length).toBeGreaterThan(0);
    for (const button of buttons) {
      // button 里不能再套 button：屏幕阅读器无法正确播报。
      expect(button.querySelector("button")).toBeNull();
    }
    expect(item.querySelector("button button")).toBeNull();
  });

  it("「忘记凭据」会调用 forget_credential", async () => {
    const backend = backendWithAccount();
    const { user } = await renderApp(backend);

    await user.click(await screen.findByRole("button", { name: "忘记凭据" }));
    await settle();

    const calls = backend.callsTo("forget_credential");
    expect(calls).toHaveLength(1);
    expect(calls[0].args).toEqual({ accountId: "acc-1" });
  });

  it("忘记凭据失败时给出说清原因的错误提示", async () => {
    const backend = backendWithAccount();
    backend.forgetError = new Error("凭据管理器不可用");
    const { user } = await renderApp(backend);

    await user.click(await screen.findByRole("button", { name: "忘记凭据" }));
    await settle();

    const notice = screen.getByTestId("app-notice");
    expect(notice).toHaveTextContent("忘记凭据失败");
    expect(notice).toHaveTextContent("凭据管理器不可用");
    // 失败是错误级别的提示，不能跟「已生效」混在一起显示。
    expect(notice).toHaveClass("app-notice-error");
  });

  it("忘记凭据成功时提示「已从系统凭据库移除」", async () => {
    const backend = backendWithAccount();
    const { user } = await renderApp(backend);

    await user.click(await screen.findByRole("button", { name: "忘记凭据" }));
    await settle();

    const notice = screen.getByTestId("app-notice");
    expect(notice).toHaveTextContent("已从系统凭据库移除该授权码");
    expect(notice).toHaveClass("app-notice-ok");
  });
});

describe("授权码保存", () => {
  it("保存会调用 save_credential，参数正确", async () => {
    const backend = backendWithAccount();
    const { user } = await renderApp(backend);

    const input = await screen.findByLabelText(/IMAP 授权码/);
    await user.type(input, CREDENTIAL);
    await user.click(
      screen.getByRole("button", { name: /保存授权码到系统凭据库/ }),
    );
    await settle();

    const calls = backend.callsTo("save_credential");
    expect(calls).toHaveLength(1);
    expect(calls[0].args).toEqual({
      accountId: "acc-1",
      credential: CREDENTIAL,
    });
  });

  it("口令不出现在任何渲染输出里", async () => {
    const backend = backendWithAccount();
    const { user } = await renderApp(backend);

    const input = await screen.findByLabelText(/IMAP 授权码/);
    // 输入框本身是密码框，明文不会被显示出来。
    expect(input).toHaveAttribute("type", "password");

    await user.type(input, CREDENTIAL);
    await settle();

    // 界面上任何地方都不该出现这段明文。
    expect(document.body.textContent ?? "").not.toContain(CREDENTIAL);

    await user.click(
      screen.getByRole("button", { name: /保存授权码到系统凭据库/ }),
    );
    await settle();

    // 存完之后连输入框里都不留明文。
    expect(screen.getByLabelText(/IMAP 授权码/)).toHaveValue("");
    expect(document.body.textContent ?? "").not.toContain(CREDENTIAL);
  });

  it("保存失败时给出说清原因的错误提示", async () => {
    const backend = backendWithAccount();
    backend.saveError = new Error("当前平台暂不支持保存凭据");
    const { user } = await renderApp(backend);

    const input = await screen.findByLabelText(/IMAP 授权码/);
    await user.type(input, CREDENTIAL);
    await user.click(
      screen.getByRole("button", { name: /保存授权码到系统凭据库/ }),
    );
    await settle();

    const notice = screen.getByTestId("app-notice");
    expect(notice).toHaveTextContent("保存授权码失败");
    expect(notice).toHaveTextContent("当前平台暂不支持保存凭据");
    expect(notice).toHaveClass("app-notice-error");
  });
});
