import { act, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import App from "../App";
import type { Backend } from "./backend";
import { setInvokeImpl } from "./tauriState";

/**
 * 集成测试的脚手架。
 *
 * 这里的每个函数都只做一件事：**把异步的界面更新等到落定**。
 * 后端的每一次 `invoke` 都是异步的，不主动排空，React 就会在
 * `act(...)` 之外更新状态，然后报「更新没有包在 act 里」。
 */

/** 测试里用的账户邮箱（也是列表里那个按钮的可访问名）。 */
export const ACCOUNT_EMAIL = "me@example.com";

/**
 * 把 pending 的 promise 链跑完，并让 React 把结果刷到界面上。
 *
 * 用一个 `setTimeout(0)` 而不是几个 `Promise.resolve()`：
 * 微任务队列里可能还排着好几层 `.then()`，宏任务能让它们全部跑完。
 */
export async function settle(): Promise<void> {
  await act(async () => {
    await new Promise((resolve) => {
      setTimeout(resolve, 0);
    });
  });
}

export interface AppHarness {
  backend: Backend;
  user: ReturnType<typeof userEvent.setup>;
  /** 选中账户 → 选中文件夹；返回时邮件列表已经加载完。 */
  openFolder(name?: string): Promise<void>;
}

/**
 * 挂上 `<App />`，并把 `invoke` 接到假后端上。
 */
export async function renderApp(backend: Backend): Promise<AppHarness> {
  setInvokeImpl(backend.impl);
  const user = userEvent.setup();
  render(<App />);
  await settle();

  const openFolder = async (name = "收件箱"): Promise<void> => {
    await user.click(screen.getByRole("button", { name: ACCOUNT_EMAIL }));
    await settle();
    await user.click(screen.getByRole("button", { name: new RegExp(name) }));
    await settle();
  };

  return { backend, user, openFolder };
}
