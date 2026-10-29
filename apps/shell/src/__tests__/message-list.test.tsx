/**
 * 邮件列表本身的展示规则。
 *
 * 这组用例不碰后端（`MessageList` 不发命令），所以直接渲染组件。
 */
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import MessageList from "../components/MessageList";
import { makeMessage } from "../test/fixtures";
import { FLAG_FLAGGED, FLAG_SEEN } from "../types";

describe("邮件列表", () => {
  it("未读与已读有明确的区分", () => {
    render(
      <MessageList
        messages={[
          makeMessage({ id: "m1", uid: 1, subject: "未读那封", flags: 0 }),
          makeMessage({
            id: "m2",
            uid: 2,
            subject: "已读那封",
            flags: FLAG_SEEN,
          }),
        ]}
        selectedUid={null}
        onSelect={() => {}}
        onToggleStar={() => {}}
      />,
    );

    expect(screen.getByText("未读那封")).toHaveClass("message-unread");
    expect(screen.getByText("已读那封")).not.toHaveClass("message-unread");
  });

  it("星标按钮的无障碍状态跟着星标走", () => {
    render(
      <MessageList
        messages={[
          makeMessage({ id: "m1", uid: 1, subject: "普通那封", flags: 0 }),
          makeMessage({
            id: "m2",
            uid: 2,
            subject: "有星标那封",
            flags: FLAG_FLAGGED,
          }),
        ]}
        selectedUid={null}
        onSelect={() => {}}
        onToggleStar={() => {}}
      />,
    );

    expect(screen.getByRole("button", { name: "加星标" })).toHaveAttribute(
      "aria-pressed",
      "false",
    );
    expect(screen.getByRole("button", { name: "取消星标" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
  });

  it("点星标会把这一封交给上层处理", async () => {
    const user = userEvent.setup();
    const toggled: number[] = [];
    const message = makeMessage({ id: "m2", uid: 2, subject: "有星标那封" });

    render(
      <MessageList
        messages={[message]}
        selectedUid={null}
        onSelect={() => {}}
        onToggleStar={(m) => {
          toggled.push(m.uid);
        }}
      />,
    );

    await user.click(screen.getByRole("button", { name: "加星标" }));
    expect(toggled).toEqual([2]);
  });

  it("没有邮件时给出空状态提示", () => {
    render(
      <MessageList
        messages={[]}
        selectedUid={null}
        onSelect={() => {}}
        onToggleStar={() => {}}
      />,
    );

    expect(screen.getByText("这个文件夹里没有邮件。")).toBeInTheDocument();
  });
});
