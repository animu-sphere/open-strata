#!/usr/bin/env python3
"""Validate relative links and heading anchors across the repository's Markdown.

Checks every root-level ``*.md`` and everything under ``docs/`` for:

* relative link targets that do not exist on disk (files, images, schemas);
* ``#anchor`` fragments that do not match a heading in the target document,
  using GitHub's heading-slug rules.

External links (``http(s)://``, ``mailto:``) are not fetched. Links inside
fenced or inline code are ignored, matching how GitHub renders them.

Usage: ``python3 scripts/check_doc_links.py [repo_root]`` (default: cwd).
Exits non-zero if any relative link or anchor is broken.
"""
from __future__ import annotations

import os
import re
import sys

LINK = re.compile(r"\[[^\]]*\]\(([^)]+)\)")
ATX = re.compile(r"^(#{1,6})\s+(.*?)\s*#*\s*$")


def gh_slug(text: str) -> str:
    """Approximate GitHub's heading -> anchor slug algorithm."""
    t = re.sub(r"`([^`]*)`", r"\1", text)
    t = re.sub(r"\[([^\]]*)\]\([^)]*\)", r"\1", t)
    t = t.lower()
    t = re.sub(r"[^\w\s-]", "", t, flags=re.UNICODE)
    t = t.strip()
    return re.sub(r"\s", "-", t)


_anchor_cache: dict[str, set[str]] = {}


def anchors_of(path: str) -> set[str]:
    if path in _anchor_cache:
        return _anchor_cache[path]
    counts: dict[str, int] = {}
    try:
        with open(path, encoding="utf-8") as fh:
            lines = fh.readlines()
    except OSError:
        _anchor_cache[path] = set()
        return _anchor_cache[path]
    in_fence = False
    for ln in lines:
        if ln.lstrip().startswith("```"):
            in_fence = not in_fence
            continue
        if in_fence:
            continue
        m = ATX.match(ln.rstrip("\n"))
        if m:
            s = gh_slug(m.group(2))
            counts[s] = counts.get(s, 0) + 1
    out: set[str] = set()
    for s, c in counts.items():
        out.add(s)
        for i in range(1, c):  # GitHub disambiguates repeats with -1, -2, ...
            out.add(f"{s}-{i}")
    _anchor_cache[path] = out
    return out


def md_files(root: str) -> list[str]:
    files = [
        os.path.join(root, fn)
        for fn in sorted(os.listdir(root))
        if fn.endswith(".md") and os.path.isfile(os.path.join(root, fn))
    ]
    docs = os.path.join(root, "docs")
    for dp, _dns, fns in os.walk(docs):
        for fn in sorted(fns):
            if fn.endswith(".md"):
                files.append(os.path.join(dp, fn))
    return files


def strip_code(text: str) -> str:
    text = re.sub(r"```.*?```", "", text, flags=re.DOTALL)
    return re.sub(r"`[^`]*`", "", text)


def main() -> int:
    root = os.path.abspath(sys.argv[1] if len(sys.argv) > 1 else ".")
    broken: list[tuple[str, str, str]] = []
    for f in md_files(root):
        with open(f, encoding="utf-8") as fh:
            text = strip_code(fh.read())
        for m in LINK.finditer(text):
            tgt = m.group(1).strip()
            if tgt.startswith(("http://", "https://", "mailto:")):
                continue
            if tgt.startswith("#"):
                anc = tgt[1:]
                if anc and anc not in anchors_of(f):
                    broken.append((f, tgt, "anchor-missing-in-file"))
                continue
            path, _, anchor = tgt.partition("#")
            if not path:
                continue
            resolved = os.path.normpath(os.path.join(os.path.dirname(f), path))
            if not os.path.exists(resolved):
                broken.append((f, tgt, "path-missing"))
                continue
            if anchor and resolved.endswith(".md"):
                if anchor not in anchors_of(resolved):
                    broken.append((f, tgt, "anchor-missing"))

    def rel(p: str) -> str:
        return os.path.relpath(p, root).replace("\\", "/")

    if broken:
        for f, tgt, why in broken:
            print(f"BROKEN [{why}] {rel(f)} -> {tgt}")
        print(f"\n{len(broken)} broken link(s)")
        return 1
    print("All relative links + anchors OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
