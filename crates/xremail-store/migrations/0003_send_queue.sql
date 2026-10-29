-- ===========================================================================
-- 扩展 send_queue：草稿正文 / 附件 / 幂等键
-- ===========================================================================
--
-- ## 为什么扩表，而不是新建一张 outgoing_message
--
-- `0001_init.sql` 已经按 ARCHITECTURE §5 把发信队列设计成了 `send_queue`：
-- envelope（实际投递地址）与 headers（显示用 To/Cc/Subject）分离，
-- 保证 Bcc 不泄漏且不破坏 To/Cc。
--
-- 再建一张同义表 = 两处同义字段。ARCHITECTURE §4.3 对 sync_state 已经这么说过一次
-- （「不另建 sync_state 表——那样会出现两处同义字段」），这里同理。
--
-- ## 本次补的列
--
-- * `body_text` / `body_html`：草稿阶段可编辑的正文。
--   入队后以 `message_bytes` 为准（构建好的 MIME 全文，固定不变）。
-- * `message_bytes`：入队时构建好的邮件字节。**一旦入队就固定**，
--   重试发的是同样的字节 → `Message-ID` 不变 → 对端认得出这是同一封。
-- * `message_id`：从构建结果取出，便于展示与对账。
-- * `idempotency_key`：同一封逻辑邮件的稳定标识。重试走「更新同一行」
--   而不是「新建一行」，所以不会出现「重试变成了另一封信」。
--
-- ## 状态取值
--
-- `0001` 的注释写的是 queued / running / succeeded / retryable / failed / uncertain，
-- 本迁移**沿用这套取值**（`xremail_core::model::SendState` 与之逐字对应），
-- 只多一个 `draft`：草稿阶段（用户还在编辑，尚未构建邮件）。
--
-- 草稿与发信队列同表同行的理由：草稿转成待发邮件时，**用户改过的内容要原样带过去**；
-- 拆成两张表就得再写一遍「合并草稿到待发记录」的逻辑，还容易漏字段。
--
-- ## 关于 body_ref
--
-- `0001` 留了 `body_ref`（正文的外部引用）。正文与附件现在**内联**存在库里，
-- 还没有外置到 blob 库，所以这一列当前恒为 NULL——
-- 留着一个恒为 NULL 的列，好过填一个指不到东西的假引用。
-- 等真的有了 blob 库再把它用起来。

ALTER TABLE send_queue ADD COLUMN body_text TEXT;
ALTER TABLE send_queue ADD COLUMN body_html TEXT;
ALTER TABLE send_queue ADD COLUMN message_bytes BLOB;
ALTER TABLE send_queue ADD COLUMN message_id TEXT;
ALTER TABLE send_queue ADD COLUMN idempotency_key TEXT;

-- 同一个幂等键只能有一行：这是「不会重复投递」的**最后一道闸**。
-- 部分索引（排除空串与 NULL）是给历史行留的：0001 时还没有这一列。
CREATE UNIQUE INDEX idx_send_queue_idempotency
    ON send_queue (account_id, idempotency_key)
    WHERE idempotency_key IS NOT NULL AND idempotency_key <> '';

-- 发信附件：原始字节直接内联（进 MIME 之前不编码，编码是构建那一步的事）。
-- 随 send_queue 级联删除：草稿删了，附件没有存在的道理。
CREATE TABLE send_attachment (
    id           TEXT    PRIMARY KEY NOT NULL,
    send_id      TEXT    NOT NULL REFERENCES send_queue (id) ON DELETE CASCADE,
    filename     TEXT    NOT NULL,
    content_type TEXT    NOT NULL,
    content      BLOB    NOT NULL,
    size         INTEGER NOT NULL,
    position     INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX idx_send_attachment_send ON send_attachment (send_id, position);
