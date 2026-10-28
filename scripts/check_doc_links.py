#!/usr/bin/env python3
"""检查文档内部链接与源码注释里的相对路径是否断链。

**本地与 CI 用的是同一份实现**（`.github/workflows/ci.yml` 的 "Docs sanity" job
调用本文件），避免两处各写一套、久了就不一致。

跑法：

```text
python3 scripts/check_doc_links.py
```

退出码 0 = 全部通过；1 = 有断链（会把断链逐条列出来）。

## 为什么要有这个检查

文档里的 `[ARCHITECTURE §9](../../../../../docs/ARCHITECTURE.md)` 这种相对链接，
**层级写错一位也不会有人立刻发现**——它只在链接真正被点开时才暴露，
而仓库里没人会逐个点。所以让它成为一道自动闸门。

2026-10-26 就靠它逮到过一次：`apps/shell/src-tauri/src/secrets/mod.rs` 的
`../../../docs/ARCHITECTURE.md` 少写了一层 `../`，CI 直接红。
"""

from __future__ import annotations

import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent


def _skip_dirs(parts: tuple[str, ...]) -> bool:
    """扫描时跳过的目录名。"""
    return any(part in {"target", "node_modules", ".git"} for part in parts)


def check_markdown_links() -> list[str]:
    """检查 markdown 内部链接指向的文件是否存在。"""
    # 覆盖 docs/ 与根目录的 README/CHANGELOG，避免根目录链接无人校验。
    targets = sorted((ROOT / "docs").rglob("*.md"))
    targets += [p for p in (ROOT / "README.md", ROOT / "CHANGELOG.md") if p.exists()]

    bad: list[str] = []
    for md in targets:
        text = md.read_text(encoding="utf-8")
        # 去掉代码块，避免示例中的路径被误判为链接。
        text = re.sub(r"```.*?```", "", text, flags=re.S)
        for m in re.finditer(r"\[[^\]]*\]\(([^)#]+?)(?:#[^)]*)?\)", text):
            target = m.group(1)
            if target.startswith(("http://", "https://", "mailto:")):
                continue
            resolved = (md.parent / target).resolve()
            if not resolved.exists():
                bad.append(f"{md.relative_to(ROOT)}: {target}")
    return bad


def check_rust_doc_links() -> list[str]:
    """检查 Rust 源码注释里指向 docs/ 的相对路径（曾经全部断链）。"""
    bad: list[str] = []
    for rs in ROOT.rglob("*.rs"):
        if _skip_dirs(rs.parts):
            continue
        text = rs.read_text(encoding="utf-8")
        for m in re.finditer(r"\((\.\.?/[^)\s]+)\)", text):
            resolved = (rs.parent / m.group(1)).resolve()
            if not resolved.exists():
                bad.append(f"{rs.relative_to(ROOT)}: {m.group(1)}")
    return bad


def main() -> int:
    failed = False

    bad_md = check_markdown_links()
    if bad_md:
        print("Broken internal links:")
        print("\n".join(bad_md))
        failed = True

    bad_rs = check_rust_doc_links()
    if bad_rs:
        print("Broken doc-comment links:")
        print("\n".join(bad_rs))
        failed = True

    if failed:
        return 1

    md_count = len(sorted((ROOT / "docs").rglob("*.md")))
    print(f"All internal doc links OK ({md_count} md files)")
    print("All Rust doc-comment links OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
