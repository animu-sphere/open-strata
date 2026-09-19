# Current

The next milestone. Shipped detail is in
[releases/](../releases/) and the [delivery history](../reports/delivery-history.md).

## v0.23.0 - release CI, component closure and host adapters

**Status:** active. **Depends on:** the
[v0.22.10 runtime-correctness foundation](../releases/v0.22.10.md).

Expand release automation and bind the same canonical, digest-pinned runtime
identity to DCC hosts without creating parallel dependency stores or hiding
host-owned APIs. The v0.22.10 downstream pass also makes package closure and
cross-repository library identity release gates for this milestone.

### Workstreams

- **Release-lane completeness:** reconcile generated and externally managed
  lanes against the same pins and support declaration.
- **Attributable evidence:** report workspace test results per discovered member
  and warn when a testable member contributes no tests.
- **Component closure:** package and test each bundle against its own declared
  installed library files, independent of the order other bundles were built.
  Admit versioned, digest-pinned library artifacts from other repositories into
  the same graph, CI and product provenance.
- **Consumer portability:** exercise the actual pinned OpenUSD runtime artifact
  through a clean-host CMake configure, link and test; distinguish this from
  configure-only validation. Keep generated optional OpenUSD arguments valid on
  hosted macOS and allow an empty scaffold to render honest CI.
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
- Package records name files present in that bundle's own closure; rebuilding
  bundles in another order does not change the product's loadability or
  provenance. External library dependencies have checked version, digest and
  runtime identity across graph, build, package, consumer and CI.
- A clean-host consumer links against the exact runtime artifact selected by
  the lock; generated jobs with absent optional selectors run on macOS.
- A host add-on resolves to the same canonical runtime identity and records
  explained capability SKIPs where the host is unavailable.

### Exit criteria

The v0.23.0 items in the [runtime-composition follow-through](runtime-composition.md)
and [downstream report intake](downstream-report-intake.md) are complete, including
the v0.22.10 dogfooding acceptance. Sessions,
Kubernetes execution, broad AI/GPU profiles and renderer-template expansion stay
outside this milestone.
