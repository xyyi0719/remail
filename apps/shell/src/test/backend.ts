import { FLAG_FLAGGED, FLAG_SEEN } from "../types";
import type { Account, Folder, Message } from "../types";
import type { InvokeArgs, InvokeImpl } from "./tauriState";

/**
 * 一个假的 Tauri 后端。
 *
 * 前端的所有写操作都走 `invoke`；这里按命令名分发，行为尽量贴近
 * Rust 侧真正的语义（尤其是 `set_message_flag` 会改本地账本，
 * 之后 `list_messages` 会把改过的位返回来）。
 */

export interface InvokeCall {
  cmd: string;
  args: InvokeArgs;
}

export interface BackendOptions {
  accounts?: Account[];
  folders?: Folder[];
  messages?: Message[];
  /** `set_message_flag` 抛这个错（测「回写失败」）。 */
  flagError?: Error | null;
  /** `forget_credential` 抛这个错。 */
  forgetError?: Error | null;
  /** `save_credential` 抛这个错。 */
  saveError?: Error | null;
  /** `sync_account` 抛这个错。 */
  syncError?: Error | null;
}

export interface Backend {
  accounts: Account[];
  folders: Folder[];
  messages: Message[];
  flagError: Error | null;
  forgetError: Error | null;
  saveError: Error | null;
  syncError: Error | null;
  /** 所有 invoke 调用（含参数），按顺序。 */
  calls: InvokeCall[];
  /** 装给 `invoke` 的实现。 */
  impl: InvokeImpl;
  /** 某个命令的全部调用。 */
  callsTo(cmd: string): InvokeCall[];
  /** 某个命令的最后一次调用。 */
  lastCallTo(cmd: string): InvokeCall | undefined;
}

export function createBackend(options: BackendOptions = {}): Backend {
  const backend: Backend = {
    accounts: options.accounts ?? [],
    folders: options.folders ?? [],
    messages: options.messages ?? [],
    flagError: options.flagError ?? null,
    forgetError: options.forgetError ?? null,
    saveError: options.saveError ?? null,
    syncError: options.syncError ?? null,
    calls: [],
    impl: async () => undefined,
    callsTo(cmd: string): InvokeCall[] {
      return backend.calls.filter((call) => call.cmd === cmd);
    },
    lastCallTo(cmd: string): InvokeCall | undefined {
      const all = backend.callsTo(cmd);
      return all[all.length - 1];
    },
  };

  const handlers: Record<string, (args: InvokeArgs) => unknown> = {
    list_accounts: () => backend.accounts.map((account) => ({ ...account })),
    app_info: () => ({
      name: "Xremail",
      version: "0.1.0",
      platform: "test",
      dataDir: null,
    }),
    list_folders: () => backend.folders.map((folder) => ({ ...folder })),
    // 每次都返回**新的对象**：后端返回新快照，前端才真的会重渲染。
    list_messages: () => backend.messages.map((message) => ({ ...message })),
    set_message_flag: (args) => {
      if (backend.flagError) {
        throw backend.flagError;
      }
      const uid = Number(args.uid);
      const message = backend.messages.find((m) => m.uid === uid);
      if (!message) {
        throw new Error(`没有 uid=${uid} 的邮件`);
      }
      const bit = args.flag === "flagged" ? FLAG_FLAGGED : FLAG_SEEN;
      message.flags =
        args.value === true ? message.flags | bit : message.flags & ~bit;
      return {
        queued: false,
        applied: true,
        state: "succeeded",
        message: null,
      };
    },
    forget_credential: () => {
      if (backend.forgetError) {
        throw backend.forgetError;
      }
      return true;
    },
    save_credential: () => {
      if (backend.saveError) {
        throw backend.saveError;
      }
      return undefined;
    },
    sync_account: () => {
      if (backend.syncError) {
        throw backend.syncError;
      }
      return { folders: backend.folders.length, added: 0, removed: 0 };
    },
    open_message: () => ({
      messageId: "<m1@example.com>",
      textPlain: "正文内容",
      htmlRaw: null,
      charset: null,
    }),
    create_account: () => ({}),
  };

  backend.impl = async (cmd: string, args?: InvokeArgs): Promise<unknown> => {
    const call: InvokeCall = { cmd, args: args ?? {} };
    backend.calls.push(call);
    const handler = handlers[cmd];
    if (!handler) {
      throw new Error(`测试里没有为命令「${cmd}」准备 mock`);
    }
    return handler(call.args);
  };

  return backend;
}
