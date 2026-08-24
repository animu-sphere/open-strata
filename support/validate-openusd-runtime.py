#!/usr/bin/env python3
"""Backend-aware validation for canonical OpenUSD runtime leaves."""

from __future__ import annotations

import argparse
import json
import re
import subprocess
import sys
from pathlib import Path


def require_file(root: Path, relative: str) -> Path:
    path = root / relative
    if not path.is_file():
        raise RuntimeError(f"required file is missing: {path}")
    return path


def require_glob(root: Path, patterns: tuple[str, ...]) -> list[Path]:
    matches = sorted({path for pattern in patterns for path in root.glob(pattern) if path.is_file()})
    if not matches:
        raise RuntimeError(f"no files matched: {', '.join(patterns)}")
    return matches


def version_from_header(header: Path) -> str:
    source = header.read_text(encoding="utf-8")

    def value(name: str) -> int:
        match = re.search(rf"^\s*#\s*define\s+{name}\s+(\d+)\s*$", source, re.MULTILINE)
        if not match:
            raise RuntimeError(f"{name} is missing from {header}")
        return int(match.group(1))

    return f"{value('PXR_MINOR_VERSION')}.{value('PXR_PATCH_VERSION'):02d}"


def plugin_info(root: Path, plugin: str) -> Path:
    return next(
        (
            path
            for path in (
                root / "lib" / "usd" / plugin / "resources" / "plugInfo.json",
                root / "plugin" / "usd" / plugin / "resources" / "plugInfo.json",
            )
            if path.is_file()
        ),
        None,
    ) or require_file(root, f"lib/usd/{plugin}/resources/plugInfo.json")


def validate_relocation(root: Path, libraries: list[Path], platform: str) -> None:
    if platform != "macos":
        return
    for library in libraries:
        result = subprocess.run(
            ["otool", "-L", str(library)], capture_output=True, text=True, check=False
        )
        if result.returncode != 0:
            raise RuntimeError(f"otool failed for {library}: {result.stderr.strip()}")
        forbidden = [line.strip() for line in result.stdout.splitlines()[1:] if "/build/" in line or "/work/" in line]
        if forbidden:
            raise RuntimeError(f"non-relocatable dylib dependency in {library}: {forbidden[0]}")


def validate(root: Path, version: str, variant: str, platform: str, arch: str) -> dict[str, object]:
    root = root.resolve()
    actual = version_from_header(require_file(root, "include/pxr/pxr.h"))
    if actual != version:
        raise RuntimeError(f"OpenUSD version mismatch: expected {version}, found {actual}")

    imaging = variant != "core"
    features: dict[str, object] = {"examples_required": imaging}
    libraries: list[Path] = []
    if imaging:
        examples = require_glob(root, ("share/usd/examples/**/*",))
        if version == "26.08":
            require_glob(root, ("share/exec/examples/**/*",))
        features["examples"] = len(examples)

    if variant == "vulkan":
        require_file(root, "include/pxr/imaging/hgiVulkan/hgi.h")
        info = plugin_info(root, "hgiVulkan")
        patterns = {
            "windows": ("lib/*hgiVulkan*.dll", "bin/*hgiVulkan*.dll"),
            "linux": ("lib/lib*hgiVulkan*.so*",),
        }[platform]
        libraries = require_glob(root, patterns)
        features["vulkan"] = {"plugin": str(info.relative_to(root)), "files": len(libraries)}
    elif variant == "metal":
        require_file(root, "include/pxr/imaging/hgiMetal/hgi.h")
        info = plugin_info(root, "hgiMetal")
        libraries = require_glob(root, ("lib/lib*hgiMetal*.dylib", "lib/*hgiMetal*.dylib"))
        features["metal"] = {"plugin": str(info.relative_to(root)), "files": len(libraries)}
    elif variant == "gl":
        info = plugin_info(root, "hgiGL")
        features["opengl"] = {"plugin": str(info.relative_to(root))}
    else:
        forbidden = [root / "include/pxr/imaging", root / "lib/usd/hd"]
        if any(path.exists() for path in forbidden):
            raise RuntimeError("core runtime unexpectedly contains the imaging surface")

    validate_relocation(root, libraries, platform)
    return {
        "status": "passed",
        "runtime_root": str(root),
        "openusd": actual,
        "profile": "usd",
        "variant": variant,
        "platform": platform,
        "arch": arch,
        "features": features,
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("runtime_root", type=Path)
    parser.add_argument("--version", required=True, choices=("26.05", "26.08"))
    parser.add_argument("--variant", required=True, choices=("core", "gl", "vulkan", "metal"))
    parser.add_argument("--platform", required=True, choices=("windows", "linux", "macos"))
    parser.add_argument("--arch", required=True, choices=("x86_64", "arm64"))
    args = parser.parse_args()
    try:
        result = validate(args.runtime_root, args.version, args.variant, args.platform, args.arch)
    except RuntimeError as error:
        print(f"validation failed: {error}", file=sys.stderr)
        return 1
    print(json.dumps(result, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
