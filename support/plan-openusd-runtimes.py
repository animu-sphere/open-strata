#!/usr/bin/env python3
"""Expand the canonical OpenUSD support declaration into producer jobs."""

from __future__ import annotations

import argparse
import json
from pathlib import Path
from typing import Iterable

VERSIONS = ["26.05", "26.08"]
GATES = ["build", "runtime", "graphics", "artifact", "distribution"]
REPOSITORY = "oci://ghcr.io/animu-sphere/openstrata-runtime-cy2026-usd"
PUBLICATION = {
    "leaf_tags": "<openusd-version>-<variant>-<os>-<arch>",
    "digest_pull_required": True,
    "multi_platform_aliases": False,
}
CANONICAL = {
    "linux": {
        "arch": "x86_64",
        "runner": "ubuntu-24.04",
        "adapter": "build/openusd-linux.sh",
        "variants": ["core", "gl", "vulkan"],
    },
    "windows": {
        "arch": "x86_64",
        "runner": "windows-2025",
        "adapter": "build/openusd-windows.ps1",
        "variants": ["core", "gl", "vulkan"],
    },
    "macos": {
        "arch": "arm64",
        "runner": "macos-15",
        "adapter": "build/openusd-macos.sh",
        "variants": ["core", "metal"],
        "sdk": "15.5",
        "deployment_target": "13.0",
    },
}


# Every declared variant of every canonical version, across the primary cells:
# Linux and Windows publish core/gl/vulkan, macOS publishes core/metal.
CANONICAL_LEAF_COUNT = len(VERSIONS) * sum(len(cell["variants"]) for cell in CANONICAL.values())


def expand(document: dict[str, object]) -> list[dict[str, object]]:
    if document.get("schema") != 1 or document.get("profile") != "usd":
        raise ValueError("matrix must use schema 1 and profile 'usd'")
    versions = document.get("versions")
    cells = document.get("cells")
    if versions != VERSIONS or not isinstance(cells, list):
        raise ValueError("matrix must declare OpenUSD 26.05 and 26.08 in order")
    if document.get("repository") != REPOSITORY:
        raise ValueError(f"matrix repository must be {REPOSITORY!r}")
    if document.get("gates") != GATES:
        raise ValueError(f"matrix gates must be {GATES!r} in order")
    if document.get("publication") != PUBLICATION:
        raise ValueError("matrix publication policy is not canonical")
    if len(cells) != len(CANONICAL):
        raise ValueError("matrix must declare exactly the three primary producer cells")

    jobs: list[dict[str, object]] = []
    seen_operating_systems: set[str] = set()
    for cell in cells:
        if not isinstance(cell, dict):
            raise ValueError("each matrix cell must be an object")
        os_name = str(cell.get("os"))
        expected = CANONICAL.get(os_name)
        if expected is None:
            raise ValueError(f"unsupported producer cell: {cell}")
        if os_name in seen_operating_systems:
            raise ValueError(f"matrix declares more than one {os_name} producer cell")
        seen_operating_systems.add(os_name)
        actual = {key: value for key, value in cell.items() if key != "os"}
        if actual != expected:
            raise ValueError(f"{os_name} producer cell is not canonical: expected {expected!r}")
        variants = expected["variants"]
        for version in versions:
            for variant in variants:
                jobs.append(
                    {
                        "openusd": version,
                        "profile": "usd",
                        "variant": variant,
                        "os": os_name,
                        "arch": expected["arch"],
                        "runner": expected["runner"],
                        "adapter": expected["adapter"],
                        "examples_required": variant != "core",
                        "tag": f"{version}-{variant}-{os_name}-{expected['arch']}",
                        "sdk": expected.get("sdk"),
                        "deployment_target": expected.get("deployment_target"),
                    }
                )
    if len(jobs) != CANONICAL_LEAF_COUNT:
        raise ValueError(
            f"primary matrix must expand to {CANONICAL_LEAF_COUNT} leaves, got {len(jobs)}"
        )
    if len({job["tag"] for job in jobs}) != len(jobs):
        raise ValueError("canonical leaf tags are not unique")
    return jobs


def select_jobs(
    jobs: Iterable[dict[str, object]],
    *,
    host: str | None = None,
    arch: str | None = None,
    versions: set[str] | None = None,
    variants: set[str] | None = None,
) -> list[dict[str, object]]:
    if (host is None) != (arch is None):
        raise ValueError("--host and --arch must be provided together")
    selected = [
        job
        for job in jobs
        if (host is None or (job["os"] == host and job["arch"] == arch))
        and (versions is None or job["openusd"] in versions)
        and (variants is None or job["variant"] in variants)
    ]
    if not selected:
        # Name what the host actually declares. Asking for a variant a platform
        # does not publish -- `gl` on macOS, `metal` on Linux -- is the common
        # way to land here, and "no leaves" alone does not say which it was.
        on_host = [
            job
            for job in jobs
            if host is None or (job["os"] == host and job["arch"] == arch)
        ]
        where = f"{host}-{arch}" if host else "the canonical matrix"
        if not on_host:
            raise ValueError(f"{where} declares no canonical runtime leaves")
        raise ValueError(
            f"the requested filters select no canonical runtime leaves; {where} "
            f"declares versions {sorted({job['openusd'] for job in on_host})} "
            f"and variants {sorted({job['variant'] for job in on_host})}"
        )
    return selected


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "matrix",
        nargs="?",
        type=Path,
        default=Path(__file__).with_name("openusd-runtime-matrix.json"),
    )
    parser.add_argument("--github", action="store_true", help="emit a GitHub Actions matrix")
    parser.add_argument("--host", choices=tuple(CANONICAL), help="select one producer OS")
    parser.add_argument("--arch", choices=("x86_64", "arm64"), help="select one producer architecture")
    parser.add_argument("--version", action="append", choices=VERSIONS, dest="versions")
    parser.add_argument(
        "--variant",
        action="append",
        choices=("core", "gl", "vulkan", "metal"),
        dest="variants",
    )
    args = parser.parse_args()
    document = json.loads(args.matrix.read_text(encoding="utf-8"))
    jobs = select_jobs(
        expand(document),
        host=args.host,
        arch=args.arch,
        versions=set(args.versions) if args.versions else None,
        variants=set(args.variants) if args.variants else None,
    )
    print(json.dumps({"include": jobs} if args.github else {"schema": 1, "jobs": jobs}, separators=(",", ":")))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
