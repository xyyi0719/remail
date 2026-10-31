import { useState } from "react";
import { invoke } from "@tauri-apps/api/core";

/**
 * 一条搜索命中（与后端 `SearchHitDto` 一一对应）。
 *
 * 自己声明一份，而不是塞进 `types.ts`：
 * `types.ts` 是几个并行任务共用的文件，各自往里加类型容易互相覆盖。
 */
export interface SearchHit {
  messageId: string;
  accountId: string;
  folderId: string;
  uid: number;
  subject: string;
  from: string;
  dateSent: number | null;
  snippet: string;
}

interface Props {
  /** 点开一条命中（由 App 切到对应文件夹并打开那封邮件）。 */
  onOpen: (hit: SearchHit) => void;
  onError: (message: string) => void;
}

/**
 * 本地全文搜索面板。
 *
 * 搜的是**本地缓存**（SQLite FTS5），不查服务器——
 * 163 的 `UID SEARCH` 只认 `ALL`，其它关键字静默失效（风险 R23）。
 * 所以搜不到还没同步下来的邮件，这一点在界面上说清楚。
 */
function SearchPanel({ onOpen, onError }: Props) {
  const [query, setQuery] = useState("");
  const [hits, setHits] = useState<SearchHit[] | null>(null);
  const [searching, setSearching] = useState(false);

  const search = async (text: string) => {
    const trimmed = text.trim();
    if (trimmed === "") {
      setHits(null);
      return;
    }
    setSearching(true);
    try {
      const results = await invoke<SearchHit[]>("search_messages", {
        query: trimmed,
        limit: 50,
      });
      setHits(results);
    } catch (error) {
      setHits(null);
      onError(String(error));
    } finally {
      setSearching(false);
    }
  };

  return (
    <section className="panel search-panel">
      <h2 className="panel-title">搜索</h2>
      <form
        className="search-form"
        onSubmit={(event) => {
          event.preventDefault();
          void search(query);
        }}
      >
        <input
          type="search"
          value={query}
          placeholder="搜本地缓存里的邮件"
          aria-label="搜索邮件"
          onChange={(event) => setQuery(event.target.value)}
        />
        <button type="submit" disabled={searching || query.trim() === ""}>
          {searching ? "搜着…" : "搜索"}
        </button>
      </form>

      {hits !== null && (
        <p className="panel-hint">
          {hits.length === 0
            ? "没找到（只搜已经同步到本地的邮件）。"
            : `找到 ${hits.length} 封。`}
        </p>
      )}

      {hits !== null && hits.length > 0 && (
        <ul className="search-results">
          {hits.map((hit) => (
            <li key={hit.messageId} className="search-result">
              <button
                type="button"
                className="search-result-button"
                onClick={() => onOpen(hit)}
                data-testid="search-result"
              >
                <span className="search-result-subject">
                  {hit.subject || "（无主题）"}
                </span>
                <span className="search-result-meta">
                  {hit.from || "（未知发件人）"}
                </span>
                <Snippet text={hit.snippet} />
              </button>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

/**
 * 命中片段。
 *
 * 后端把命中词用 `**` 包起来（`Subject` 与 `Body` 两列都这么标），
 * 这里按 `**` 切开、奇数段加粗高亮。
 */
function Snippet({ text }: { text: string }) {
  const parts = text.split("**");
  return (
    <span className="search-result-snippet">
      {parts.map((part, index) =>
        index % 2 === 1 ? (
          <mark key={index}>{part}</mark>
        ) : (
          <span key={index}>{part}</span>
        ),
      )}
    </span>
  );
}

export default SearchPanel;
