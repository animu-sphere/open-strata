#!/usr/bin/env python3
"""Expand the canonical OpenUSD support declaration into producer jobs."""

from __future__ import annotations

import argparse
import json
from pathlib import Path

CANONICAL = {
    "linux": {"core", "gl", "vulkan"},
    "windows": {"core", "gl", "vulkan"},
    "macos": {"core", "gl", "metal"},
}


def expand(document: dict[str, object]) -> list[dict[str, object]]:
    if document.get("schema") != 1 or document.get("profile") != "usd":
        raise ValueError("matrix must use schema 1 and profile 'usd'")
    versions = document.get("versions")
    cells = document.get("cells")
    if versions != ["26.05", "26.08"] or not isinstance(cells, list):
        raise ValueError("matrix must declare OpenUSD 26.05 and 26.08 in order")
    jobs: list[dict[str, object]] = []
    for cell in cells:
        if not isinstance(cell, dict):
            raise ValueError("each matrix cell must be an object")
        os_name = str(cell.get("os"))
        variants = cell.get("variants")
        if os_name not in CANONICAL or not isinstance(variants, list):
            raise ValueError(f"unsupported producer cell: {cell}")
        if set(map(str, variants)) != CANONICAL[os_name]:
            raise ValueError(f"{os_name} variants do not match the canonical set")
        for version in versions:
            for variant in variants:
                jobs.append(
                    {
                        "openusd": version,
                        "profile": "usd",
                        "variant": variant,
                        "os": os_name,
                        "arch": cell["arch"],
                        "runner": cell["runner"],
                        "adapter": cell["adapter"],
                        "examples_required": variant != "core",
                        "tag": f"{version}-{variant}-{os_name}-{cell['arch']}",
                        "sdk": cell.get("sdk"),
                        "deployment_target": cell.get("deployment_target"),
                    }
                )
    if len(jobs) != 18:
        raise ValueError(f"primary matrix must expand to 18 leaves, got {len(jobs)}")
    if len({job["tag"] for job in jobs}) != len(jobs):
        raise ValueError("canonical leaf tags are not unique")
    return jobs


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "matrix",
        nargs="?",
        type=Path,
        default=Path(__file__).with_name("openusd-runtime-matrix.json"),
    )
    parser.add_argument("--github", action="store_true", help="emit a GitHub Actions matrix")
    args = parser.parse_args()
    document = json.loads(args.matrix.read_text(encoding="utf-8"))
    jobs = expand(document)
    print(json.dumps({"include": jobs} if args.github else {"schema": 1, "jobs": jobs}, separators=(",", ":")))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
