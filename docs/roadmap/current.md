# Current

The v0.23.1 repair is recorded in the
[release record](../releases/v0.23.1.md). This page tracks the remaining
acceptance and the next feature work.

## Post-release v0.23.1 evidence

- Confirm the published release assets, checksums and attestations, then run an
  installed CLI smoke test. Fix release defects in v0.23.x.
- Migrate `usd-motion-plugins` `motionCore` into `usd-vrm-plugins` through the
  new external artifact edge, and run an independent installed consumer.
- Run the packaged VRM consumer and missing-file negative case across target
  operating systems, including a different bundle build order.
- Run a clean-host configure, link and test against the existing pinned
  OpenUSD runtime with v0.23.1's materialized-prefix repair. Independently
  republish and test an artifact whose CMake exports contain no producer-local
  Python paths.
- Exercise empty and populated optional OpenUSD selectors on hosted macOS,
  including cache verification and remote pulls.

## Next feature work

- Complete Maya/Houdini host add-on packaging, headless smoke suites,
  capability SKIPs and the host/OS/OpenUSD/Python matrix with pinned runtime and
  component identities.
- Continue installed-package dependency correctness and component-level
  architecture evidence in the [package-contract plan](component-package-contracts.md).

The [runtime-composition follow-through](runtime-composition.md) and
[downstream report intake](downstream-report-intake.md) retain the detailed
acceptance and report sources. Broader work remains in the
[backlog](backlog.md).
