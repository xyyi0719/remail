/**
 * 测试环境准备。
 *
 * 三件事：
 *
 * 1. 引入 jest-dom 的 DOM 匹配器（`toBeInTheDocument` 之类）；
 * 2. 打开 React 19 的 act 环境（不开的话 `act(...)` 会警告）；
 * 3. **要求 console 干净**：测试期间任何 `console.error` / `console.warn`
 *    （包括 React 的 act 警告、「更新没有包在 act 里」之类）都会让用例失败。
 *    交互测试最常见的坑就是「测过了但控制台在报警」，这里直接把它变成红灯。
 *
 * 另外把 Tauri 的文件选择对话框插件 mock 掉：真插件会拉起原生窗口，
 * 单元测试里没法用。
 */
import "@testing-library/jest-dom/vitest";
import { afterEach, beforeEach, vi } from "vitest";
import { open } from "@tauri-apps/plugin-dialog";

// 不开这个开关，React 19 的 `act(...)` 会警告「当前测试环境不支持 act」。
globalThis.IS_REACT_ACT_ENVIRONMENT = true;

vi.mock("@tauri-apps/plugin-dialog", () => ({
  open: vi.fn(async () => null),
  save: vi.fn(async () => null),
  message: vi.fn(async () => undefined),
  ask: vi.fn(async () => false),
  confirm: vi.fn(async () => false),
}));

interface ConsoleRecord {
  kind: "error" | "warn";
  args: string[];
}

let records: ConsoleRecord[] = [];
let restoreConsole: (() => void) | null = null;

beforeEach(() => {
  records = [];

  // 文件选择框默认「一个都没选」：用例需要选文件时自己改返回值。
  vi.mocked(open).mockReset();
  vi.mocked(open).mockResolvedValue(null);

  const errorSpy = vi
    .spyOn(console, "error")
    .mockImplementation((...args: unknown[]) => {
      records.push({ kind: "error", args: args.map((a) => String(a)) });
    });
  const warnSpy = vi
    .spyOn(console, "warn")
    .mockImplementation((...args: unknown[]) => {
      records.push({ kind: "warn", args: args.map((a) => String(a)) });
    });

  restoreConsole = () => {
    errorSpy.mockRestore();
    warnSpy.mockRestore();
  };
});

afterEach(() => {
  restoreConsole?.();
  restoreConsole = null;

  const dirty = records;
  records = [];

  if (dirty.length > 0) {
    const detail = dirty
      .map((r) => `  [${r.kind}] ${r.args.join(" ")}`)
      .join("\n");
    throw new Error(
      `测试期间 console 里有输出（${dirty.length} 条），必须处理干净：\n${detail}`,
    );
  }
});
