/**
 * Tauri `invoke` / `listen` 的共享替身。
 *
 * 每个测试文件自己写 `vi.mock("@tauri-apps/api/core", ...)`（mock 必须
 * 写在**被测文件所在的那个文件**里，vitest 才会把它套到组件的 import 上），
 * 工厂里把 `invoke` 指向这里导出的 `invokeMock`。
 *
 * 真正的行为由测试通过 `setInvokeImpl` 装进来（见 `backend.ts`）。
 */

export interface InvokeArgs {
  [key: string]: unknown;
}

export type InvokeImpl = (cmd: string, args?: InvokeArgs) => Promise<unknown>;

const noBackend: InvokeImpl = async (cmd: string) => {
  throw new Error(`测试里没有为命令「${cmd}」准备 mock`);
};

let currentImpl: InvokeImpl = noBackend;

/** 装上本次测试要用的 invoke 实现。 */
export function setInvokeImpl(impl: InvokeImpl): void {
  currentImpl = impl;
}

/** 卸掉实现，回到「未配置」状态。 */
export function resetInvokeImpl(): void {
  currentImpl = noBackend;
}

/** 供 `vi.mock` 工厂使用的 `invoke` 替身。 */
export async function invokeMock(cmd: string, args?: InvokeArgs): Promise<unknown> {
  return currentImpl(cmd, args);
}

/** `listen` 订阅过的事件名（用来确认订阅没漏）。 */
export const listenedEvents: string[] = [];

/** 供 `vi.mock` 工厂使用的 `listen` 替身。 */
export async function listenMock(event: string): Promise<() => void> {
  listenedEvents.push(event);
  return () => {};
}

/** 清掉 `listen` 的记录。 */
export function resetListenMock(): void {
  listenedEvents.length = 0;
}
