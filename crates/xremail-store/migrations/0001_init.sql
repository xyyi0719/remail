-- Xremail 初始 schema。
--
-- 本文件把 docs/ARCHITECTURE.md 的不变量直接编码进 schema，
-- 而不是只写在注释里：约束能在编译期之外真正拦住错误写入。
--
-- 约定：
-- * 表名 snake_case 复数（见 docs/CONTRIBUTING.md 命名规范）。
-- * 主键用本地生成的 UUID 字符串（TEXT），不依赖服务端 ID。
-- * 时间戳一律 epoch 毫秒（INTEGER）。
-- * 布尔列声明为 BOOLEAN，使 SQLite 亲和性映射到 Rust bool。
--
-- 删除策略**逐表声明**（ARCHITECTURE §2、§5）：
-- 缓存类数据随账户/文件夹级联删除；操作与发信记录虽然也级联，
-- 但它们对邮件只做**逻辑引用**（不设外键），因为 UIDVALIDITY 变化
-- 会让旧 UID 失效，此时应隔离旧操作而不是让它随邮件一起消失。

-- ===========================================================================
-- 账户
-- ===========================================================================
-- 凭据不落库：secret_ref 只是操作系统钥匙串中一条机密的不透明引用，
-- 不是机密本身（见 ARCHITECTURE §2、docs/SECURITY.md）。
CREATE TABLE account (
    id                 TEXT    PRIMARY KEY NOT NULL,
    email              TEXT    NOT NULL,
    provider           TEXT    NOT NULL,
    display_name       TEXT,
    color              TEXT,
    auth_kind          TEXT    NOT NULL,

    -- ServerConfig 展开存储：便于按主机名/端口建索引与诊断查询。
    imap_host          TEXT    NOT NULL,
    imap_port          INTEGER NOT NULL CHECK (imap_port BETWEEN 1 AND 65535),
    imap_tls           TEXT    NOT NULL,
    smtp_host          TEXT    NOT NULL,
    smtp_port          INTEGER NOT NULL CHECK (smtp_port BETWEEN 1 AND 65535),
    smtp_tls           TEXT    NOT NULL,
    smtp_auth          TEXT    NOT NULL,

    username           TEXT    NOT NULL,
    secret_ref         TEXT,
    oauth_client_id    TEXT,
    sync_mode          TEXT    NOT NULL,
    poll_interval_secs INTEGER NOT NULL CHECK (poll_interval_secs > 0),
    signature_html     TEXT,

    created_at         INTEGER NOT NULL,
    updated_at         INTEGER NOT NULL
);

-- ===========================================================================
-- 文件夹
-- ===========================================================================
-- 同步游标与 epoch（uidvalidity）放在这里，与 xremail-core 的 Folder 结构
-- 一一对应；**不再另建 sync_state 表**，避免出现同义重复字段
-- （ARCHITECTURE §4.3：无需重复添加同义字段）。
--
-- uidvalidity 是 epoch：服务器更换后必须在一个事务内切换并失效旧 UID 映射。
CREATE TABLE folder (
    id             TEXT    PRIMARY KEY NOT NULL,
    account_id     TEXT    NOT NULL REFERENCES account (id) ON DELETE CASCADE,
    remote_name    TEXT    NOT NULL,
    remote_id      TEXT,
    local_name     TEXT,
    delimiter      TEXT,
    subscribed     BOOLEAN NOT NULL DEFAULT 0,
    selectable     BOOLEAN NOT NULL DEFAULT 1,
    last_sync_at   INTEGER,
    last_uid       INTEGER CHECK (last_uid IS NULL OR last_uid >= 0),
    highest_modseq INTEGER CHECK (highest_modseq IS NULL OR highest_modseq >= 0),
    uidvalidity    INTEGER CHECK (uidvalidity IS NULL OR uidvalidity > 0),

    -- 同一账户下服务器侧的文件夹名唯一。
    UNIQUE (account_id, remote_name),
    -- 供 message 的复合外键引用，保证 message.account_id 与所属文件夹一致。
    UNIQUE (account_id, id)
);

-- ===========================================================================
-- 邮件信封
-- ===========================================================================
-- 正文放在 message_body 单独一张表：列文件夹时不需要碰大文本列
-- （与 xremail-core::model::Message 的文档注释一致）。
--
-- 身份不变量（ARCHITECTURE §4.1）：
-- 远端邮件定位使用 (account_id, folder_id, uidvalidity, uid)。
-- 本地 id 独立生成；RFC 5322 Message-ID **仅作线索**，
-- 它不保证唯一、可能缺失、跨文件夹也不唯一，因此**不建唯一约束**。
CREATE TABLE message (
    id             TEXT    PRIMARY KEY NOT NULL,
    account_id     TEXT    NOT NULL,
    folder_id      TEXT    NOT NULL,

    uid            INTEGER NOT NULL CHECK (uid > 0),
    uidvalidity    INTEGER NOT NULL CHECK (uidvalidity > 0),

    -- 仅线索，不得用作去重键。
    message_id     TEXT,
    subject        TEXT,
    snippet        TEXT,

    -- 地址列表以 JSON 数组存储（Address 结构体），不拆表：
    -- 读写总是整组取用，不存在单地址查询需求。
    from_addr      TEXT    NOT NULL DEFAULT '[]',
    to_addr        TEXT    NOT NULL DEFAULT '[]',
    cc_addr        TEXT    NOT NULL DEFAULT '[]',
    bcc_addr       TEXT    NOT NULL DEFAULT '[]',

    date_sent      INTEGER,
    internal_date  INTEGER,
    -- MessageFlags 位掩码，布局对齐 IMAP FLAGS。
    flags          INTEGER NOT NULL DEFAULT 0,
    size           INTEGER CHECK (size IS NULL OR size >= 0),
    has_attachment BOOLEAN NOT NULL DEFAULT 0,

    -- 身份不变量落到 schema 层。
    UNIQUE (account_id, folder_id, uidvalidity, uid),
    -- 复合外键：同时保证 folder 存在、且与 account_id 属于同一账户。
    -- 删文件夹 → 删其邮件（缓存数据，无独立价值）。
    FOREIGN KEY (account_id, folder_id)
        REFERENCES folder (account_id, id) ON DELETE CASCADE
);

CREATE INDEX idx_message_folder ON message (folder_id);
CREATE INDEX idx_message_account_date ON message (account_id, date_sent DESC);

-- ===========================================================================
-- 邮件正文
-- ===========================================================================
-- html_raw 是**未经清洗**的原始 HTML；清洗在渲染层完成
-- （见 xremail-core::model::MessageBody 文档注释与 docs/SECURITY.md）。
CREATE TABLE message_body (
    message_id TEXT    PRIMARY KEY NOT NULL REFERENCES message (id) ON DELETE CASCADE,
    text_plain TEXT,
    html_raw   TEXT,
    charset    TEXT
);

-- ===========================================================================
-- 附件
-- ===========================================================================
CREATE TABLE attachment (
    id         TEXT    PRIMARY KEY NOT NULL,
    message_id TEXT    NOT NULL REFERENCES message (id) ON DELETE CASCADE,
    filename   TEXT,
    mime       TEXT,
    size       INTEGER CHECK (size IS NULL OR size >= 0),
    content_id TEXT,
    inline     BOOLEAN NOT NULL DEFAULT 0,
    local_path TEXT,
    sha256     TEXT,
    fetched    BOOLEAN NOT NULL DEFAULT 0
);

CREATE INDEX idx_attachment_message ON attachment (message_id);

-- ===========================================================================
-- 本地操作队列（flag / move / delete / append）
-- ===========================================================================
-- 与 send_queue 分表：两者的重试语义完全不同，混在一个 outbox 里
-- 会让"不确定结果"的处理规则无法分别表达（ARCHITECTURE §5）。
--
-- 对邮件只做逻辑引用（不设外键）：epoch 切换后旧 UID 失效，
-- 这些操作应被**隔离**并保留冲突原因，而不是随邮件一起被级联删除
-- （ARCHITECTURE §4.3）。
CREATE TABLE mail_operation (
    id           TEXT    PRIMARY KEY NOT NULL,
    account_id   TEXT    NOT NULL REFERENCES account (id) ON DELETE CASCADE,
    folder_id    TEXT    NOT NULL,
    uid          INTEGER NOT NULL CHECK (uid > 0),
    uidvalidity  INTEGER NOT NULL CHECK (uidvalidity > 0),

    kind         TEXT    NOT NULL,
    intent       TEXT    NOT NULL,
    -- queued / running / succeeded / retryable / failed / uncertain
    state        TEXT    NOT NULL,
    -- 已知的服务器 MODSEQ 基线；无 CONDSTORE 时为 NULL。
    base_version INTEGER,
    attempts     INTEGER NOT NULL DEFAULT 0,
    error_kind   TEXT,
    error_message TEXT,

    created_at   INTEGER NOT NULL,
    updated_at   INTEGER NOT NULL
);

CREATE INDEX idx_mail_operation_pending
    ON mail_operation (account_id, state, created_at);

-- ===========================================================================
-- 发信队列（SMTP）
-- ===========================================================================
-- envelope（实际投递地址）与 headers（显示用 To/Cc/Subject）分离，
-- 保证 Bcc 不泄漏且不破坏 To/Cc（ARCHITECTURE §5、P3 要求）。
CREATE TABLE send_queue (
    id            TEXT    PRIMARY KEY NOT NULL,
    account_id    TEXT    NOT NULL REFERENCES account (id) ON DELETE CASCADE,
    envelope      TEXT    NOT NULL,
    headers       TEXT    NOT NULL,
    body_ref      TEXT,
    -- queued / running / succeeded / retryable / failed / uncertain
    state         TEXT    NOT NULL,
    attempts      INTEGER NOT NULL DEFAULT 0,
    error_kind    TEXT,
    error_message TEXT,
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL
);

CREATE INDEX idx_send_queue_pending ON send_queue (account_id, state, created_at);

-- ===========================================================================
-- 本地机密（版本化密文）
-- ===========================================================================
-- 只存**密文**：不存明文凭据，也不存可直接使用的密钥。
-- KDF 参数与 salt 一并持久化，密文格式自带版本号，
-- 以便日后提升 KDF 成本时仍能解开旧数据（ARCHITECTURE §8）。
--
-- 与操作系统钥匙串（SecretStore）是两回事：本表是 vault 的本地落盘部分。
CREATE TABLE local_secret (
    id          TEXT    PRIMARY KEY NOT NULL,
    -- 可空：vault 条目未必绑定某个账户。
    account_id  TEXT    REFERENCES account (id) ON DELETE CASCADE,
    kind        TEXT    NOT NULL,
    version     INTEGER NOT NULL CHECK (version > 0),
    kdf_params  TEXT    NOT NULL,
    salt        BLOB    NOT NULL,
    nonce       BLOB    NOT NULL,
    ciphertext  BLOB    NOT NULL,
    created_at  INTEGER NOT NULL,
    updated_at  INTEGER NOT NULL
);

CREATE INDEX idx_local_secret_account ON local_secret (account_id);

-- ===========================================================================
-- 标签与规则（L2 预留）
-- ===========================================================================
-- P0 只预留稳定 ID、schema 版本与必要生命周期字段，
-- 不提前冻结复杂的 CRDT / 同步协议（ARCHITECTURE §8）。
CREATE TABLE label (
    id             TEXT    PRIMARY KEY NOT NULL,
    account_id     TEXT    NOT NULL REFERENCES account (id) ON DELETE CASCADE,
    name           TEXT    NOT NULL,
    color          TEXT,
    schema_version INTEGER NOT NULL DEFAULT 1,
    created_at     INTEGER NOT NULL,
    updated_at     INTEGER NOT NULL,
    UNIQUE (account_id, name)
);

CREATE TABLE rule (
    id             TEXT    PRIMARY KEY NOT NULL,
    account_id     TEXT    NOT NULL REFERENCES account (id) ON DELETE CASCADE,
    name           TEXT    NOT NULL,
    condition      TEXT    NOT NULL,
    action         TEXT    NOT NULL,
    enabled        BOOLEAN NOT NULL DEFAULT 1,
    schema_version INTEGER NOT NULL DEFAULT 1,
    created_at     INTEGER NOT NULL,
    updated_at     INTEGER NOT NULL
);

CREATE INDEX idx_rule_account ON rule (account_id);
