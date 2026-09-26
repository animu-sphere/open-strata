# Current

The v0.23.11 delivery is recorded in the
[release record](../releases/v0.23.11.md). This page tracks the remaining
acceptance and the next feature work.

## Post-release v0.23.11 evidence

- Re-run hydra-toon's combined VRM/renderer Formation against v0.23.11 and
  the published lookdev runtime. Expand renderer acceptance beyond local
  Windows evidence; record macOS usdview/Metal evidence before adding a
  canonical macOS lookdev leaf.

- Recheck `usd-mmd-plugins` report 01 against the published target-local bundle
  and tool stages: source tests, packages, and a second target must consume
  their own outputs without source-tree generated files.
- Adopt `usd-imaging` for `vrmImaging` and repeat native registry/schema closure
  validation with published v0.23.10 under the locked `usd` imaging runtime,
  including Linux/macOS. Report 50 confirms native registration on Windows;
  remove the old explicit `hydra-preview` requirement and update the downstream
  baseline session when adopting. VRM report 48 closes
  the Windows report 47 packaging regression; repeat those package cells on
  Linux/macOS. Track its separate producer RUNPATH cleanup request in the intake.

- Confirm the published release assets, checksums and attestations, then run an
  installed CLI smoke test. Fix release defects in v0.23.x.
- Complete the `usd-vrm-plugins` migration through the external artifact edge,
  including the three identities consumed only by tools, and run an independent
  installed consumer. Four packages and 75 tests already passed in the root
  build under v0.23.2; the tool-only edges need a v0.23.3 downstream rerun.
- Run the packaged VRM consumer and missing-file negative case across target
  operating systems, including a different bundle build order.
- Run a clean-host configure, link and test against the existing pinned
  OpenUSD runtime with v0.23.1's materialized-prefix repair. Independently
  republish and test an artifact whose CMake exports contain no producer-local
  Python paths.
- Exercise empty and populated optional OpenUSD selectors on hosted macOS,
  including cache verification and remote pulls.
- Exercise the standalone VRM bundle and library build sequence against the
  published v0.23.4 executable with a deliberately stale runtime cache.

## Next feature work

- Complete Maya/Houdini host add-on packaging, headless smoke suites,
  capability SKIPs and the host/OS/OpenUSD/Python matrix with pinned runtime and
  component identities.
- Continue installed-package dependency correctness and component-level
  architecture evidence in the [package-contract plan](component-package-contracts.md).
- Complete the `usd-vrm-plugins` migration to the published `execMotion` bundle
  and `motion_convert` tool, then verify the installed consumer and fixture flow.

The [runtime-composition follow-through](runtime-composition.md) and
[downstream report intake](downstream-report-intake.md) retain the detailed
acceptance and report sources. Broader work remains in the
[backlog](backlog.md).
