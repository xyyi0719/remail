import type { Folder } from "../types";

interface Props {
  folders: Folder[];
  selectedId: string | null;
  onSelect: (folder: Folder) => void;
}

/** 文件夹列表。名字是**解码后**的（「草稿箱」而不是 `&XfJT0ZAB-`）。 */
function FolderList({ folders, selectedId, onSelect }: Props) {
  if (folders.length === 0) {
    return (
      <section className="panel">
        <h2 className="panel-title">文件夹</h2>
        <p className="panel-empty">还没有同步过。先点账户上的「同步」。</p>
      </section>
    );
  }

  return (
    <section className="panel">
      <h2 className="panel-title">文件夹（{folders.length}）</h2>
      <ul className="folder-list">
        {folders.map((folder) => (
          <li key={folder.id}>
            <button
              type="button"
              className={
                folder.id === selectedId
                  ? "folder-item folder-item-active"
                  : "folder-item"
              }
              onClick={() => onSelect(folder)}
            >
              <span className="folder-name">{folder.name}</span>
              {folder.uidvalidity === null && (
                <span className="folder-detail">未同步</span>
              )}
            </button>
          </li>
        ))}
      </ul>
    </section>
  );
}

export default FolderList;
