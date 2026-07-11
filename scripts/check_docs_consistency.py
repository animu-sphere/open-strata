#!/usr/bin/env python3
"""Documentation consistency + hygiene gate.

Fails when documentation contradicts its machine-readable sources or drifts into
sloppiness. Dependency-free (standard library only). Checks:

* **Crate inventory** — the crate table in docs/architecture/crates.md matches
  the workspace `members` in the root Cargo.toml.
* **Version consistency** — the workspace version has a release record, the
  releases index lists it, and the README names it as the current release.
* **Roadmap freshness** — no released version is still an active milestone in the
  roadmap, and no roadmap doc carries `status: completed`.
* **Markdown hygiene** — no trailing whitespace or tabs, and a final newline
  (generated third-party notices are exempt).

Usage: ``python3 scripts/check_docs_consistency.py [repo_root]`` (default: cwd).
"""
from __future__ import annotations

import os
import re
import sys

problems: list[str] = []


def fail(msg: str) -> None:
    problems.append(msg)


def read(root: str, rel: str) -> str:
    with open(os.path.join(root, rel), encoding="utf-8") as f:
        return f.read()


def version_tuple(v: str) -> tuple[int, ...]:
    return tuple(int(x) for x in v.split("."))


def check_crate_inventory(root: str) -> None:
    cargo = read(root, "Cargo.toml")
    members_block = re.search(r"members\s*=\s*\[(.*?)\]", cargo, re.DOTALL)
    if not members_block:
        fail("Cargo.toml: could not find workspace `members`")
        return
    members = set(re.findall(r'"crates/(ost-[a-z-]+)"', members_block.group(1)))

    crates_md = read(root, "docs/architecture/crates.md")
    table = set(re.findall(r"^\|\s*`(ost-[a-z-]+)`\s*\|", crates_md, re.MULTILINE))

    missing = members - table
    extra = table - members
    if missing:
        fail(f"crates.md is missing workspace crates: {sorted(missing)}")
    if extra:
        fail(f"crates.md lists crates not in Cargo.toml: {sorted(extra)}")


def workspace_version(root: str) -> str | None:
    cargo = read(root, "Cargo.toml")
    m = re.search(
        r"\[workspace\.package\].*?^version\s*=\s*\"([0-9]+\.[0-9]+\.[0-9]+)\"",
        cargo,
        re.DOTALL | re.MULTILINE,
    )
    return m.group(1) if m else None


def check_version_consistency(root: str) -> str | None:
    version = workspace_version(root)
    if not version:
        fail("Cargo.toml: could not find [workspace.package] version")
        return None

    if not os.path.exists(os.path.join(root, "docs/releases", f"v{version}.md")):
        fail(f"no release record docs/releases/v{version}.md for workspace version {version}")

    index = read(root, "docs/releases/README.md")
    if f"[v{version}.md]" not in index:
        fail(f"docs/releases/README.md does not list v{version}.md")

    readme = read(root, "README.md")
    m = re.search(r"current release is \*\*v([0-9]+\.[0-9]+\.[0-9]+)\*\*", readme)
    if not m:
        fail("README.md: could not find the current-release line")
    elif m.group(1) != version:
        fail(
            f"README current release v{m.group(1)} != workspace version v{version}"
        )
    return version


def check_roadmap_state(root: str, latest: str | None) -> None:
    roadmap_dir = os.path.join(root, "docs/roadmap")
    for name in os.listdir(roadmap_dir):
        if not name.endswith(".md"):
            continue
        text = read(root, f"docs/roadmap/{name}")
        if re.search(r"status:\s*completed", text):
            fail(f"docs/roadmap/{name}: uses `status: completed` (move to a release record)")
        if latest is None:
            continue
        # Active milestone bullets/headers: `- ⬜ **vX.Y.Z` / `## ... : vX.Y.Z`.
        for m in re.finditer(r"^[-#].*?\bv([0-9]+\.[0-9]+\.[0-9]+)\b", text, re.MULTILINE):
            line = m.group(0)
            if not ("⬜" in line or "🚧" in line or "Next milestone" in line):
                continue
            if version_tuple(m.group(1)) <= version_tuple(latest):
                fail(
                    f"docs/roadmap/{name}: milestone v{m.group(1)} is <= latest "
                    f"release v{latest} — move it to a release record"
                )


EXEMPT_HYGIENE = {"THIRD_PARTY_NOTICES.md"}


def md_files(root: str) -> list[str]:
    files = [
        f
        for f in sorted(os.listdir(root))
        if f.endswith(".md") and os.path.isfile(os.path.join(root, f))
    ]
    files = [f for f in files if f not in EXEMPT_HYGIENE]
    out = [os.path.join(root, f) for f in files]
    for dp, _dns, fns in os.walk(os.path.join(root, "docs")):
        for fn in sorted(fns):
            if fn.endswith(".md"):
                out.append(os.path.join(dp, fn))
    return out


def check_hygiene(root: str) -> None:
    for path in md_files(root):
        rel = os.path.relpath(path, root).replace("\\", "/")
        with open(path, encoding="utf-8") as f:
            text = f.read()
        for i, line in enumerate(text.splitlines(), 1):
            if line != line.rstrip():
                fail(f"{rel}:{i}: trailing whitespace")
            if "\t" in line:
                fail(f"{rel}:{i}: tab character")
        if text and not text.endswith("\n"):
            fail(f"{rel}: no final newline")


def main() -> int:
    root = os.path.abspath(sys.argv[1] if len(sys.argv) > 1 else ".")
    check_crate_inventory(root)
    latest = check_version_consistency(root)
    check_roadmap_state(root, latest)
    check_hygiene(root)

    if problems:
        for p in problems:
            print(f"DOC-CHECK {p}")
        print(f"\n{len(problems)} documentation consistency problem(s)")
        return 1
    print("Documentation consistency + hygiene OK")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
