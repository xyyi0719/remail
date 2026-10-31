/**
 * 写信相关的用例共用的脚手架。
 *
 * ## 为什么 `vi.mock` 不在这里
 *
 * Vitest 只把 `vi.mock` 提到**所在文件**的最前面。写在被 import 的模块里
 * 就晚了一步——`App.tsx` 可能先把真的 `invoke` 抓在手里，
 * 于是测试根本接不上假后端。所以那两个 `vi.mock` 必须留在每个用例文件里。
 */
import { act, screen } from "@testing-library/react";
import { createBackend } from "../test/backend";
import { makeAccount } from "../test/fixtures";
import type { AppHarness } from "../test/harness";
import { ACCOUNT_EMAIL, settle } from "../test/harness";
import type { DraftInput } from "../types";

/** 一个只含账户的假后端。 */
export function backendWithAccount() {
  return createBackend({
    accounts: [makeAccount({ id: "acc-1", email: ACCOUNT_EMAIL })],
  });
}

/** 选中账户 → 打开写信面板。 */
export async function openCompose(user: AppHarness["user"]): Promise<void> {
  await user.click(screen.getByRole("button", { name: ACCOUNT_EMAIL }));
  await settle();
  await user.click(screen.getByRole("button", { name: "写邮件" }));
  await settle();
}

/** 等过自动存草稿的防抖窗口（800ms），再把异步刷新排空。 */
export async function waitForAutosave(): Promise<void> {
  await act(async () => {
    await new Promise((resolve) => {
      setTimeout(resolve, 900);
    });
  });
  await settle();
}

/** 从一次调用里取出草稿参数。 */
export function draftOf(call: { args: { [key: string]: unknown } }): DraftInput {
  return call.args.draft as DraftInput;
}
