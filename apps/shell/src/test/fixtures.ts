import type { Account, Folder, Message } from "../types";

/**
 * 测试用的数据样板。
 *
 * 只填**测试真正关心的字段**，其余给合理的默认值——这样用例读起来
 * 一眼能看出「这条断言关心什么」。
 */

export function makeAccount(overrides: Partial<Account> = {}): Account {
  return {
    id: "acc-1",
    email: "me@example.com",
    provider: "netease_163",
    displayName: null,
    color: null,
    authKind: "password",
    server: {
      imapHost: "imap.163.com",
      imapPort: 993,
      imapTls: "implicit",
      smtpHost: "smtp.163.com",
      smtpPort: 465,
      smtpTls: "implicit",
      smtpAuth: "login",
    },
    username: "me@example.com",
    secretRef: "cred-1",
    oauthClientId: null,
    syncMode: "poll",
    pollIntervalSecs: 300,
    signatureHtml: null,
    createdAt: 1_700_000_000,
    updatedAt: 1_700_000_000,
    ...overrides,
  };
}

export function makeFolder(overrides: Partial<Folder> = {}): Folder {
  return {
    id: "f1",
    name: "收件箱",
    rawName: "INBOX",
    delimiter: "/",
    uidvalidity: 1,
    lastUid: 101,
    lastSyncAt: 1_700_000_000,
    ...overrides,
  };
}

export function makeMessage(overrides: Partial<Message> = {}): Message {
  return {
    id: "m1",
    accountId: "acc-1",
    folderId: "f1",
    uid: 101,
    uidvalidity: 1,
    messageId: "<m1@example.com>",
    subject: "测试邮件",
    snippet: "这是一封用来测试的邮件",
    from: ["sender@example.com"],
    to: ["me@example.com"],
    cc: [],
    bcc: [],
    dateSent: 1_700_000_000,
    internalDate: 1_700_000_000,
    flags: 0,
    size: 128,
    hasAttachment: false,
    createdAt: 1_700_000_000,
    updatedAt: 1_700_000_000,
    ...overrides,
  };
}
