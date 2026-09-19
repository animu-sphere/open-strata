# Current

The next milestone. Shipped detail is in
[releases/](../releases/) and the [delivery history](../reports/delivery-history.md).

## v0.23.0 - release CI, host adapters and matrix

**Status:** active. **Depends on:** the
[v0.22.10 runtime-correctness foundation](../releases/v0.22.10.md).

Expand release automation and bind the same canonical, digest-pinned runtime
identity to DCC hosts without creating parallel dependency stores or hiding
host-owned APIs.

### Workstreams

- **Release-lane completeness:** reconcile generated and externally managed
  lanes against the same pins and support declaration.
- **Attributable evidence:** report workspace test results per discovered member
  and warn when a testable member contributes no tests.
- **Host add-ons:** the first `usdview-plugin` / `host-addon` vertical slice is
  implemented during v0.23.0 development; extend the same component contract
  to later DCC bindings without inventing host-specific stores.
- **Host matrix:** generate and exercise headless adapter cells with pinned
  host, OS, OpenUSD, Python, runtime/plugin digest, validation-tier and execution
  evidence across supported platforms.

The exact report sources and deduplicated acceptance are recorded in the
[downstream report intake](downstream-report-intake.md). Broader future work
remains ordered in the [backlog](backlog.md).

### Acceptance

- Generated and hand-authored release lanes are checked against the same OST
  version, runtime identity and support declaration.
- Graph, build, test, pyramid and package claims are attributable to the member
  that supplied the evidence.
- A host add-on resolves to the same canonical runtime identity and records
  explained capability SKIPs where the host is unavailable.

### Exit criteria

The v0.23.0 items in the [runtime-composition follow-through](runtime-composition.md)
and [downstream report intake](downstream-report-intake.md) are complete. Sessions,
Kubernetes execution, broad AI/GPU profiles and renderer-template expansion stay
outside this milestone.
