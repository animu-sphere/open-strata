# Adopting a renderer project

This guide is for a repository that builds a **renderer** — a host-neutral
rendering core, an optional Hydra delegate, native viewport or headless tools —
and wants OpenStrata to drive its build, adopt a runtime, run it, and record
evidence **without** reshaping it into a plugin workspace. It is transferable to
any renderer; the [hdMerlin](../projects/hydra-merlin.md) reference project is
used as a worked example, not a required layout.

A renderer is not a bundle graph. OpenStrata drives your project's own CMake
targets and discovers the built renderer in the install tree; it does not split
your internal targets into artificial bundles.

## 1. Keep your CMake targets; let `ost` drive them

Point OpenStrata at your existing project. `ost build` runs your CMake
configure/build under a recorded runtime, generator intent, and configuration,
and records a build fingerprint:

```sh
ost build --check          # preflight without building
ost build --jobs auto      # configure + build (Ninja)
```

Your target boundaries are unchanged. OpenStrata never claims it produced a build
tree it did not configure — an externally built tree is validated with
`ost validate --build-dir <dir>` and only upgrades runtime compatibility on a full
identity match.

## 2. Adopt a digest-pinned runtime

Exercise the renderer against a known OpenUSD build rather than a local install.
Pull the runtime profile the renderer needs (its host-neutral core may need only
`core`; Hydra/usdview paths need `usd`):

```sh
ost runtime pull cy2026 --profile core     # host-neutral core build
ost runtime pull cy2026 --profile usd      # Hydra / usdview paths
```

In CI, adopt the runtime by digest so every cell is reproducible.

## 3. Validate and record evidence bound to a producer

Validate the built target and emit machine-readable evidence:

```sh
ost validate --json
```

A renderer PASS is bound to a **completed producer session** — a result is only
mergeable after its command or declared check completes (v0.18.0). When several
producers contribute reports, `ost renderer merge` preserves provenance and
refuses assertions from failed, incomplete, or superseded sessions.

Managed `ost build`, `ost test`, and `ost renderer viewport` own this step: they
snapshot renderer evidence before writing the target and stamp only reports
created or rewritten by their operation after it concludes. An untouched report
keeps its earlier owner instead of being laundered through a no-op incremental
build. For a genuinely external producer, attach its timing and outcome
explicitly; the origin is fixed to `external-unverified`:

```sh
ost renderer attach-session build/external/renderer-report.json \
  --target external-release --started-unix 1750000000 \
  --completed-unix 1750000030 --outcome success
```

## 4. Open the renderer

Open a scene in the matching `usdview` session with your Hydra renderer selected,
or launch the standalone native viewport:

```sh
ost renderer view scene.usda --profile usd     # managed usdview session
ost renderer viewport -- --frames 8 --hidden   # standalone native viewport
```

Before paying the configure/build cost, resolve the named intent and
scene/runtime capabilities:

```sh
ost renderer viewport --preflight --intent viewport-usd --profile usd -- \
  --usd path/to/scene.usd --frames 1 --hidden
```

The JSON form publishes normalized `requested`, `applied`, `skipped`, and
`unrequested` capability evidence. A USD scene workflow fails here—before the
build tree is touched—unless the selected profile provides `usd-stage-read`.
The same preflight object is retained in the durable viewport launch record.

`renderer view` defaults to automatic camera selection and classifies optional
host warnings separately from real plugin-discovery, renderer-selection, or
first-frame failures.

## 5. Gate GPU work as capabilities

Keep GPU/Vulkan checks as separate capability jobs and let validation explain a
SKIP (no display, no GPU) rather than fail blindly. `ost doctor` next actions
depend on the selected profile and the capability exercised.

## Where to go next

- Command details: [reference/cli.md](../reference/cli.md); JSON contract:
  [reference/json-output.md](../reference/json-output.md).
- Renderer direction:
  [design/proposed/renderer-templates.md](../design/proposed/renderer-templates.md).
- Composing this renderer with plugins from another repository (VRM rendered by
  hdMerlin) is the planned [Formation](../design/proposed/formations.md) model —
  see [compose a formation](compose-a-formation.md) and
  [combined-formations.md](../projects/combined-formations.md).
