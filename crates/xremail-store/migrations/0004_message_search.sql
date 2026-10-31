-- 本地全文搜索（SQLite FTS5）。
--
-- 背景：远端 IMAP 的搜索能力不可靠——163 的 UID SEARCH 只认 ALL，
-- 其它关键字静默失效（已知风险 R23）。所以搜索必须落在**本地缓存**上。
--
-- 结构（三张对象 + 五组触发器）：
--   message_search_doc       「文档表」：一封邮件一行，存可搜索的投影
--                            （标题 / 收发件人 / 纯文本正文）。
--   message_search_fts       FTS5 **外部内容表**：倒排索引单独存放，
--                            正文只在文档表里存一份，不存两遍。
--   message_search_people    视图：把 message 的三个地址列拼成一个人类可读的
--                            「谁参与了这封邮件」字符串。
--   doc ↔ fts 触发器          文档表与 FTS 表之间的增删改同步。
--   message / message_body   邮件写入与删除时同步文档表——
--     触发器                  **索引因此永远跟着 message 表走**：
--                            任何写入路径（DAO、同步、epoch 切换、级联删除）
--                            都绕不过触发器，不会留下与 message 表脱节的索引。
--
-- 分词器必须是 trigram：中文正文没有空格，SQLite 默认的 unicode61 会把
-- 整句中文当成**一个词**，切不出子串，「项目进度」搜不到「进度」；
-- trigram 按 3 字符滑窗切分，才能按中文短语命中。代价是索引更大
-- （每个 3 字符滑窗一个词元），并且**短于 3 字符的词元服务不了**
-- ——DAO 层对这类查询退化成 LIKE 扫描。
--
-- 不索引 html_raw：原始 HTML 里的标签、样式、内联脚本是噪音，
-- 命中会被标签切碎，片段也是一堆尖括号。只索引 text_plain。

-- ---------------------------------------------------------------------------
-- 「谁参与了这封邮件」：from + to + cc 拼成一个字符串。
--
-- 单独建视图是为了让这段 json_each 拼装逻辑**只写一次**：
-- 下面的触发器与迁移末尾的回填都要用它。
--
-- 只取 from/to/cc，**不取 bcc**：密送不该被当成可检索的收件人。
-- json_valid 兜底：地址列是 JSON 文本，理论上一直合法，
-- 但万一被外部写坏，这里必须退化成空串而不是让整封邮件写不进去。
-- ---------------------------------------------------------------------------
CREATE VIEW message_search_people AS
SELECT
    m.id AS message_id,
    COALESCE(
        (
            SELECT group_concat(part, ' ')
            FROM (
                SELECT TRIM(
                    COALESCE(json_extract(fj.value, '$.name'), '') || ' ' ||
                    COALESCE(json_extract(fj.value, '$.email'), '')
                ) AS part
                FROM json_each(
                    CASE WHEN json_valid(m.from_addr) THEN m.from_addr ELSE '[]' END
                ) AS fj
                UNION ALL
                SELECT TRIM(
                    COALESCE(json_extract(tj.value, '$.name'), '') || ' ' ||
                    COALESCE(json_extract(tj.value, '$.email'), '')
                )
                FROM json_each(
                    CASE WHEN json_valid(m.to_addr) THEN m.to_addr ELSE '[]' END
                ) AS tj
                UNION ALL
                SELECT TRIM(
                    COALESCE(json_extract(cj.value, '$.name'), '') || ' ' ||
                    COALESCE(json_extract(cj.value, '$.email'), '')
                )
                FROM json_each(
                    CASE WHEN json_valid(m.cc_addr) THEN m.cc_addr ELSE '[]' END
                ) AS cj
            ) AS parts
            WHERE parts.part <> ''
        ),
        ''
    ) AS people
FROM message AS m;

-- ---------------------------------------------------------------------------
-- 文档表：一封邮件一行。
--
-- 外键指向 message：邮件被删时文档行跟着删（缓存数据没有独立价值）。
-- ---------------------------------------------------------------------------
CREATE TABLE message_search_doc (
    message_id TEXT    PRIMARY KEY NOT NULL REFERENCES message (id) ON DELETE CASCADE,
    subject    TEXT    NOT NULL DEFAULT '',
    people     TEXT    NOT NULL DEFAULT '',
    body       TEXT    NOT NULL DEFAULT ''
);

-- ---------------------------------------------------------------------------
-- FTS5 外部内容表。
--
-- content / content_rowid 指向文档表：FTS 表只存倒排索引，
-- 取值时回文档表读，避免正文存两遍。
--
-- tokenize='trigram'：中文必须按 3 字符滑窗切分（见文件头注释）。
-- trigram 自 SQLite 3.34 起提供；本仓库用的是 sqlx 的 sqlite-bundled
-- （自带 3.51.x），始终可用。
-- ---------------------------------------------------------------------------
CREATE VIRTUAL TABLE message_search_fts USING fts5(
    subject,
    people,
    body,
    content='message_search_doc',
    content_rowid='rowid',
    tokenize='trigram'
);

-- ---------------------------------------------------------------------------
-- 文档表 ↔ FTS 表的同步。
--
-- 外部内容表的 'delete' 命令必须带上**被删掉那一行的旧值**，
-- FTS5 才能正确地把对应词元从倒排索引里摘掉；因此这里一律用 old.*。
-- ---------------------------------------------------------------------------
CREATE TRIGGER message_search_doc_ai AFTER INSERT ON message_search_doc BEGIN
    INSERT INTO message_search_fts (rowid, subject, people, body)
    VALUES (new.rowid, new.subject, new.people, new.body);
END;

CREATE TRIGGER message_search_doc_ad AFTER DELETE ON message_search_doc BEGIN
    INSERT INTO message_search_fts (message_search_fts, rowid, subject, people, body)
    VALUES ('delete', old.rowid, old.subject, old.people, old.body);
END;

CREATE TRIGGER message_search_doc_au AFTER UPDATE ON message_search_doc BEGIN
    INSERT INTO message_search_fts (message_search_fts, rowid, subject, people, body)
    VALUES ('delete', old.rowid, old.subject, old.people, old.body);
    INSERT INTO message_search_fts (rowid, subject, people, body)
    VALUES (new.rowid, new.subject, new.people, new.body);
END;

-- ---------------------------------------------------------------------------
-- message → 文档表：这就是「索引跟着 message 表走」的同步点。
--
-- * 新邮件落库时立刻建文档行；
-- * 标题 / 收发件人变化时更新文档行；
-- * 邮件被删时删掉文档行（外键级联也会删，这里显式写一遍：
--   即使有人关掉了 foreign_keys，索引也不会留下孤儿）。
--
-- UPDATE 只监听 subject / from_addr / to_addr / cc_addr：
-- 改标记（flags）不该惊动搜索索引。
-- ---------------------------------------------------------------------------
CREATE TRIGGER message_search_message_ai AFTER INSERT ON message BEGIN
    INSERT INTO message_search_doc (message_id, subject, people, body)
    VALUES (
        new.id,
        COALESCE(new.subject, ''),
        COALESCE(
            (SELECT people FROM message_search_people WHERE message_id = new.id),
            ''
        ),
        COALESCE(
            (SELECT text_plain FROM message_body WHERE message_id = new.id),
            ''
        )
    );
END;

CREATE TRIGGER message_search_message_au
AFTER UPDATE OF subject, from_addr, to_addr, cc_addr ON message BEGIN
    UPDATE message_search_doc
    SET subject = COALESCE(new.subject, ''),
        people  = COALESCE(
            (SELECT people FROM message_search_people WHERE message_id = new.id),
            ''
        )
    WHERE message_id = new.id;
END;

CREATE TRIGGER message_search_message_ad AFTER DELETE ON message BEGIN
    DELETE FROM message_search_doc WHERE message_id = old.id;
END;

-- ---------------------------------------------------------------------------
-- message_body → 文档表：正文是单独一张表，正文到达时补上文档行的 body。
-- ---------------------------------------------------------------------------
CREATE TRIGGER message_search_body_ai AFTER INSERT ON message_body BEGIN
    UPDATE message_search_doc
    SET body = COALESCE(new.text_plain, '')
    WHERE message_id = new.message_id;
END;

CREATE TRIGGER message_search_body_au
AFTER UPDATE OF text_plain ON message_body BEGIN
    UPDATE message_search_doc
    SET body = COALESCE(new.text_plain, '')
    WHERE message_id = new.message_id;
END;

-- ---------------------------------------------------------------------------
-- 回填：迁移之前就存在的邮件没有文档行（触发器只对之后的写入生效）。
--
-- 没有这一步，老库升级上来后搜索会**一条都搜不到**，
-- 直到下一次同步把邮件重新写一遍。
-- ---------------------------------------------------------------------------
INSERT INTO message_search_doc (message_id, subject, people, body)
SELECT
    m.id,
    COALESCE(m.subject, ''),
    COALESCE(
        (SELECT people FROM message_search_people WHERE message_id = m.id),
        ''
    ),
    COALESCE(
        (SELECT text_plain FROM message_body WHERE message_id = m.id),
        ''
    )
FROM message AS m
WHERE NOT EXISTS (
    SELECT 1 FROM message_search_doc AS d WHERE d.message_id = m.id
);
