# USD VRM PR #140: render-probe host prerequisites

## Observed failure

[Source CI run 32920972689](https://github.com/animu-sphere/usd-vrm-plugins/actions/runs/32920972689)
tested downstream commit `78af124f933f97d14b2ad704a0e91171f24ea91e`
with OST 0.22.5. All five Windows and all five macOS cells failed before
plugin/workspace compilation, at `Validate the materialized runtime (runnable
tools)`. The five Linux cells and the workspace graph check passed.

The [Windows workspace job](https://github.com/animu-sphere/usd-vrm-plugins/actions/runs/32920972689/job/98034337026)
selected CPython 3.13.15. Its loader and consumer-configure checks passed;
the WGL probe reported `GDI Generic` as a passing device observation.
`usdrecord` then failed importing Qt, ending with
`ModuleNotFoundError: No module named 'PySide2'`.

The [macOS workspace job](https://github.com/animu-sphere/usd-vrm-plugins/actions/runs/32920972689/job/98034336959)
passed consumer configure, Metal loader, and Metal device enumeration. The
render probe selected `/Library/Frameworks/Python.framework/Versions/3.13/bin/python3.13`
and failed at the same Qt import.

This is a new prerequisite failure after the interpreter-selection fix in
0.22.5. The installed OpenUSD 26.08 `usdrecord` tries PySide6's Qt OpenGL
modules, falls back to PySide2 on `ImportError`, and calls
`_SetupOpenGLContext` whenever GPU rendering is enabled, including Metal.
The final PySide2 exception alone does not mean PySide2 is the preferred
dependency or that Python selection is still broken.

During this investigation, downstream commit `04ea86a` added
[report 36 section 11](https://github.com/animu-sphere/usd-vrm-plugins/blob/04ea86a2b51c0db5b055f7430d7e282f4e989c7f/docs/reports/ost/36-2026-08-25-v0.22.3-canonical-runtimes-and-release-membership.md#11-added-2026-08-26-the-render-probe-needs-qt-and-this-workstation-had-it)
with the same failure and a request to distinguish software renderers from
physical-device evidence. Its OST pin remains 0.22.5.

## Change

- Python `usdrecord` probes check the same Qt imports under the resolved
  interpreter and runtime environment before invoking the actual script.
  The preflight and render share one child process and the existing timeout.
- If neither binding can import, a dedicated exit code **and** diagnostic
  marker produce `openusd-render: skip`. The detail names the interpreter,
  missing prerequisite, and that no render was attempted. It is not render
  success. Real tool, pxr, and Hydra failures remain failures.
- `GDI Generic`, `llvmpipe`, and `softpipe` contexts are reported as software
  renderers, not physical devices. They cannot enable the render probe.
  Modern software OpenGL may render, but does not establish the physical-device
  evidence this check measures. Real NVIDIA/AMD observations remain eligible.
- Producer evidence is not synthesized from a skip; artifact-consumer
  observations still do not rewrite immutable producer identity. The export
  gates requiring positive device/render evidence remain unchanged.

No runtime artifacts, downstream digests, generated workflows, release pins,
or validation exit policies have been changed by this patch.

## Local verification

Windows, the exact pinned `sha256:ebb0c7da509ee14ada19ee5b461de6996aad0024b5c9640f12dde76912e849b5`
runtime, CPython 3.13, NVIDIA RTX A5000:

| Probe environment | Released 0.22.5 | Patched build |
| --- | --- | --- |
| Qt imports deliberately made unavailable using temporary Python modules | exit 5, render `fail` | exit 0, render `skip`, no frame claimed |
| Normal installed Qt and GPU | not repeated in this comparison | exit 0, render `pass`, 64px frame, 1716 bytes |

The missing-Qt reproduction did not uninstall packages or alter SDK files.
Temporary modules were supplied only through the child command's environment.

- `cargo test -p ost-runtime --locked`: 36 passed, including the deterministic
  software-versus-physical-device regression.
- `cargo test -p ost-cli --bin ost --locked`: 187 passed, including isolated
  subprocess tests for missing Qt, PySide6, PySide2 fallback, and a tool failure.
- `cargo clippy --workspace --all-targets --locked -- -D warnings`:
  passed.
- `cargo test --workspace --locked`: 823 passed across 34 test/doc-test
  executables, zero failures.

## Still to verify

This patch has not run on a hosted macOS or Windows runner. Local GPU success
does not establish hosted-runner render support. The available WSL Ubuntu has
no Cargo installation, so Linux compilation was not repeated locally.
Downstream PR #140 still pins
released OST 0.22.5, so its existing red checks are not fixed merely by editing
this repository. After a release containing the fix is available, update its
`bootstrap.ost.version`, regenerate the OST workflows, update the hand-authored
`release.yml` pin, and run all source cells again. Later build or test failures
may be hidden behind the current validation failure.

The downstream scheduled lane's old 26.05-built plugin artifact against a
26.08 runtime is a separate, pre-existing evidence issue.
