-- 本地待提交操作的本地单调序号。
--
-- flag 意图的合并靠它定序：同一封信上多个意图时，序号大的覆盖序号小的
-- （见 ARCHITECTURE §6）。
--
-- 不能用 created_at 代替：同一毫秒里的两个意图会撞号，
-- 而定序必须是确定的。
--
-- DEFAULT 0 只是为了让 ALTER TABLE 能加 NOT NULL 列；
-- 真正的序号由存储层在插入时分配（同一账户 + 文件夹内递增）。
ALTER TABLE mail_operation ADD COLUMN seq INTEGER NOT NULL DEFAULT 0;

-- 按「账户 + 文件夹 + 状态」查待提交操作。
-- 同步层每轮都要查「这个文件夹有哪些还没提交的操作」。
CREATE INDEX idx_mail_operation_folder_state
    ON mail_operation (account_id, folder_id, state);

-- 同一账户 + 文件夹内，序号唯一。
CREATE UNIQUE INDEX idx_mail_operation_seq
    ON mail_operation (account_id, folder_id, seq);
