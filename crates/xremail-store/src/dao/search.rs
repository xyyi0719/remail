//! 本地全文搜索。
//!
//! 搜索落在**本地缓存**上：远端 IMAP 的搜索能力不可靠——163 的 UID SEARCH
//! 只认 ALL，其它关键字静默失效（已知风险 R23），所以不能把搜索外包给服务器。
//!
//! 索引本身由**数据库触发器**维护（迁移 `0004_message_search.sql`）：
//! `message` / `message_body` 的写入与删除会自动同步到 FTS 表，
//! 因此索引不会与 `message` 表脱节——包括 epoch 切换、级联删除这些
//! 不经过本模块的路径。本模块只负责「查」和「显式重建某一封」。
//!
//! ## 两条查询路径
//!
//! * **FTS5 路径**（查询词都 ≥ 3 个字符）：走倒排索引。
//!   每个词都被包成 FTS5 的**字符串字面量**（`"..."`，内部引号加倍），
//!   因此用户输入里的 `OR`、`*`、`-`、`:` 之类**不会**被当成 FTS5 语法，
//!   而是按字面匹配。多个词之间是 AND。
//! * **LIKE 路径**（查询词里有 1~2 个字符的词）：对文档表做 `LIKE` 扫描。
//!   trigram 索引切不出短于 3 个字符的词元（实测 `MATCH '"项目"'` 命中 0），
//!   只能退化。**为什么这样是对的**：trigram 索引查的是「词元序列」，
//!   2 字的词根本不存在于索引里；而 `LIKE '%项目%'` 查的是**子串**，
//!   与用户输入「项目」时的直觉一致。代价是全表扫描（个人邮箱量级可接受）。
//!
//! 查询词按空白切分后是 AND 关系：`foo OR bar` 找的是**同时**含有
//! 字面量 `foo`、`OR`、`bar` 的邮件，而不是「foo 或者 bar」。

use sqlx::FromRow;
use sqlx::QueryBuilder;
use sqlx::Sqlite;
use sqlx::SqlitePool;
use xremail_core::AppResult;

use crate::error::map_sqlx;

/// 一条搜索命中。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchHit {
    /// 命中的邮件本地 ID。
    pub message_id: String,
    /// 命中片段：命中词用 `**` 包裹标出。
    pub snippet: String,
}

/// 文档表的一行——搜索命中后取回来拼片段。
#[derive(Debug, Clone, FromRow)]
#[sqlx(rename_all = "snake_case")]
struct SearchDocRow {
    message_id: String,
    subject: String,
    people: String,
    body: String,
}

/// trigram 索引能表达的最小长度：3 个字符。
///
/// SQLite 的 trigram 分词器按 3 字符滑窗切词，短于 3 个字符的串
/// 切不出任何词元，索引里根本没有它。
const MIN_TRIGRAM_LEN: usize = 3;

/// 片段里命中位置前后各留多少字符。
const SNIPPET_RADIUS: usize = 60;

/// 片段最大长度（字符）。
const SNIPPET_MAX_CHARS: usize = 240;

/// FTS5 路径的查询。
///
/// `message_search_fts` **不能起别名**：FTS5 的 MATCH 左值必须写表全名，
/// 写成别名会报 `no such column`。文档表可以起别名。
const FTS_SEARCH_SQL: &str = r#"
    SELECT d.message_id, d.subject, d.people, d.body
    FROM message_search_fts
    JOIN message_search_doc AS d ON d.rowid = message_search_fts.rowid
    WHERE message_search_fts MATCH ?
    ORDER BY rank
    LIMIT ?
"#;

/// 把一封邮件写进搜索索引。
///
/// **幂等**：同一封邮件重复索引只会覆盖文档行，不会产生重复结果。
///
/// 正常写入路径不需要调用它——迁移 0004 的触发器会跟着 `message` /
/// `message_body` 的写入自动同步索引。本函数用于**显式重建**：
/// 换分词器、修数据、或从别的来源补索引。
///
/// `people` 是「发件人 / 收件人 / 抄送」拼成的可读字符串；
/// `body` 只放纯文本正文，**不要**放原始 HTML。
pub async fn index_message(
    pool: &SqlitePool,
    message_id: &str,
    subject: &str,
    people: &str,
    body: &str,
) -> AppResult<()> {
    sqlx::query(
        r#"
        INSERT INTO message_search_doc (message_id, subject, people, body)
        VALUES (?, ?, ?, ?)
        ON CONFLICT (message_id) DO UPDATE SET
            subject = excluded.subject,
            people  = excluded.people,
            body    = excluded.body
        "#,
    )
    .bind(message_id)
    .bind(subject)
    .bind(people)
    .bind(body)
    .execute(pool)
    .await
    .map_err(|e| map_sqlx(e, "本地搜索：写入索引"))?;

    Ok(())
}

/// 把一封邮件从搜索索引里摘掉。
///
/// 删邮件时索引会自动跟着删（触发器 + 外键级联），本函数用于
/// 「邮件还在，但不想让它被搜到」这种显式摘除。
pub async fn remove_message(pool: &SqlitePool, message_id: &str) -> AppResult<()> {
    sqlx::query("DELETE FROM message_search_doc WHERE message_id = ?")
        .bind(message_id)
        .execute(pool)
        .await
        .map_err(|e| map_sqlx(e, "本地搜索：删除索引"))?;

    Ok(())
}

/// **重建整个搜索索引**，返回重建后的邮件数。
///
/// **可重复**：跑一次、跑两次结果一样。
///
/// 正常情况下用不上——触发器会让索引一直跟着 `message` 表走。
/// 需要它的场景：
///
/// * 换了分词器（trigram → 别的）；
/// * 怀疑索引与 `message` 表脱节（`check_index_consistency` 会报出来）；
/// * 导入了别处来的老库。
///
/// 实现方式是「先清空文档表、再从 `message` + `message_body` 全量重建」：
/// 文档表 ↔ FTS 表的同步由触发器做，所以清空文档表就等于清空索引，
/// 不需要直接碰 FTS 表。
pub async fn rebuild_index(pool: &SqlitePool) -> AppResult<usize> {
    sqlx::query("DELETE FROM message_search_doc")
        .execute(pool)
        .await
        .map_err(|e| map_sqlx(e, "本地搜索：清空索引"))?;

    let result = sqlx::query(
        r#"
        INSERT INTO message_search_doc (message_id, subject, people, body)
        SELECT
            m.id,
            COALESCE(m.subject, ''),
            COALESCE((SELECT people FROM message_search_people WHERE message_id = m.id), ''),
            COALESCE((SELECT text_plain FROM message_body WHERE message_id = m.id), '')
        FROM message AS m
        "#,
    )
    .execute(pool)
    .await
    .map_err(|e| map_sqlx(e, "本地搜索：重建索引"))?;

    Ok(result.rows_affected() as usize)
}

/// 检查索引与 `message` 表是否一致，返回**不一致的说明**。
///
/// 返回空列表 = 一致。
///
/// 查三类脱节：
///
/// 1. `message` 里有邮件，但文档表里**没有**对应行（漏索引）；
/// 2. 文档表里有行，但 `message` 里**没有**对应邮件（孤儿行）；
/// 3. 文档表的 `body` 与 `message_body.text_plain` **对不上**（正文过期）。
///
/// 第 3 类是实际踩过的坑：正文晚到（先落邮件、后落正文）时，
/// 如果索引没跟着更新，搜标题能中、搜正文内容中不了。
pub async fn check_index_consistency(pool: &SqlitePool) -> AppResult<Vec<String>> {
    let missing = sqlx::query_scalar::<_, String>(
        r#"
        SELECT m.id
        FROM message AS m
        LEFT JOIN message_search_doc AS d ON d.message_id = m.id
        WHERE d.message_id IS NULL
        "#,
    )
    .fetch_all(pool)
    .await
    .map_err(|e| map_sqlx(e, "本地搜索：检查漏索引"))?;

    let orphan = sqlx::query_scalar::<_, String>(
        r#"
        SELECT d.message_id
        FROM message_search_doc AS d
        LEFT JOIN message AS m ON m.id = d.message_id
        WHERE m.id IS NULL
        "#,
    )
    .fetch_all(pool)
    .await
    .map_err(|e| map_sqlx(e, "本地搜索：检查孤儿行"))?;

    let stale_body = sqlx::query_scalar::<_, String>(
        r#"
        SELECT d.message_id
        FROM message_search_doc AS d
        JOIN message_body AS b ON b.message_id = d.message_id
        WHERE d.body <> COALESCE(b.text_plain, '')
        "#,
    )
    .fetch_all(pool)
    .await
    .map_err(|e| map_sqlx(e, "本地搜索：检查正文过期"))?;

    let mut problems = Vec::new();
    for row in missing {
        problems.push(format!("漏索引：{row}"));
    }
    for row in orphan {
        problems.push(format!("孤儿索引：{row}"));
    }
    for row in stale_body {
        problems.push(format!("正文过期：{row}"));
    }

    Ok(problems)
}

/// 搜索本地缓存里的邮件。
///
/// 返回顺序：FTS5 路径按相关度（`rank`），LIKE 路径按邮件时间倒序。
/// 空查询返回空结果。
pub async fn search(pool: &SqlitePool, query: &str, limit: usize) -> AppResult<Vec<SearchHit>> {
    let terms: Vec<String> = query.split_whitespace().map(str::to_string).collect();
    if terms.is_empty() || limit == 0 {
        return Ok(Vec::new());
    }

    let rows = if terms.iter().all(|t| is_trigram_expressible(t)) {
        search_by_fts(pool, &terms, limit).await?
    } else {
        search_by_like(pool, &terms, limit).await?
    };

    Ok(rows
        .into_iter()
        .map(|row| {
            let snippet = build_snippet(&row, &terms);
            SearchHit {
                message_id: row.message_id,
                snippet,
            }
        })
        .collect())
}

/// 这个词能不能交给 trigram 索引。
///
/// 判据是**字符数**（不是字节数）：「项目」是 2 个字符、6 个字节，
/// 但 trigram 索引的是字符，2 个字符切不出词元。
fn is_trigram_expressible(term: &str) -> bool {
    term.chars().count() >= MIN_TRIGRAM_LEN
}

/// FTS5 路径：走倒排索引。
async fn search_by_fts(
    pool: &SqlitePool,
    terms: &[String],
    limit: usize,
) -> AppResult<Vec<SearchDocRow>> {
    let match_query = build_match_query(terms);

    let rows = sqlx::query_as::<_, SearchDocRow>(FTS_SEARCH_SQL)
        .bind(match_query)
        .bind(limit as i64)
        .fetch_all(pool)
        .await
        .map_err(|e| map_sqlx(e, "本地搜索：查询"))?;

    Ok(rows)
}

/// 把查询词变成一条 FTS5 表达式。
///
/// 每个词都包成字符串字面量：FTS5 里双引号内的内容是**纯字符串**，
/// 语法符号（`OR`、`*`、`-`、`:`、`^` …）在里面没有特殊含义。
/// 词内部的引号按 FTS5 的规则**加倍**转义。
///
/// 这样用户输入永远不会变成 FTS5 语法：`foo OR bar` 查的是
/// 同时含有字面量 `foo`、`OR`、`bar` 的邮件。
fn build_match_query(terms: &[String]) -> String {
    terms
        .iter()
        .map(|term| format!("\"{}\"", term.replace('"', "\"\"")))
        .collect::<Vec<_>>()
        .join(" AND ")
}

/// LIKE 路径：全表扫描。
///
/// 只在查询词里有 trigram 服务不了的短词时才会走到这里。
async fn search_by_like(
    pool: &SqlitePool,
    terms: &[String],
    limit: usize,
) -> AppResult<Vec<SearchDocRow>> {
    // 条件是固定片段的重复（每个词一组），用户输入一律走绑定参数，
    // 因此用 QueryBuilder 拼条件而不是拼字符串。
    let mut builder: QueryBuilder<Sqlite> = QueryBuilder::new(
        "SELECT d.message_id, d.subject, d.people, d.body \
         FROM message_search_doc AS d \
         JOIN message AS m ON m.id = d.message_id \
         WHERE ",
    );

    for (index, term) in terms.iter().enumerate() {
        if index > 0 {
            builder.push(" AND ");
        }
        // 占位符由 push_bind 自己插入，片段里不能带 `?`。
        let pattern = like_pattern(term);
        builder.push("(d.subject LIKE ");
        builder.push_bind(pattern.clone());
        builder.push(" ESCAPE '\\' OR d.people LIKE ");
        builder.push_bind(pattern.clone());
        builder.push(" ESCAPE '\\' OR d.body LIKE ");
        builder.push_bind(pattern);
        builder.push(" ESCAPE '\\')");
    }

    builder
        .push(" ORDER BY m.date_sent DESC, d.message_id LIMIT ")
        .push_bind(limit as i64);

    let rows = builder
        .build_query_as::<SearchDocRow>()
        .fetch_all(pool)
        .await
        .map_err(|e| map_sqlx(e, "本地搜索：扫描查询"))?;

    Ok(rows)
}

/// 把查询词变成 LIKE 的匹配串。
///
/// `%` 与 `_` 在 LIKE 里是通配符，必须转义，否则用户输入的 `_`
/// 会变成「匹配任意一个字符」。
fn like_pattern(term: &str) -> String {
    let mut pattern = String::with_capacity(term.len() + 2);
    pattern.push('%');
    for ch in term.chars() {
        if matches!(ch, '%' | '_' | '\\') {
            pattern.push('\\');
        }
        pattern.push(ch);
    }
    pattern.push('%');
    pattern
}

/// 拼片段。
///
/// 正文优先（给命中位置前后的片段），其次标题，最后是地址；
/// 命中词用 `**` 包裹标出。
fn build_snippet(row: &SearchDocRow, terms: &[String]) -> String {
    for term in terms {
        if let Some(pos) = find_first(&row.body, term) {
            let (window, cut_head, cut_tail) = window_around(&row.body, pos, term.len());
            let mut snippet = String::new();
            if cut_head {
                snippet.push('…');
            }
            snippet.push_str(&mark_terms(&window, terms));
            if cut_tail {
                snippet.push('…');
            }
            return snippet;
        }
    }

    for term in terms {
        if find_first(&row.subject, term).is_some() {
            return mark_terms(&truncate(&row.subject, SNIPPET_MAX_CHARS), terms);
        }
    }

    if terms.iter().any(|t| find_first(&row.people, t).is_some()) {
        return mark_terms(&truncate(&row.people, SNIPPET_MAX_CHARS), terms);
    }

    String::new()
}

/// 找第一次出现的位置（ASCII 大小写不敏感）。
///
/// 用 `to_ascii_lowercase` 而不是 `to_lowercase`：前者**不改变字节长度**，
/// 所以命中位置在原串里同样成立，可以直接拿去切片。
/// 这与 FTS5 的默认行为一致（`case_sensitive` 关闭时也只折叠 ASCII）。
fn find_first(haystack: &str, needle: &str) -> Option<usize> {
    if needle.is_empty() {
        return None;
    }
    haystack
        .to_ascii_lowercase()
        .find(&needle.to_ascii_lowercase())
}

/// 取命中位置前后的一段。
///
/// 返回 `(片段, 头部被截断, 尾部被截断)`；片段边界会对齐到字符边界，
/// 免得把一个多字节字符切成两半。
fn window_around(text: &str, pos: usize, len: usize) -> (String, bool, bool) {
    let raw_start = pos.saturating_sub(SNIPPET_RADIUS);
    let raw_end = (pos + len).saturating_add(SNIPPET_RADIUS).min(text.len());
    let start = floor_char_boundary(text, raw_start);
    let end = ceil_char_boundary(text, raw_end);

    (text[start..end].to_string(), start > 0, end < text.len())
}

/// 向下对齐到字符边界。
fn floor_char_boundary(text: &str, mut index: usize) -> usize {
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

/// 向上对齐到字符边界。
fn ceil_char_boundary(text: &str, mut index: usize) -> usize {
    while index < text.len() && !text.is_char_boundary(index) {
        index += 1;
    }
    index
}

/// 截断到指定字符数。
fn truncate(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let head: String = text.chars().take(max_chars).collect();
    format!("{head}…")
}

/// 把命中的词用 `**` 包裹标出来。
///
/// 从左往右扫，命中就整段包起来（长词优先），因此不会出现嵌套标记。
fn mark_terms(text: &str, terms: &[String]) -> String {
    let mut needles: Vec<(String, usize)> = terms
        .iter()
        .filter(|term| !term.is_empty())
        .map(|term| (term.to_ascii_lowercase(), term.len()))
        .collect();
    // 长词优先：先匹配到长词，短词就不会再切一遍。
    needles.sort_by_key(|needle| std::cmp::Reverse(needle.0.len()));

    if needles.is_empty() {
        return text.to_string();
    }

    let lower = text.to_ascii_lowercase();
    let mut out = String::with_capacity(text.len() + 16);
    let mut index = 0;

    while index < text.len() {
        let matched = needles
            .iter()
            .find(|(needle, _)| lower[index..].starts_with(needle.as_str()))
            .map(|(_, len)| *len);

        match matched {
            Some(len) => {
                out.push_str("**");
                out.push_str(&text[index..index + len]);
                out.push_str("**");
                index += len;
            }
            None => {
                let ch = text[index..].chars().next().unwrap_or('?');
                out.push(ch);
                index += ch.len_utf8();
            }
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trigram_expressible_counts_characters_not_bytes() {
        // 「项目」是 2 个字符（6 字节），trigram 服务不了。
        assert!(!is_trigram_expressible("项目"));
        // 「项目进度」是 4 个字符，可以。
        assert!(is_trigram_expressible("项目进度"));
        assert!(is_trigram_expressible("abc"));
        assert!(!is_trigram_expressible("ab"));
    }

    #[test]
    fn match_query_quotes_every_term() {
        let query = build_match_query(&["foo".to_string(), "OR".to_string()]);

        assert_eq!(query, "\"foo\" AND \"OR\"");
    }

    #[test]
    fn match_query_escapes_inner_quotes() {
        let query = build_match_query(&["a\"b".to_string()]);

        // 词内的引号加倍，外面再包一层。
        assert_eq!(query, "\"a\"\"b\"");
    }

    #[test]
    fn like_pattern_escapes_wildcards() {
        assert_eq!(like_pattern("100%"), "%100\\%%");
        assert_eq!(like_pattern("a_b"), "%a\\_b%");
        assert_eq!(like_pattern("中文"), "%中文%");
    }

    #[test]
    fn snippet_marks_the_hit() {
        let row = SearchDocRow {
            message_id: "m1".to_string(),
            subject: "项目进度报告".to_string(),
            people: "alice@example.com".to_string(),
            body: "这是一封关于项目进度的中文邮件，用来验证片段。".to_string(),
        };

        let snippet = build_snippet(&row, &["项目进度".to_string()]);

        assert!(
            snippet.contains("**项目进度**"),
            "片段应标出命中词: {snippet}"
        );
    }

    #[test]
    fn snippet_falls_back_to_subject_then_people() {
        let row = SearchDocRow {
            message_id: "m1".to_string(),
            subject: "预算超支说明".to_string(),
            people: "bob@example.com".to_string(),
            body: "正文里没有关键词".to_string(),
        };

        let subject_hit = build_snippet(&row, &["预算".to_string()]);
        assert!(subject_hit.contains("**预算**"), "{subject_hit}");

        let people_hit = build_snippet(&row, &["bob@example.com".to_string()]);
        assert!(people_hit.contains("**bob@example.com**"), "{people_hit}");
    }
}
