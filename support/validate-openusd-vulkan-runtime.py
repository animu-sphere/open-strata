#!/usr/bin/env python3
"""Validate the OpenUSD features required by the published cy2026 runtimes."""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path


def _require_file(root: Path, relative: str) -> Path:
    path = root / relative
    if not path.is_file():
        raise RuntimeError(f"required file is missing: {path}")
    return path


def _require_glob(root: Path, pattern: str) -> list[Path]:
    matches = sorted(path for path in root.glob(pattern) if path.is_file())
    if not matches:
        raise RuntimeError(f"no files matched required pattern: {root / pattern}")
    return matches


def _require_one_file(root: Path, relatives: tuple[str, ...]) -> Path:
    for relative in relatives:
        path = root / relative
        if path.is_file():
            return path
    choices = ", ".join(str(root / relative) for relative in relatives)
    raise RuntimeError(f"none of the required files exist: {choices}")


def _pxr_version(pxr_header: Path) -> str:
    contents = pxr_header.read_text(encoding="utf-8")

    def value(name: str) -> int:
        match = re.search(rf"^\s*#\s*define\s+{name}\s+(\d+)\s*$", contents, re.MULTILINE)
        if not match:
            raise RuntimeError(f"{name} is missing from {pxr_header}")
        return int(match.group(1))

    return f"{value('PXR_MINOR_VERSION')}.{value('PXR_PATCH_VERSION'):02d}"


def validate(root: Path, expected_version: str, platform: str) -> dict[str, object]:
    root = root.resolve()
    if not root.is_dir():
        raise RuntimeError(f"runtime root does not exist: {root}")

    pxr_header = _require_file(root, "include/pxr/pxr.h")
    actual_version = _pxr_version(pxr_header)
    if actual_version != expected_version:
        raise RuntimeError(
            f"OpenUSD version mismatch: expected {expected_version}, found {actual_version}"
        )

    hgi_header = _require_file(root, "include/pxr/imaging/hgiVulkan/hgi.h")
    plugin_info = _require_one_file(
        root,
        (
            "lib/usd/hgiVulkan/resources/plugInfo.json",
            "plugin/usd/hgiVulkan/resources/plugInfo.json",
        ),
    )
    if "hgiVulkan" not in plugin_info.read_text(encoding="utf-8"):
        raise RuntimeError(f"hgiVulkan registration is missing from {plugin_info}")

    if platform == "windows":
        vulkan_libraries = _require_glob(root, "lib/*hgiVulkan*.lib")
        vulkan_runtime = _require_glob(root, "lib/*hgiVulkan*.dll")
    else:
        vulkan_libraries = _require_glob(root, "lib/lib*hgiVulkan*.so*")
        vulkan_runtime = vulkan_libraries

    usd_examples = _require_glob(root, "share/usd/examples/**/*")
    exec_examples: list[Path] = []
    if expected_version == "26.08":
        exec_examples = _require_glob(root, "share/exec/examples/**/*")

    result = {
        "status": "passed",
        "runtime_root": str(root),
        "openusd": actual_version,
        "platform": platform,
        "features": {
            "vulkan": {
                "header": str(hgi_header.relative_to(root)),
                "plugin": str(plugin_info.relative_to(root)),
                "libraries": [str(path.relative_to(root)) for path in vulkan_libraries],
                "runtime_files": [str(path.relative_to(root)) for path in vulkan_runtime],
            },
            "examples": {
                "usd_file_count": len(usd_examples),
                "exec_file_count": len(exec_examples),
            },
        },
    }
    return result


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("runtime_root", type=Path)
    parser.add_argument("--version", required=True, choices=("26.05", "26.08"))
    parser.add_argument("--platform", required=True, choices=("windows", "linux"))
    args = parser.parse_args()

    try:
        result = validate(args.runtime_root, args.version, args.platform)
    except RuntimeError as error:
        print(f"validation failed: {error}", file=sys.stderr)
        return 1

    print(json.dumps(result, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
