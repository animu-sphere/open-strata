// SPDX-License-Identifier: Apache-2.0
//! GitHub Actions workflow generation from a support matrix (Phase 5).
//!
//! Cells render by **lane** into up to three workflows:
//!
//! - **source CI** (`pull_request` / `main` lanes): checkout → materialize a
//!   digest-pinned runtime SDK artifact → build/test/package the bundle from
//!   source. Never publishes, never uses secrets (fork-PR safety).
//! - **support matrix** (`scheduled` / `workflow_dispatch` lanes): re-verify a
//!   pinned runtime×plugin artifact pair from the runner's local registry.
//! - **trusted release** (`publish: candidate` on `main` cells): exact tag
//!   validation → reproducible package/from-package gates → immutable candidate
//!   handoff → a separate OIDC-authorized publisher job.
//!
//! Cells reference named runner profiles; the renderer maps a profile to
//! `runs-on` (`github-hosted.image` → the image, `self-hosted.labels` → the
//! label list) and emits a billing notice on GitHub-hosted jobs. Generation is
//! deterministic (same matrix in, same YAML out) and every third-party action
//! is pinned to a full commit SHA (SEC-004), reusing the SHAs this repository
//! itself pins.

use crate::matrix::{Bootstrap, Lane, ReleaseMode, SourceCheck, SupportCell, SupportMatrix};

/// Default path of the generated support-matrix workflow.
pub const WORKFLOW_PATH: &str = ".github/workflows/ost-support-matrix.yml";

/// Default path of the generated source-CI workflow.
pub const SOURCE_WORKFLOW_PATH: &str = ".github/workflows/ost-source-ci.yml";

/// Default path of the generated trusted-release workflow.
pub const RELEASE_WORKFLOW_PATH: &str = ".github/workflows/ost-release.yml";

/// `actions/checkout`, pinned (SEC-004). Matches ci.yml.
const CHECKOUT: &str = "actions/checkout@9c091bb21b7c1c1d1991bb908d89e4e9dddfe3e0 # v7.0.0";

/// `actions/upload-artifact`, pinned (SEC-004). Matches release.yml.
const UPLOAD_ARTIFACT: &str =
    "actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a # v7";

/// `actions/download-artifact`, pinned (SEC-004). Matches release.yml.
const DOWNLOAD_ARTIFACT: &str =
    "actions/download-artifact@3e5f45b2cfb9172054b4087a40e8e0b5a5461e7c # v8";

/// `actions/cache`, pinned (SEC-004). Matches ci.yml.
const CACHE: &str = "actions/cache@55cc8345863c7cc4c66a329aec7e433d2d1c52a9 # v6.1.0";

/// `actions/setup-python`, pinned (SEC-004). Installs the runtime's declared
/// schema-tooling Python ABI on a hosted source cell that has no bundled
/// interpreter (v0.12.0 macOS dogfood).
const SETUP_PYTHON: &str = "actions/setup-python@ece7cb06caefa5fff74198d8649806c4678c61a1 # v6.3.0";

/// The hosted-runner billing notice step, gated per include entry so
/// self-hosted cells in the same job stay quiet.
const BILLING_NOTICE: &str = "\
\x20     - name: Hosted runner billing notice
        if: ${{ matrix.hosted }}
        shell: bash
        run: echo \"::notice title=OpenStrata hosted-runner usage::This job uses GitHub-hosted infrastructure. Private repositories may incur GitHub Actions usage charges. Review repository billing and Actions usage settings.\"
";

/// One rendered workflow: where it belongs and its YAML.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedWorkflow {
    pub path: &'static str,
    pub yaml: String,
}

/// Render the matrix into its workflows (support, source CI, then release).
/// Lanes with no cells render nothing; a non-empty matrix always yields at
/// least one workflow.
pub fn generate_github(matrix: &SupportMatrix) -> Vec<GeneratedWorkflow> {
    let mut out = Vec::new();
    if let Some(yaml) = generate_support(matrix) {
        out.push(GeneratedWorkflow {
            path: WORKFLOW_PATH,
            yaml,
        });
    }
    if let Some(yaml) = generate_source(matrix) {
        out.push(GeneratedWorkflow {
            path: SOURCE_WORKFLOW_PATH,
            yaml,
        });
    }
    if let Some(yaml) = generate_release(matrix) {
        out.push(GeneratedWorkflow {
            path: RELEASE_WORKFLOW_PATH,
            yaml,
        });
    }
    out
}

/// One `matrix.include` entry. `extra` appends lane-specific fields.
fn include_entry(matrix: &SupportMatrix, cell: &SupportCell, extra: &str) -> String {
    let runs_on = matrix
        .runs_on(cell)
        .into_iter()
        .map(|l| format!("\"{l}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let runner_profile = cell
        .runner
        .as_deref()
        .map(|name| format!("            runner_profile: {name}\n"))
        .unwrap_or_default();
    format!(
        "          - name: {name}\n\
         \x20           lane: {lane}\n\
         \x20           runtime_artifact: {runtime}\n\
         \x20           target_trust: {target_trust}\n\
         \x20           minimum_trust: {minimum_trust}\n\
         \x20           require_evidence: {require_evidence}\n\
         \x20           evidence_flags: \"{evidence_flags}\"\n\
         \x20           platform: {platform}\n\
         \x20           profile: {profile}\n\
         \x20           up_to: {up_to}\n\
         \x20           runs_on: [{runs_on}]\n\
         \x20           hosted: {hosted}\n\
         {runner_profile}\
         {extra}",
        name = cell.name,
        lane = cell.lane.as_str(),
        runtime = cell.runtime_artifact,
        target_trust = cell.trust,
        minimum_trust = matrix.minimum_trust(cell),
        require_evidence = matrix.require_evidence(cell).as_str(),
        evidence_flags = matrix.require_evidence(cell).verify_flags(),
        platform = cell.platform,
        profile = cell.profile,
        up_to = cell.up_to,
        hosted = matrix.is_hosted(cell),
    )
}

/// The job-level `OST_CI_*` exports (CI evidence): every `ost` invocation in
/// the job — and so every report.json it writes — records which support
/// claim it proves (cell, lane, runner profile, resolved runs-on, pinned
/// digests). `matrix.runner_profile` resolves to an empty string for
/// label-based cells, which the report reader treats as absent.
fn ci_env(with_plugin_artifact: bool) -> String {
    let mut env = String::from(
        "    env:\n\
         \x20     OST_CI_CELL: ${{ matrix.name }}\n\
         \x20     OST_CI_LANE: ${{ matrix.lane }}\n\
         \x20     OST_CI_RUNNER_PROFILE: ${{ matrix.runner_profile }}\n\
         \x20     OST_CI_RUNS_ON: ${{ join(matrix.runs_on, ',') }}\n\
         \x20     OST_CI_RUNTIME_ARTIFACT: ${{ matrix.runtime_artifact }}\n\
         \x20     OST_CI_MINIMUM_TRUST: ${{ matrix.minimum_trust }}",
    );
    if with_plugin_artifact {
        env.push_str("\n      OST_CI_PLUGIN_ARTIFACT: ${{ matrix.plugin_artifact }}");
    }
    env
}

/// Render the support-matrix workflow (`scheduled`/`workflow_dispatch`
/// cells), or `None` when the matrix declares no such cells.
pub fn generate_support(matrix: &SupportMatrix) -> Option<String> {
    let cells = matrix.support_cells();
    if cells.is_empty() {
        return None;
    }

    let scheduled: Vec<&SupportCell> = cells
        .iter()
        .filter(|c| c.lane == Lane::Scheduled)
        .copied()
        .collect();
    let dispatch: Vec<&SupportCell> = cells
        .iter()
        .filter(|c| c.lane == Lane::WorkflowDispatch)
        .copied()
        .collect();

    let mut on = String::new();
    if !dispatch.is_empty() {
        on.push_str("  workflow_dispatch:\n");
    }
    if !scheduled.is_empty() {
        on.push_str(
            "  schedule:\n    # Weekly; real-runtime cells are too heavy for per-PR CI (which should\n    # keep its cheap mock/static checks).\n    - cron: \"0 3 * * 1\"\n",
        );
    }

    let mut jobs = String::new();
    if !scheduled.is_empty() {
        jobs.push_str(&support_job(matrix, "scheduled", "schedule", &scheduled));
    }
    if !dispatch.is_empty() {
        jobs.push_str(&support_job(
            matrix,
            "dispatch",
            "workflow_dispatch",
            &dispatch,
        ));
    }

    Some(format!(
        "\
# Generated by `ost ci generate github` from openstrata.ci.yaml.
# Regenerate after editing the matrix; do not edit the jobs by hand.
#
# Each job is one explicit support cell: a runtime artifact x a plugin
# artifact, both pinned by full registry digest. The runner's local registry
# (OST_HOME, default ~/.ost) must already hold those artifacts -- seed it with
# `ost artifact import` (e.g. from an `ost artifact export` handoff) -- and
# `ost` must be on PATH.
name: ost support matrix

on:
{on}
permissions:
  contents: read

jobs:
{jobs}"
    ))
}

/// One support-matrix job for a single support lane.
fn support_job(matrix: &SupportMatrix, id: &str, event: &str, cells: &[&SupportCell]) -> String {
    let mut include = String::new();
    for cell in cells {
        let plugin = cell
            .plugin_artifact
            .as_deref()
            .expect("validated: support cells carry a plugin artifact");
        include.push_str(&include_entry(
            matrix,
            cell,
            &format!("            plugin_artifact: {plugin}\n"),
        ));
    }

    let policy = matrix
        .trust
        .policy
        .as_deref()
        .map(|path| format!(" --policy {path}"))
        .unwrap_or_default();
    let policy_checkout = matrix
        .trust
        .policy
        .as_ref()
        .map(|_| {
            format!(
                "      - name: Check out the repository trust policy\n        uses: {CHECKOUT}\n"
            )
        })
        .unwrap_or_default();

    format!(
        "\
\x20 {id}:
    if: github.event_name == '{event}'
    name: ${{{{ matrix.name }}}}
    runs-on: ${{{{ matrix.runs_on }}}}
{env}
    strategy:
      # One broken support line must not hide the state of the others.
      fail-fast: false
      matrix:
        include:
{include}\
\x20   steps:
{BILLING_NOTICE}\
{policy_checkout}\
\x20     - name: Check the pinned artifacts are in the local registry
        run: |
          ost artifact show ${{{{ matrix.runtime_artifact }}}}
          ost artifact show ${{{{ matrix.plugin_artifact }}}}
      - name: Verify artifact integrity
        run: |
          ost artifact verify ${{{{ matrix.runtime_artifact }}}} --minimum-trust ${{{{ matrix.minimum_trust }}}} ${{{{ matrix.evidence_flags }}}}{policy}
          ost artifact verify ${{{{ matrix.plugin_artifact }}}} --minimum-trust ${{{{ matrix.minimum_trust }}}} ${{{{ matrix.evidence_flags }}}}{policy}
      - name: Materialize the runtime from the registry
        run: ost runtime pull ${{{{ matrix.platform }}}} --profile ${{{{ matrix.profile }}}} --from-artifact ${{{{ matrix.runtime_artifact }}}} --force
      - name: Extract the plugin bundle under test
        run: ost artifact extract ${{{{ matrix.plugin_artifact }}}} plugin-under-test
      - name: Run the verification pyramid
        run: ost plugin test plugin-under-test --target ${{{{ matrix.platform }}}} --profile ${{{{ matrix.profile }}}} --up-to ${{{{ matrix.up_to }}}} --json
      - name: Upload the verification report
        if: always()
        uses: {UPLOAD_ARTIFACT}
        with:
          name: report-${{{{ matrix.name }}}}
          path: plugin-under-test/.strata/reports/
",
        env = ci_env(true),
    )
}

/// The hosted `ost` bootstrap step (transport plan, "Bootstrap policy"):
/// download the version-pinned release asset for the runner's platform,
/// verify its checksum (against the release's published `.sha256`, plus the
/// matrix's exact-byte pin when one is declared), put the binary on PATH,
/// and save bootstrap evidence. A failure here is a *bootstrap* failure —
/// its own step — never conflated with an artifact/runtime failure.
fn bootstrap_step(bootstrap: &Bootstrap, require_exact_pin: bool) -> String {
    let ost = &bootstrap.ost;
    let mut pin_lines = String::new();
    for (triple, hex) in &ost.sha256 {
        pin_lines.push_str(&format!("            {triple}) pinned=\"{hex}\" ;;\n"));
    }
    let exact_pin_gate = if require_exact_pin {
        "          if [ -z \"$pinned\" ]; then\n            echo \"::error title=ost bootstrap::trusted release requires an exact checksum pin for $triple\" ; exit 1\n          fi\n"
    } else {
        ""
    };
    format!(
        "\
\x20     - name: Bootstrap ost {version} (pinned release asset, checksum-verified)
        if: ${{{{ matrix.hosted }}}}
        shell: bash
        run: |
          set -euo pipefail
          mkdir -p .ost-ci
          case \"${{RUNNER_OS}}-${{RUNNER_ARCH}}\" in
            Linux-X64)   triple=x86_64-unknown-linux-musl ; ext=tar.xz ;;
            macOS-ARM64) triple=aarch64-apple-darwin ; ext=tar.xz ;;
            macOS-X64)   triple=x86_64-apple-darwin ; ext=tar.xz ;;
            Windows-X64) triple=x86_64-pc-windows-msvc ; ext=zip ;;
            *) echo \"::error title=ost bootstrap::no ost release asset for ${{RUNNER_OS}}-${{RUNNER_ARCH}}\" ; exit 1 ;;
          esac
          pinned=\"\"
          case \"$triple\" in
{pin_lines}\
\x20           *) : ;;
          esac
{exact_pin_gate}\
\x20         asset=\"ost-cli-${{triple}}.${{ext}}\"
          base=\"https://github.com/{repository}/releases/download/v{version}\"
          curl -fsSLo \"$asset\" \"$base/$asset\"
          curl -fsSLo \"$asset.sha256\" \"$base/$asset.sha256\"
          actual=\"$( (command -v sha256sum > /dev/null && sha256sum \"$asset\" || shasum -a 256 \"$asset\") | cut -d' ' -f1 )\"
          published=\"$(cut -d' ' -f1 \"$asset.sha256\")\"
          if [ \"$actual\" != \"$published\" ]; then
            echo \"::error title=ost bootstrap::$asset hashes to $actual but the release publishes $published\" ; exit 1
          fi
          if [ -n \"$pinned\" ] && [ \"$actual\" != \"$pinned\" ]; then
            echo \"::error title=ost bootstrap::$asset hashes to $actual but the CI contract pins $pinned\" ; exit 1
          fi
          mkdir -p .ost-ci/bootstrap-bin
          if [ \"$ext\" = \"zip\" ]; then
            powershell -NoProfile -Command \"Expand-Archive -LiteralPath '$asset' -DestinationPath '.ost-ci/bootstrap-bin' -Force\"
          else
            tar -xf \"$asset\" -C .ost-ci/bootstrap-bin
          fi
          bin=\"$(find .ost-ci/bootstrap-bin -type f \\( -name ost -o -name ost.exe \\) | head -n 1)\"
          if [ -z \"$bin\" ]; then echo \"::error title=ost bootstrap::no ost binary inside $asset\" ; exit 1 ; fi
          chmod +x \"$bin\" 2> /dev/null || true
          bin_dir=\"$(cd \"$(dirname \"$bin\")\" && pwd)\"
          bin=\"$bin_dir/$(basename \"$bin\")\"
          executable=\"$bin\"
          exported_path=\"$bin_dir\"
          if [ \"$RUNNER_OS\" = \"Windows\" ]; then
            if ! command -v cygpath > /dev/null; then
              echo \"::error title=ost bootstrap::cygpath is required to export a native Windows PATH\" ; exit 1
            fi
            executable=\"$(cygpath -w \"$bin\")\"
            exported_path=\"$(cygpath -w \"$bin_dir\")\"
          fi
          echo \"$exported_path\" >> \"$GITHUB_PATH\"
          json_executable=\"$(printf '%s' \"$executable\" | sed 's/\\\\/\\\\\\\\/g; s/\"/\\\\\"/g')\"
          json_exported_path=\"$(printf '%s' \"$exported_path\" | sed 's/\\\\/\\\\\\\\/g; s/\"/\\\\\"/g')\"
          printf '{{\"schema\":1,\"pinned_version\":\"%s\",\"asset\":\"%s\",\"sha256\":\"%s\",\"executable\":\"%s\",\"exported_path\":\"%s\"}}\\n' \"{version}\" \"$asset\" \"$actual\" \"$json_executable\" \"$json_exported_path\" > .ost-ci/bootstrap.json
",
        version = ost.version,
        repository = ost.repository,
    )
}

/// The `ost` availability check. With a bootstrap pin, hosted cells must
/// report exactly the pinned version; the observed version always lands in
/// the CI evidence.
fn ost_version_step(bootstrap: Option<&Bootstrap>) -> String {
    let resolve_version = if bootstrap.is_some() {
        "\
\x20         if [ \"${{ matrix.hosted }}\" = \"true\" ] && [ \"$RUNNER_OS\" = \"Windows\" ]; then
            version=\"$(powershell -NoProfile -Command \"& ost.exe --version\" | tr -d '\\r')\"
          else
            version=\"$(ost --version)\"
          fi
"
    } else {
        "          version=\"$(ost --version)\"\n"
    };
    let assert = match bootstrap {
        Some(b) => format!(
            "\
\x20         if [ \"${{{{ matrix.hosted }}}}\" = \"true\" ] && [ \"$version\" != \"ost {v}\" ]; then
            echo \"::error title=ost bootstrap::expected 'ost {v}', got '$version'\" ; exit 1
          fi
",
            v = b.ost.version
        ),
        None => String::new(),
    };
    format!(
        "\
\x20     - name: Check ost is available and record its version
        shell: bash
        run: |
          set -euo pipefail
          mkdir -p .ost-ci
{resolve_version}
          echo \"$version\"
{assert}\
\x20         printf '{{\"schema\":1,\"ost_version\":\"%s\"}}\\n' \"$version\" > .ost-ci/ost-version.json
"
    )
}

/// The registry cache + remote pull steps. Cache is a bandwidth/time
/// optimization keyed by {{ost-version, os, arch, support line, runtime
/// digest}} — never branch names or run ids — and never a correctness
/// precondition: a miss (or the `OST_CI_DISABLE_CACHE` repository variable)
/// falls back to the digest-pinned remote pull, and a poisoned hosted cache
/// is wiped and re-pulled rather than trusted.
fn runtime_fetch_steps(bootstrap: Option<&Bootstrap>) -> String {
    let mut out = String::new();
    if let Some(bootstrap) = bootstrap {
        out.push_str(&format!(
            "\
\x20     - name: Restore the artifact registry cache (speed only, never correctness)
        if: ${{{{ matrix.hosted && vars.OST_CI_DISABLE_CACHE != 'true' }}}}
        uses: {CACHE}
        with:
          path: .ost-ci-home/artifacts
          key: ost-registry-{version}-${{{{ runner.os }}}}-${{{{ runner.arch }}}}-${{{{ matrix.name }}}}-${{{{ matrix.runtime_artifact }}}}
",
            version = bootstrap.ost.version,
        ));
    }
    out.push_str(
        "\
\x20     - name: Pull the pinned runtime SDK from its remote reference
        if: ${{ matrix.runtime_remote != '' }}
        shell: bash
        run: |
          set -euo pipefail
          mkdir -p .ost-ci
          if ost artifact show \"${{ matrix.runtime_artifact }}\" --json > /dev/null 2>&1 \\
             && ost artifact verify \"${{ matrix.runtime_artifact }}\" --json > .ost-ci/runtime-cache-verify.json; then
            echo \"pinned runtime already present and verified (cache hit) -- skipping the remote pull\"
          else
            if [ \"${{ matrix.hosted }}\" = \"true\" ] && [ -n \"${OST_HOME:-}\" ]; then
              rm -rf \"${OST_HOME}/artifacts\"
            fi
            ost artifact pull \"${{ matrix.runtime_remote }}\" --expect-artifact \"${{ matrix.runtime_artifact }}\" --require-kind runtime --json | tee .ost-ci/runtime-pull.json
          fi
",
    );
    out
}

/// Render the matrix's repo-specific `source_checks` as workflow steps,
/// spliced in after the verification pyramid. Each check is a quoted `- name:`
/// line plus a literal block scalar (`run: |`) whose every line is re-indented to
/// 10 spaces, so a multi-line script stays inside its own step (the validator
/// already rejected control chars and structural breakouts). Empty when the
/// matrix declares no checks, so it renders nothing.
fn source_check_steps(checks: &[SourceCheck]) -> String {
    let mut out = String::new();
    for check in checks {
        out.push_str(&format!(
            "      - name: \"{name}\"\n        shell: bash\n        run: |\n",
            name = check.name,
        ));
        for line in check.run.lines() {
            if line.is_empty() {
                out.push('\n');
            } else {
                out.push_str(&format!("          {line}\n"));
            }
        }
    }
    out
}

/// Modeled pre-build prerequisites, rendered as a first-class section *between*
/// runtime materialization and `ost plugin build` — deliberately distinct from
/// `source_checks`, which run after the verification pyramid and so are too late
/// for anything the build depends on (v0.12.0 macOS dogfood). Two prerequisites
/// are modeled today:
///
/// - **Runnable-runtime validation** (always): `ost runtime validate` re-checks
///   the freshly materialized tree, including the Unix `bin-tools-executable`
///   invariant, so a runtime whose tools lost their execute bits fails *here*
///   with visible evidence instead of deep inside `usdGenSchema`.
/// - **Host Python for schema tooling** (when a source cell declares
///   `host_python`): a pinned `setup-python` installs exactly the declared
///   CPython ABI on a hosted runner before the build, so schema-generate never
///   relies on an accidental host interpreter. The step is per-cell gated on
///   `matrix.hosted && matrix.host_python`, and every cell records the resolved
///   Python source as CI evidence.
///
/// No arbitrary pre-build shell hook is offered: prerequisites are modeled, not
/// scripted (roadmap v0.12.0 P1).
fn prebuild_steps(matrix: &SupportMatrix) -> String {
    let mut out = String::from(
        "\
\x20     - name: Validate the materialized runtime (runnable tools)
        shell: bash
        run: |
          set -euo pipefail
          mkdir -p .ost-ci
          ost runtime validate ${{ matrix.platform }} --profile ${{ matrix.profile }} --json | tee .ost-ci/runtime-validate.json
",
    );
    if matrix.needs_host_python() {
        out.push_str(&format!(
            "\
\x20     - name: Set up host Python for schema tooling
        if: ${{{{ matrix.hosted && matrix.host_python != '' }}}}
        uses: {SETUP_PYTHON}
        with:
          python-version: ${{{{ matrix.host_python }}}}
      - name: Record the schema-tooling Python contract
        shell: bash
        run: |
          set -euo pipefail
          mkdir -p .ost-ci
          if [ \"${{{{ matrix.hosted }}}}\" = \"true\" ] && [ -n \"${{{{ matrix.host_python }}}}\" ]; then
            source=host-setup-python
          elif [ -n \"${{{{ matrix.host_python }}}}\" ]; then
            source=operator-provisioned
          else
            source=runtime-bundled
          fi
          printf '{{\"schema\":1,\"host_python\":\"%s\",\"source\":\"%s\"}}\\n' \"${{{{ matrix.host_python }}}}\" \"$source\" > .ost-ci/python-setup.json
",
        ));
    }
    out
}

/// The shared step list of a source-CI job.
fn source_steps(matrix: &SupportMatrix) -> String {
    let bootstrap = matrix
        .bootstrap
        .as_ref()
        .map(|bootstrap| bootstrap_step(bootstrap, false))
        .unwrap_or_default();
    let policy = matrix
        .trust
        .policy
        .as_deref()
        .map(|path| format!(" --policy {path}"))
        .unwrap_or_default();
    format!(
        "\
\x20   steps:
      - name: Check out the repository
        uses: {CHECKOUT}
{BILLING_NOTICE}\
{bootstrap}\
{version_check}\
\x20     - name: Validate the CI manifest
        shell: bash
        run: ost ci validate
{fetch}\
\x20     - name: Verify and materialize the pinned runtime SDK
        shell: bash
        run: |
          set -euo pipefail
          mkdir -p .ost-ci
          printf '{{\"schema\":1,\"runtime_artifact\":\"%s\",\"source\":\"%s\"}}\\n' \"${{{{ matrix.runtime_artifact }}}}\" \"${{{{ matrix.runtime_remote != '' && 'remote-pull' || 'local-registry' }}}}\" > .ost-ci/runtime-source.json
          ost artifact verify ${{{{ matrix.runtime_artifact }}}} --minimum-trust ${{{{ matrix.minimum_trust }}}} ${{{{ matrix.evidence_flags }}}}{policy}
          ost runtime pull ${{{{ matrix.platform }}}} --profile ${{{{ matrix.profile }}}} --from-artifact ${{{{ matrix.runtime_artifact }}}} --force
{prebuild}\
\x20     - name: Build the plugin from source
        shell: bash
        run: ost plugin build ${{{{ matrix.bundle }}}} --target ${{{{ matrix.platform }}}} --profile ${{{{ matrix.profile }}}}
      - name: Run the verification pyramid
        shell: bash
        run: ost plugin test ${{{{ matrix.bundle }}}} --target ${{{{ matrix.platform }}}} --profile ${{{{ matrix.profile }}}} --up-to ${{{{ matrix.up_to }}}} --json
{checks}\
\x20     - name: Package the plugin (never published from this workflow)
        shell: bash
        run: ost plugin package ${{{{ matrix.bundle }}}} --target ${{{{ matrix.platform }}}} --profile ${{{{ matrix.profile }}}}
      - name: Upload the verification report and CI evidence
        if: always()
        uses: {UPLOAD_ARTIFACT}
        with:
          name: report-${{{{ matrix.name }}}}
          path: |
            ${{{{ matrix.bundle }}}}/.strata/reports/
            .ost-ci/
",
        version_check = ost_version_step(matrix.bootstrap.as_ref()),
        fetch = runtime_fetch_steps(matrix.bootstrap.as_ref()),
        prebuild = prebuild_steps(matrix),
        checks = source_check_steps(&matrix.source_checks),
    )
}

/// One source-CI job (`pr` or `mainline`) over the given cells.
fn source_job(matrix: &SupportMatrix, id: &str, event: &str, cells: &[&SupportCell]) -> String {
    let mut include = String::new();
    for cell in cells {
        let bundle = cell.bundle.as_deref().unwrap_or(".");
        let remote = cell
            .runtime_remote
            .as_ref()
            .map(|r| r.uri.as_str())
            .unwrap_or("");
        let host_python = cell.host_python.as_deref().unwrap_or("");
        include.push_str(&include_entry(
            matrix,
            cell,
            &format!(
                "            bundle: {bundle}\n\
                 \x20           runtime_remote: \"{remote}\"\n\
                 \x20           host_python: \"{host_python}\"\n"
            ),
        ));
    }
    // Hosted cells get a workspace-local OST_HOME so the registry the cache
    // step saves/restores has a deterministic path on every runner OS;
    // self-hosted cells resolve to '' and keep the operator's store (an empty
    // OST_HOME is treated as unset).
    let ost_home = if matrix.bootstrap.is_some() {
        "\n      OST_HOME: ${{ matrix.hosted && format('{0}/.ost-ci-home', github.workspace) || '' }}"
    } else {
        ""
    };
    format!(
        "\
\x20 {id}:
    if: github.event_name == '{event}'
    name: ${{{{ matrix.name }}}}
    runs-on: ${{{{ matrix.runs_on }}}}
{env}{ost_home}
    strategy:
      fail-fast: false
      matrix:
        include:
{include}\
{steps}",
        env = ci_env(false),
        steps = source_steps(matrix),
    )
}

/// Render the source-CI workflow (`pull_request`/`main` cells), or `None`
/// when the matrix declares no source cells.
pub fn generate_source(matrix: &SupportMatrix) -> Option<String> {
    let source = matrix.source_cells();
    if source.is_empty() {
        return None;
    }
    let pr: Vec<&SupportCell> = source
        .iter()
        .filter(|c| c.lane == Lane::PullRequest)
        .copied()
        .collect();
    let mainline: Vec<&SupportCell> = source
        .iter()
        .filter(|c| c.lane == Lane::Main)
        .copied()
        .collect();

    let mut on = String::new();
    if !pr.is_empty() {
        on.push_str("  pull_request:\n");
    }
    if !mainline.is_empty() {
        on.push_str("  push:\n    branches: [main]\n");
    }

    let mut jobs = String::new();
    if !pr.is_empty() {
        jobs.push_str(&source_job(matrix, "pr", "pull_request", &pr));
    }
    if !mainline.is_empty() {
        jobs.push_str(&source_job(matrix, "mainline", "push", &mainline));
    }

    Some(format!(
        "\
# Generated by `ost ci generate github` from openstrata.ci.yaml.
# Regenerate after editing the matrix; do not edit the jobs by hand.
#
# Source CI: each job checks out the repo, obtains a digest-pinned runtime
# SDK artifact, and builds/tests/packages the bundle from source. On
# GitHub-hosted runners the job bootstraps a pinned, checksum-verified `ost`
# and pulls the runtime from the cell's remote (oci://) reference — an
# actions/cache restore keyed by digest is a speed optimization, never a
# correctness precondition. Self-hosted runners keep their operator-managed
# `ost` and registry (air-gapped local import stays supported).
#
# Fork-PR safety: this workflow never publishes, never promotes, and uses no
# secrets; keep it that way when editing the matrix.
name: ost source ci

on:
{on}
permissions:
  contents: read

jobs:
{jobs}"
    ))
}

fn release_candidate_steps(matrix: &SupportMatrix) -> String {
    let release = matrix.release.as_ref().expect("validated release block");
    let bootstrap = matrix
        .bootstrap
        .as_ref()
        .map(|bootstrap| bootstrap_step(bootstrap, true))
        .unwrap_or_default();
    let policy = matrix
        .trust
        .policy
        .as_deref()
        .expect("validated release policy");
    let reproducibility = if release.reproducible {
        "      - name: Repackage and prove reproducibility\n        shell: bash\n        run: |\n          set -euo pipefail\n          sums=\"$(find \"${{ matrix.bundle }}/.strata/dist\" -type f -name SHA256SUMS -print -quit)\"\n          if [ -z \"$sums\" ]; then echo \"::error title=release::package produced no SHA256SUMS\"; exit 1; fi\n          cp \"$sums\" .ost-release/first-SHA256SUMS\n          ost plugin package ${{ matrix.bundle }} --target ${{ matrix.platform }} --profile ${{ matrix.profile }}\n          cmp .ost-release/first-SHA256SUMS \"$sums\"\n"
    } else {
        ""
    };
    let from_package = if release.from_package {
        "      - name: Test the clean extracted package\n        shell: bash\n        run: ost plugin test ${{ matrix.bundle }} --target ${{ matrix.platform }} --profile ${{ matrix.profile }} --up-to ${{ matrix.up_to }} --from-package --json\n"
    } else {
        ""
    };
    let mut checks = source_check_steps(&matrix.source_checks);
    checks.push_str(&source_check_steps(&release.checks));
    format!(
        "\
\x20   steps:
      - name: Check out the repository
        uses: {CHECKOUT}
{BILLING_NOTICE}\
{bootstrap}\
{version_check}\
\x20     - name: Enforce tag and bundle version agreement
        shell: bash
        run: |
          set -euo pipefail
          expected=\"v{release_version}\"
          if [ \"$GITHUB_REF_TYPE\" != tag ] || [ \"$GITHUB_REF_NAME\" != \"$expected\" ]; then
            echo \"::error title=release ref::expected tag $expected, got $GITHUB_REF_TYPE $GITHUB_REF_NAME\"; exit 1
          fi
          mkdir -p .ost-release
          ost --json plugin inspect ${{{{ matrix.bundle }}}} --expect-version {release_version} > .ost-release/inspect.json
      - name: Validate the trusted CI manifest
        shell: bash
        run: ost ci validate
{fetch}\
\x20     - name: Verify and materialize the pinned runtime SDK
        shell: bash
        run: |
          set -euo pipefail
          mkdir -p .ost-ci
          ost artifact verify ${{{{ matrix.runtime_artifact }}}} --minimum-trust ${{{{ matrix.minimum_trust }}}} ${{{{ matrix.evidence_flags }}}} --policy {policy}
          ost runtime pull ${{{{ matrix.platform }}}} --profile ${{{{ matrix.profile }}}} --from-artifact ${{{{ matrix.runtime_artifact }}}} --force
{prebuild}\
\x20     - name: Build the release candidate from source
        shell: bash
        run: ost plugin build ${{{{ matrix.bundle }}}} --target ${{{{ matrix.platform }}}} --profile ${{{{ matrix.profile }}}}
      - name: Run the release verification pyramid
        shell: bash
        run: ost plugin test ${{{{ matrix.bundle }}}} --target ${{{{ matrix.platform }}}} --profile ${{{{ matrix.profile }}}} --up-to ${{{{ matrix.up_to }}}} --json
{checks}\
\x20     - name: Package the lean release candidate
        shell: bash
        run: |
          set -euo pipefail
          mkdir -p .ost-release
          ost plugin package ${{{{ matrix.bundle }}}} --target ${{{{ matrix.platform }}}} --profile ${{{{ matrix.profile }}}}
{reproducibility}\
{from_package}\
\x20     - name: Verify and stage the immutable candidate
        shell: bash
        run: |
          set -euo pipefail
          ost plugin publish ${{{{ matrix.bundle }}}} --target ${{{{ matrix.platform }}}} --profile ${{{{ matrix.profile }}}} | tee .ost-release/publish.txt
          digest=\"$(sed -n 's/^[[:space:]]*digest: \\(sha256:[0-9a-fA-F]\\{{64\\}}\\)$/\\1/p' .ost-release/publish.txt | tail -n 1)\"
          if [ -z \"$digest\" ]; then echo \"::error title=release::publish produced no artifact digest\"; exit 1; fi
          ost artifact verify \"$digest\" --minimum-trust {release_trust} --require-sbom --require-provenance --policy {policy}
          ost artifact export \"$digest\" .ost-release/candidate
          printf '%s\\n' \"$digest\" > .ost-release/candidate/artifact.digest
          printf '%s\\n' \"${{{{ matrix.name }}}}\" > .ost-release/candidate/cell.name
      - name: Upload the immutable candidate handoff
        uses: {UPLOAD_ARTIFACT}
        with:
          name: candidate-${{{{ matrix.name }}}}
          path: .ost-release/candidate/
          if-no-files-found: error
      - name: Upload release reports and CI evidence
        if: always()
        uses: {UPLOAD_ARTIFACT}
        with:
          name: release-report-${{{{ matrix.name }}}}
          path: |
            ${{{{ matrix.bundle }}}}/.strata/reports/
            .ost-ci/
            .ost-release/inspect.json
            .ost-release/publish.txt
",
        release_version = release.version,
        release_trust = matrix.trust.release_min_trust,
        version_check = ost_version_step(matrix.bootstrap.as_ref()),
        fetch = runtime_fetch_steps(matrix.bootstrap.as_ref()),
        prebuild = prebuild_steps(matrix),
    )
}

fn release_candidate_job(matrix: &SupportMatrix, cells: &[&SupportCell]) -> String {
    let mut include = String::new();
    for cell in cells {
        let bundle = cell.bundle.as_deref().unwrap_or(".");
        let remote = cell
            .runtime_remote
            .as_ref()
            .map(|reference| reference.uri.as_str())
            .unwrap_or("");
        let host_python = cell.host_python.as_deref().unwrap_or("");
        include.push_str(&include_entry(
            matrix,
            cell,
            &format!(
                "            bundle: {bundle}\n\
                 \x20           runtime_remote: \"{remote}\"\n\
                 \x20           host_python: \"{host_python}\"\n"
            ),
        ));
    }
    let ost_home = if matrix.bootstrap.is_some() {
        "\n      OST_HOME: ${{ matrix.hosted && format('{0}/.ost-release-home', github.workspace) || '' }}"
    } else {
        ""
    };
    format!(
        "\
\x20 candidates:
    needs: validate-release-ref
    name: candidate ${{{{ matrix.name }}}}
    runs-on: ${{{{ matrix.runs_on }}}}
{env}{ost_home}
    strategy:
      fail-fast: false
      matrix:
        include:
{include}\
{steps}",
        env = ci_env(false),
        steps = release_candidate_steps(matrix),
    )
}

fn release_publisher_job(matrix: &SupportMatrix) -> String {
    let release = matrix.release.as_ref().expect("validated release block");
    if release.mode != ReleaseMode::Publish {
        return String::new();
    }
    let runner_name = release
        .publisher_runner
        .as_deref()
        .expect("validated publisher runner");
    let runner = matrix
        .runners
        .get(runner_name)
        .expect("validated publisher runner profile");
    let runs_on = runner
        .runs_on()
        .into_iter()
        .map(|label| format!("\"{label}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let environment = release
        .environment
        .as_deref()
        .map(|name| format!("    environment: {name}\n"))
        .unwrap_or_default();
    let bootstrap = matrix
        .bootstrap
        .as_ref()
        .map(|bootstrap| bootstrap_step(bootstrap, true))
        .unwrap_or_default();
    let policy = matrix
        .trust
        .policy
        .as_deref()
        .expect("validated release policy");
    let destination = release
        .destination
        .as_deref()
        .expect("validated release destination");
    format!(
        "\
\x20 publish:
    needs: candidates
    name: Publish verified candidates
    runs-on: ${{{{ matrix.runs_on }}}}
{environment}\
\x20   permissions:
      contents: read
      id-token: write
      packages: write
    env:
      OST_HOME: ${{{{ github.workspace }}}}/.ost-publish-home
    strategy:
      matrix:
        include:
          - name: {runner_name}
            hosted: {hosted}
            runs_on: [{runs_on}]
    steps:
      - name: Check out the repository trust policy
        uses: {CHECKOUT}
{bootstrap}\
{version_check}\
\x20     - name: Download immutable candidate handoffs
        uses: {DOWNLOAD_ARTIFACT}
        with:
          pattern: candidate-*
          path: .ost-release/candidates
      - name: Re-verify and publish candidates
        shell: bash
        # The registry credential is step-scoped: bootstrap and download run
        # without it.
        env:
          OST_REGISTRY_USER: ${{{{ github.actor }}}}
          OST_REGISTRY_PASSWORD: ${{{{ secrets.GITHUB_TOKEN }}}}
        run: |
          set -euo pipefail
          mkdir -p .ost-release/results
          found=0
          for candidate in .ost-release/candidates/candidate-*; do
            [ -d \"$candidate\" ] || continue
            found=1
            digest=\"$(tr -d '\\r\\n' < \"$candidate/artifact.digest\")\"
            cell=\"$(tr -d '\\r\\n' < \"$candidate/cell.name\")\"
            case \"$digest\" in sha256:[0-9a-fA-F]*) ;; *) echo \"::error title=release::invalid candidate digest\"; exit 1 ;; esac
            ost artifact import \"$candidate\"
            ost artifact verify \"$digest\" --minimum-trust {release_trust} --require-sbom --require-provenance --policy {policy}
            ost --json artifact push \"$digest\" \"{destination}:{release_version}-$cell\" --policy {policy} > \".ost-release/results/$cell.json\"
          done
          if [ \"$found\" != 1 ]; then echo \"::error title=release::no candidate handoffs downloaded\"; exit 1; fi
      - name: Upload publication evidence
        if: always()
        uses: {UPLOAD_ARTIFACT}
        with:
          name: publication-evidence
          path: .ost-release/results/
          if-no-files-found: error
",
        hosted = runner.is_hosted(),
        release_version = release.version,
        release_trust = matrix.trust.release_min_trust,
        version_check = ost_version_step(matrix.bootstrap.as_ref()),
    )
}

/// Render the tag-triggered trusted release workflow, or `None` when the
/// matrix has no release contract.
pub fn generate_release(matrix: &SupportMatrix) -> Option<String> {
    let release = matrix.release.as_ref()?;
    let candidates = matrix.candidate_cells();
    let candidate_job = release_candidate_job(matrix, &candidates);
    let publisher = release_publisher_job(matrix);
    Some(format!(
        "\
# Generated by `ost ci generate github` from openstrata.ci.yaml.
# Regenerate after editing the matrix; do not edit the jobs by hand.
#
# Trusted release: a read-only tag/ref gate feeds isolated candidate builders.
# Only the final publisher receives OIDC/package permissions, and only after
# every candidate passes reproducibility, clean-package, evidence, and policy
# verification. Draft mode intentionally omits that publisher job.
name: ost trusted release

on:
  push:
    tags: [\"v*\"]

permissions:
  contents: read

jobs:
  validate-release-ref:
    name: Validate release ref
    runs-on: ubuntu-latest
    steps:
      - name: Require the exact release tag
        shell: bash
        run: |
          set -euo pipefail
          expected=\"v{version}\"
          if [ \"$GITHUB_REF_TYPE\" != tag ] || [ \"$GITHUB_REF_NAME\" != \"$expected\" ]; then
            echo \"::error title=release ref::expected tag $expected, got $GITHUB_REF_TYPE $GITHUB_REF_NAME\"; exit 1
          fi
{candidate_job}\
{publisher}",
        version = release.version,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::matrix::{
        Acknowledgement, Billing, HostOs, HostSpec, Lane, OstBootstrap, Publish, ReleaseLane,
        RequireEvidence, RunnerKind, RunnerProfile, RuntimeRemote, SourceCheck, SupportCell,
        TrustRequirements, MATRIX_SCHEMA,
    };
    use ost_artifact::TrustLevel;
    use std::collections::BTreeMap;

    fn cell(name: &str) -> SupportCell {
        SupportCell {
            name: name.into(),
            lane: Lane::default(),
            runner: None,
            support: None,
            require_evidence: None,
            runtime_artifact: format!("sha256:{}", "ab".repeat(32)),
            runtime_remote: None,
            plugin_artifact: Some(format!("sha256:{}", "cd".repeat(32))),
            bundle: None,
            platform: "cy2026".into(),
            profile: "usd".into(),
            up_to: 5,
            host_python: None,
            publish: Publish::default(),
            trust: Default::default(),
            host: HostSpec::default(),
        }
    }

    fn matrix() -> SupportMatrix {
        SupportMatrix {
            schema: MATRIX_SCHEMA,
            trust: Default::default(),
            require_evidence: Default::default(),
            bootstrap: None,
            runners: BTreeMap::new(),
            source_checks: vec![],
            release: None,
            cells: vec![
                SupportCell {
                    up_to: 4,
                    host: HostSpec {
                        os: HostOs::Linux,
                        labels: vec!["self-hosted".into(), "linux".into()],
                    },
                    ..cell("linux-usd-toy")
                },
                SupportCell {
                    host: HostSpec {
                        os: HostOs::Windows,
                        labels: vec![],
                    },
                    ..cell("windows-usd-toy")
                },
            ],
        }
    }

    fn lanes_matrix() -> SupportMatrix {
        let mut runners = BTreeMap::new();
        runners.insert(
            "windows-hosted".to_string(),
            RunnerProfile {
                kind: RunnerKind::GithubHosted,
                image: Some("windows-2022".into()),
                labels: vec![],
                capabilities: vec![],
                billing: None,
            },
        );
        SupportMatrix {
            schema: MATRIX_SCHEMA,
            trust: Default::default(),
            require_evidence: Default::default(),
            bootstrap: Some(Bootstrap {
                ost: OstBootstrap {
                    version: "0.9.0".into(),
                    repository: "animu-sphere/open-strata".into(),
                    sha256: BTreeMap::from([(
                        "x86_64-pc-windows-msvc".to_string(),
                        "ee".repeat(32),
                    )]),
                },
            }),
            runners,
            source_checks: vec![],
            release: None,
            cells: vec![
                SupportCell {
                    lane: Lane::PullRequest,
                    runner: Some("windows-hosted".into()),
                    plugin_artifact: None,
                    bundle: Some("plugins/toy".into()),
                    up_to: 4,
                    runtime_remote: Some(RuntimeRemote {
                        uri: format!(
                            "oci://ghcr.io/owner/openstrata-runtime@sha256:{}",
                            "ee".repeat(32)
                        ),
                        expected_oci_digest: Some(format!("sha256:{}", "ee".repeat(32))),
                    }),
                    ..cell("plugin-pr-windows")
                },
                SupportCell {
                    host: HostSpec {
                        os: HostOs::Linux,
                        labels: vec!["self-hosted".into(), "linux".into()],
                    },
                    ..cell("linux-usd-support")
                },
            ],
        }
    }

    fn release_matrix() -> SupportMatrix {
        let mut matrix = lanes_matrix();
        matrix.runners.get_mut("windows-hosted").unwrap().billing = Some(Billing {
            acknowledgement: Acknowledgement::Required,
        });
        matrix.trust = TrustRequirements {
            policy: Some("openstrata-artifact-policy.toml".into()),
            pr_min_trust: TrustLevel::Attested,
            main_min_trust: TrustLevel::Verified,
            release_min_trust: TrustLevel::Trusted,
        };
        matrix.cells[0].lane = Lane::Main;
        matrix.cells[0].publish = Publish::Candidate;
        matrix.cells[0].trust = TrustLevel::Trusted;
        matrix.release = Some(ReleaseLane {
            version: "1.2.3".into(),
            mode: ReleaseMode::Publish,
            destination: Some("oci://ghcr.io/owner/plugin".into()),
            publisher_runner: Some("windows-hosted".into()),
            environment: Some("release".into()),
            reproducible: true,
            from_package: true,
            checks: vec![SourceCheck {
                name: "Release corpus smoke".into(),
                run: "ctest --test-dir build/corpus --output-on-failure".into(),
            }],
        });
        matrix.validate().unwrap();
        matrix
    }

    #[test]
    fn workflow_is_deterministic_valid_yaml_with_one_entry_per_cell() {
        let a = generate_support(&matrix()).unwrap();
        let b = generate_support(&matrix()).unwrap();
        assert_eq!(a, b, "generation is deterministic");

        // It parses as YAML and carries an explicit include per cell.
        let doc: serde_yaml::Value = serde_yaml::from_str(&a).unwrap();
        let include = &doc["jobs"]["scheduled"]["strategy"]["matrix"]["include"];
        let entries = include.as_sequence().unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0]["name"], "linux-usd-toy");
        assert_eq!(
            entries[0]["runtime_artifact"],
            format!("sha256:{}", "ab".repeat(32))
        );
        assert_eq!(entries[0]["up_to"], 4);
        // Labels win; a label-less cell falls back to the hosted runner.
        assert_eq!(entries[0]["runs_on"].as_sequence().unwrap().len(), 2);
        assert_eq!(entries[1]["runs_on"][0], "windows-latest");
        assert_eq!(entries[1]["hosted"], true);

        // `steps` must live under the job. A column-0 `steps:` (the v0.6.0
        // string-continuation bug) still *parses* as YAML — it just becomes a
        // stray top-level key — so assert placement, not merely parseability.
        let steps = doc["jobs"]["scheduled"]["steps"].as_sequence().unwrap();
        assert_eq!(steps.len(), 7);
        assert!(doc.get("steps").is_none(), "no stray top-level steps key");

        // Never a Cartesian product: only `include`, no free axes.
        let m = doc["jobs"]["scheduled"]["strategy"]["matrix"]
            .as_mapping()
            .unwrap();
        assert_eq!(m.len(), 1, "matrix has only the include list");

        // Scheduled, not per-PR; fail-fast off; actions SHA-pinned.
        assert!(doc["on"]["schedule"].is_sequence());
        assert!(doc["on"].get("workflow_dispatch").is_none());
        assert_eq!(
            doc["jobs"]["scheduled"]["if"],
            "github.event_name == 'schedule'"
        );
        assert_eq!(doc["jobs"]["scheduled"]["strategy"]["fail-fast"], false);
        assert!(a.contains("actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a"));
        assert!(!a.contains("@v"), "no mutable action tags");

        // CI evidence: the job exports the OST_CI_* contract so every report
        // written inside it records which support claim it proves.
        let env = &doc["jobs"]["scheduled"]["env"];
        assert_eq!(env["OST_CI_CELL"], "${{ matrix.name }}");
        assert_eq!(env["OST_CI_LANE"], "${{ matrix.lane }}");
        assert_eq!(
            env["OST_CI_PLUGIN_ARTIFACT"],
            "${{ matrix.plugin_artifact }}"
        );
        assert_eq!(entries[0]["lane"], "scheduled");

        // All cells are lane-default (scheduled): no source workflow.
        assert!(generate_source(&matrix()).is_none());
        assert_eq!(generate_github(&matrix()).len(), 1);
    }

    #[test]
    fn source_workflow_builds_from_source_on_the_profile_runner() {
        let m = lanes_matrix();
        let a = generate_source(&m).unwrap();
        assert_eq!(a, generate_source(&m).unwrap(), "deterministic");

        let doc: serde_yaml::Value = serde_yaml::from_str(&a).unwrap();
        // PR cells only: a pull_request trigger, no push trigger, one pr job.
        assert!(doc["on"]
            .as_mapping()
            .unwrap()
            .contains_key(serde_yaml::Value::from("pull_request")));
        assert!(!doc["on"]
            .as_mapping()
            .unwrap()
            .contains_key(serde_yaml::Value::from("push")));
        let entries = doc["jobs"]["pr"]["strategy"]["matrix"]["include"]
            .as_sequence()
            .unwrap();
        assert_eq!(entries.len(), 1);
        // The hosted profile maps to its fixed image and is flagged hosted.
        assert_eq!(entries[0]["runs_on"][0], "windows-2022");
        assert_eq!(entries[0]["hosted"], true);
        assert_eq!(entries[0]["bundle"], "plugins/toy");
        // No plugin artifact: the bundle is built from the checkout.
        assert!(entries[0].get("plugin_artifact").is_none());

        let steps = doc["jobs"]["pr"]["steps"].as_sequence().unwrap();
        assert!(doc.get("steps").is_none(), "steps live under the job");
        let names: Vec<&str> = steps.iter().map(|s| s["name"].as_str().unwrap()).collect();
        assert!(names.iter().any(|n| n.contains("billing notice")));
        assert!(names.iter().any(|n| n.contains("Build the plugin")));
        // Fork-PR safety: read-only token, no publish step, no secrets.
        assert_eq!(doc["permissions"]["contents"], "read");
        assert!(!a.contains("plugin publish"), "source CI never publishes");
        assert!(!a.contains("secrets."), "source CI uses no secrets");

        // CI evidence: lane + profile travel in the include entry, and the
        // job exports the OST_CI_* contract (no plugin digest on source CI —
        // the bundle is built from the checkout).
        assert_eq!(entries[0]["lane"], "pull_request");
        assert_eq!(entries[0]["runner_profile"], "windows-hosted");
        let env = &doc["jobs"]["pr"]["env"];
        assert_eq!(env["OST_CI_CELL"], "${{ matrix.name }}");
        assert_eq!(env["OST_CI_RUNS_ON"], "${{ join(matrix.runs_on, ',') }}");
        assert!(env.get("OST_CI_PLUGIN_ARTIFACT").is_none());

        // The support half renders the scheduled cell alongside.
        let workflows = generate_github(&m);
        assert_eq!(workflows.len(), 2);
        assert_eq!(workflows[0].path, WORKFLOW_PATH);
        assert_eq!(workflows[1].path, SOURCE_WORKFLOW_PATH);
    }

    #[test]
    fn hosted_bootstrap_cache_and_remote_pull_render_into_source_ci() {
        let m = lanes_matrix();
        let a = generate_source(&m).unwrap();
        let doc: serde_yaml::Value = serde_yaml::from_str(&a).unwrap();
        let steps = doc["jobs"]["pr"]["steps"].as_sequence().unwrap();
        let names: Vec<&str> = steps.iter().map(|s| s["name"].as_str().unwrap()).collect();

        // Bootstrap: hosted-gated, version-pinned URL, checksum verification,
        // the matrix's exact-byte pin, and evidence output.
        let bootstrap = steps
            .iter()
            .find(|s| {
                s["name"]
                    .as_str()
                    .unwrap()
                    .starts_with("Bootstrap ost 0.9.0")
            })
            .expect("bootstrap step");
        assert_eq!(bootstrap["if"], "${{ matrix.hosted }}");
        let run = bootstrap["run"].as_str().unwrap();
        assert!(
            run.contains("https://github.com/animu-sphere/open-strata/releases/download/v0.9.0")
        );
        assert!(run.contains("ost-cli-${triple}.${ext}"));
        assert!(run.contains(&"ee".repeat(32)), "matrix sha256 pin rendered");
        assert!(run.contains(".ost-ci/bootstrap.json"));
        assert!(run.contains("bin=\"$bin_dir/$(basename \"$bin\")\""));
        assert!(run.contains("cygpath -w \"$bin_dir\""));
        assert!(run.contains("echo \"$exported_path\" >> \"$GITHUB_PATH\""));
        assert!(run.contains("\"executable\":\"%s\""));
        assert!(run.contains("\"exported_path\":\"%s\""));

        // The next step asks native PowerShell process creation to find and
        // launch ost.exe. Quoted conversion/export variables preserve paths
        // containing spaces.
        let check = steps
            .iter()
            .find(|s| s["name"].as_str().unwrap().contains("record its version"))
            .expect("version step");
        let run = check["run"].as_str().unwrap();
        assert!(run.contains("ost 0.9.0"));
        assert!(run.contains("powershell -NoProfile -Command \"& ost.exe --version\""));

        // Cache: digest-keyed, hosted-gated, disableable, never branch/run ids.
        let cache = steps
            .iter()
            .find(|s| s["name"].as_str().unwrap().contains("registry cache"))
            .expect("cache step");
        assert!(cache["uses"]
            .as_str()
            .unwrap()
            .starts_with("actions/cache@"));
        let key = cache["with"]["key"].as_str().unwrap();
        assert!(key.contains("0.9.0"));
        assert!(key.contains("${{ matrix.runtime_artifact }}"));
        assert!(
            !key.contains("github.ref"),
            "cache identity is never a branch"
        );
        assert!(!key.contains("run_id"), "cache identity is never a run id");
        assert!(cache["if"]
            .as_str()
            .unwrap()
            .contains("vars.OST_CI_DISABLE_CACHE != 'true'"));

        // Remote pull: gated on the cell's remote reference, digest-pinned,
        // kind-checked, evidence-teed; cache hit skips it via verify.
        let pull = steps
            .iter()
            .find(|s| s["name"].as_str().unwrap().contains("remote reference"))
            .expect("pull step");
        assert_eq!(pull["if"], "${{ matrix.runtime_remote != '' }}");
        let run = pull["run"].as_str().unwrap();
        assert!(run.contains("--expect-artifact"));
        assert!(run.contains("--require-kind runtime"));
        assert!(run.contains(".ost-ci/runtime-pull.json"));

        // The include entry carries the pinned remote uri; the self-hosted
        // mainline path stays possible (empty remote renders as "").
        let entries = doc["jobs"]["pr"]["strategy"]["matrix"]["include"]
            .as_sequence()
            .unwrap();
        assert!(entries[0]["runtime_remote"]
            .as_str()
            .unwrap()
            .starts_with("oci://ghcr.io/owner/openstrata-runtime@sha256:"));

        // Hosted cells get a workspace-local OST_HOME; the expression falls
        // back to '' (treated as unset) for self-hosted cells.
        let env = &doc["jobs"]["pr"]["env"];
        assert!(env["OST_HOME"].as_str().unwrap().contains("matrix.hosted"));

        // Evidence travels with the report upload.
        let upload = steps.last().unwrap();
        assert!(upload["with"]["path"]
            .as_str()
            .unwrap()
            .contains(".ost-ci/"));

        // Still no secrets, still never publishes.
        assert!(!a.contains("secrets."), "source CI uses no secrets");
        assert!(!a.contains("plugin publish"), "source CI never publishes");
        assert!(names.iter().any(|n| n.contains("Validate the CI manifest")));
    }

    #[test]
    fn source_checks_render_as_steps_after_the_pyramid() {
        let mut m = lanes_matrix();
        m.source_checks = vec![
            SourceCheck {
                name: "Run corpus CTest smoke".into(),
                run: "set -euo pipefail\nctest --test-dir build/corpus --output-on-failure".into(),
            },
            SourceCheck {
                name: "- Assert schema round-trips".into(),
                run: "python tools/check_corpus.py".into(),
            },
        ];
        let a = generate_source(&m).unwrap();
        assert_eq!(a, generate_source(&m).unwrap(), "deterministic");

        let doc: serde_yaml::Value = serde_yaml::from_str(&a).unwrap();
        let steps = doc["jobs"]["pr"]["steps"].as_sequence().unwrap();
        let names: Vec<&str> = steps.iter().map(|s| s["name"].as_str().unwrap()).collect();

        // Both checks render, in declared order.
        let smoke = names
            .iter()
            .position(|n| *n == "Run corpus CTest smoke")
            .expect("corpus check present");
        let schema = names
            .iter()
            .position(|n| *n == "- Assert schema round-trips")
            .expect("schema check present");
        assert!(smoke < schema, "checks keep declared order");

        // Placed after the verification pyramid and before packaging — the
        // built plugin is present, the package step still runs last.
        let pyramid = names
            .iter()
            .position(|n| n.contains("verification pyramid"))
            .unwrap();
        let package = names
            .iter()
            .position(|n| n.contains("Package the plugin"))
            .unwrap();
        assert!(
            pyramid < smoke && schema < package,
            "checks sit post-build, pre-package"
        );

        // The multi-line run is preserved verbatim as a block scalar.
        let step = &steps[smoke];
        assert_eq!(step["shell"], "bash");
        let run = step["run"].as_str().unwrap();
        assert!(run.contains("set -euo pipefail"));
        assert!(run.contains("ctest --test-dir build/corpus"));

        // Still fork-PR safe.
        assert!(!a.contains("secrets."), "source CI uses no secrets");
        assert!(!a.contains("plugin publish"), "source CI never publishes");

        // No checks -> no extra steps (the baseline lanes matrix).
        let plain = generate_source(&lanes_matrix()).unwrap();
        let pdoc: serde_yaml::Value = serde_yaml::from_str(&plain).unwrap();
        let pnames: Vec<&str> = pdoc["jobs"]["pr"]["steps"]
            .as_sequence()
            .unwrap()
            .iter()
            .map(|s| s["name"].as_str().unwrap())
            .collect();
        assert!(!pnames.iter().any(|n| n.contains("corpus")));
    }

    #[test]
    fn runtime_validation_renders_between_materialize_and_build() {
        // The runnable-runtime check is a modeled pre-build prerequisite: it
        // always renders, after materialization and before the build, so a
        // runtime whose tools lost their execute bits fails in CI rather than
        // silently inside usdGenSchema (v0.12.0 P0).
        let a = generate_source(&lanes_matrix()).unwrap();
        let doc: serde_yaml::Value = serde_yaml::from_str(&a).unwrap();
        let steps = doc["jobs"]["pr"]["steps"].as_sequence().unwrap();
        let names: Vec<&str> = steps.iter().map(|s| s["name"].as_str().unwrap()).collect();

        let materialize = names
            .iter()
            .position(|n| n.contains("materialize the pinned runtime"))
            .expect("materialize step");
        let validate = names
            .iter()
            .position(|n| n.contains("Validate the materialized runtime"))
            .expect("runtime validate step");
        let build = names
            .iter()
            .position(|n| n.contains("Build the plugin"))
            .expect("build step");
        assert!(
            materialize < validate && validate < build,
            "runtime validate sits between materialize and build: {names:?}"
        );
        let run = steps[validate]["run"].as_str().unwrap();
        assert!(run.contains("ost runtime validate"));
        assert!(
            run.contains(".ost-ci/runtime-validate.json"),
            "evidence teed"
        );

        // No host_python declared -> no setup-python step at all.
        assert!(
            !names.iter().any(|n| n.contains("Set up host Python")),
            "setup-python only renders when a cell declares host_python: {names:?}"
        );
    }

    #[test]
    fn host_python_renders_setup_python_before_build() {
        let mut m = lanes_matrix();
        m.cells[0].host_python = Some("3.13".into());
        let a = generate_source(&m).unwrap();
        assert_eq!(a, generate_source(&m).unwrap(), "deterministic");
        let doc: serde_yaml::Value = serde_yaml::from_str(&a).unwrap();
        let steps = doc["jobs"]["pr"]["steps"].as_sequence().unwrap();
        let names: Vec<&str> = steps.iter().map(|s| s["name"].as_str().unwrap()).collect();

        let setup = steps
            .iter()
            .position(|s| s["name"].as_str().unwrap().contains("Set up host Python"))
            .expect("setup-python step");
        let build = names
            .iter()
            .position(|n| n.contains("Build the plugin"))
            .unwrap();
        assert!(setup < build, "python setup precedes the build: {names:?}");

        // Pinned action (SEC-004), gated on hosted + a declared ABI, exact ABI.
        let step = &steps[setup];
        assert!(step["uses"]
            .as_str()
            .unwrap()
            .starts_with("actions/setup-python@"));
        // The raw YAML pins a full SHA with a `# vN` comment (SEC-004); YAML
        // strips the comment on parse, so assert it on the rendered text.
        assert!(
            a.contains("actions/setup-python@ece7cb06caefa5fff74198d8649806c4678c61a1 # v6.3.0"),
            "setup-python is SHA-pinned with a version comment"
        );
        assert_eq!(
            step["if"],
            "${{ matrix.hosted && matrix.host_python != '' }}"
        );
        assert_eq!(step["with"]["python-version"], "${{ matrix.host_python }}");

        // The declared ABI travels in the include entry, and the resolved
        // Python source is recorded as CI evidence.
        let entries = doc["jobs"]["pr"]["strategy"]["matrix"]["include"]
            .as_sequence()
            .unwrap();
        assert_eq!(entries[0]["host_python"], "3.13");
        let evidence = steps
            .iter()
            .find(|s| s["name"].as_str().unwrap().contains("Python contract"))
            .expect("python evidence step");
        assert!(evidence["run"]
            .as_str()
            .unwrap()
            .contains(".ost-ci/python-setup.json"));
        assert!(evidence["run"]
            .as_str()
            .unwrap()
            .contains("source=operator-provisioned"));

        // Still fork-PR safe.
        assert!(!a.contains("secrets."), "source CI uses no secrets");
    }

    #[test]
    fn matrix_without_bootstrap_renders_no_hosted_bootstrap_steps() {
        // A self-hosted-only source matrix keeps the operator-managed flow:
        // no bootstrap, no cache, no OST_HOME override.
        let mut m = lanes_matrix();
        m.bootstrap = None;
        m.cells[0].runner = None;
        m.cells[0].runtime_remote = None;
        m.cells[0].host = HostSpec {
            os: HostOs::Linux,
            labels: vec!["self-hosted".into(), "linux".into()],
        };
        let a = generate_source(&m).unwrap();
        let doc: serde_yaml::Value = serde_yaml::from_str(&a).unwrap();
        let steps = doc["jobs"]["pr"]["steps"].as_sequence().unwrap();
        let names: Vec<&str> = steps.iter().map(|s| s["name"].as_str().unwrap()).collect();
        assert!(!names.iter().any(|n| n.starts_with("Bootstrap ost")));
        assert!(!names.iter().any(|n| n.contains("registry cache")));
        assert!(doc["jobs"]["pr"]["env"].get("OST_HOME").is_none());
        // The pull step still renders (gated per cell) but this cell's remote
        // is empty, so it would be skipped at run time.
        assert_eq!(
            doc["jobs"]["pr"]["strategy"]["matrix"]["include"][0]["runtime_remote"],
            ""
        );
    }

    #[test]
    fn main_lane_renders_a_push_gated_job() {
        let mut m = lanes_matrix();
        m.cells[0].lane = Lane::Main;
        let a = generate_source(&m).unwrap();
        let doc: serde_yaml::Value = serde_yaml::from_str(&a).unwrap();
        assert!(doc["on"]
            .as_mapping()
            .unwrap()
            .contains_key(serde_yaml::Value::from("push")));
        assert_eq!(doc["jobs"]["mainline"]["if"], "github.event_name == 'push'");
        assert!(doc["jobs"].get("pr").is_none());
    }

    #[test]
    fn support_lanes_are_event_filtered() {
        let mut m = matrix();
        m.cells[1].lane = Lane::WorkflowDispatch;
        let a = generate_support(&m).unwrap();
        let doc: serde_yaml::Value = serde_yaml::from_str(&a).unwrap();

        assert!(doc["on"]
            .as_mapping()
            .unwrap()
            .contains_key(serde_yaml::Value::from("schedule")));
        assert!(doc["on"]
            .as_mapping()
            .unwrap()
            .contains_key(serde_yaml::Value::from("workflow_dispatch")));
        assert_eq!(
            doc["jobs"]["scheduled"]["if"],
            "github.event_name == 'schedule'"
        );
        assert_eq!(
            doc["jobs"]["dispatch"]["if"],
            "github.event_name == 'workflow_dispatch'"
        );

        let scheduled = doc["jobs"]["scheduled"]["strategy"]["matrix"]["include"]
            .as_sequence()
            .unwrap();
        let dispatch = doc["jobs"]["dispatch"]["strategy"]["matrix"]["include"]
            .as_sequence()
            .unwrap();
        assert_eq!(scheduled.len(), 1);
        assert_eq!(scheduled[0]["name"], "linux-usd-toy");
        assert_eq!(dispatch.len(), 1);
        assert_eq!(dispatch[0]["name"], "windows-usd-toy");
    }

    #[test]
    fn generated_jobs_enforce_effective_trust_and_artifact_evidence() {
        let mut source = lanes_matrix();
        source.trust = TrustRequirements {
            policy: Some("policies/artifacts.toml".into()),
            pr_min_trust: TrustLevel::Attested,
            main_min_trust: TrustLevel::Verified,
            release_min_trust: TrustLevel::Trusted,
        };
        source.cells[0].trust = TrustLevel::Unsigned;
        let yaml = generate_source(&source).unwrap();
        let doc: serde_yaml::Value = serde_yaml::from_str(&yaml).unwrap();
        let include = &doc["jobs"]["pr"]["strategy"]["matrix"]["include"][0];
        assert_eq!(include["target_trust"], "unsigned");
        assert_eq!(include["minimum_trust"], "attested");
        assert_eq!(
            doc["jobs"]["pr"]["env"]["OST_CI_MINIMUM_TRUST"],
            "${{ matrix.minimum_trust }}"
        );
        let verify = doc["jobs"]["pr"]["steps"]
            .as_sequence()
            .unwrap()
            .iter()
            .find(|step| step["name"] == "Verify and materialize the pinned runtime SDK")
            .unwrap()["run"]
            .as_str()
            .unwrap();
        for required in [
            "--minimum-trust ${{ matrix.minimum_trust }}",
            "${{ matrix.evidence_flags }}",
            "--policy policies/artifacts.toml",
        ] {
            assert!(verify.contains(required), "missing {required}: {verify}");
        }
        // The demand itself is a matrix column, so a cell can carry its own
        // level; the default is still both sidecars.
        assert_eq!(include["require_evidence"], "all");
        assert_eq!(
            include["evidence_flags"],
            "--require-sbom --require-provenance"
        );
        assert!(!yaml.contains("artifact push"));
        assert!(!yaml.contains("plugin publish"));

        let mut support = matrix();
        support.trust.policy = Some("policies/artifacts.toml".into());
        support.cells[0].trust = TrustLevel::Verified;
        let yaml = generate_support(&support).unwrap();
        let doc: serde_yaml::Value = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(
            doc["jobs"]["scheduled"]["strategy"]["matrix"]["include"][0]["minimum_trust"],
            "verified"
        );
        let verify = doc["jobs"]["scheduled"]["steps"]
            .as_sequence()
            .unwrap()
            .iter()
            .find(|step| step["name"] == "Verify artifact integrity")
            .unwrap()["run"]
            .as_str()
            .unwrap();
        assert_eq!(verify.matches("${{ matrix.evidence_flags }}").count(), 2);
        assert_eq!(
            doc["jobs"]["scheduled"]["strategy"]["matrix"]["include"][0]["evidence_flags"],
            "--require-sbom --require-provenance"
        );
        assert!(doc["jobs"]["scheduled"]["steps"]
            .as_sequence()
            .unwrap()
            .iter()
            .any(|step| step["name"] == "Check out the repository trust policy"));
    }

    #[test]
    fn require_evidence_is_declarable_globally_and_per_cell() {
        // A repo whose pins predate evidence must be able to render a gate it
        // can actually satisfy while it republishes, rather than adopting a
        // release that reds out every lane with no repo-side cause.
        let mut matrix = matrix();
        matrix.require_evidence = RequireEvidence::None;
        let doc: serde_yaml::Value =
            serde_yaml::from_str(&generate_support(&matrix).unwrap()).unwrap();
        let include = &doc["jobs"]["scheduled"]["strategy"]["matrix"]["include"][0];
        assert_eq!(include["require_evidence"], "none");
        assert_eq!(include["evidence_flags"], "");

        // The gate line is unchanged — only what the column expands to moves,
        // so trust and integrity checks still run either way.
        let verify = doc["jobs"]["scheduled"]["steps"]
            .as_sequence()
            .unwrap()
            .iter()
            .find(|step| step["name"] == "Verify artifact integrity")
            .unwrap()["run"]
            .as_str()
            .unwrap();
        assert!(verify.contains("--minimum-trust ${{ matrix.minimum_trust }}"));
        assert!(verify.contains("${{ matrix.evidence_flags }}"));

        // A cell overrides the document default in either direction.
        let mut mixed = matrix.clone();
        mixed.cells[0].require_evidence = Some(RequireEvidence::Sbom);
        let doc: serde_yaml::Value =
            serde_yaml::from_str(&generate_support(&mixed).unwrap()).unwrap();
        let include = &doc["jobs"]["scheduled"]["strategy"]["matrix"]["include"][0];
        assert_eq!(include["require_evidence"], "sbom");
        assert_eq!(include["evidence_flags"], "--require-sbom");
        assert_eq!(
            mixed.require_evidence(&mixed.cells[0]),
            RequireEvidence::Sbom
        );
        // Uniformity is about the resolved level across all cells: an override
        // on one cell of many makes the document non-uniform.
        assert_eq!(
            matrix.uniform_require_evidence(),
            Some(RequireEvidence::None)
        );
        if mixed.cells.len() > 1 {
            assert_eq!(mixed.uniform_require_evidence(), None);
        }
    }

    #[test]
    fn trusted_release_separates_candidate_and_publisher_permissions() {
        let matrix = release_matrix();
        let yaml = generate_release(&matrix).expect("release workflow");
        assert_eq!(yaml, generate_release(&matrix).unwrap(), "deterministic");
        let doc: serde_yaml::Value = serde_yaml::from_str(&yaml).expect("valid YAML");

        assert_eq!(doc["permissions"]["contents"], "read");
        assert_eq!(doc["jobs"]["candidates"]["needs"], "validate-release-ref");
        assert!(doc["jobs"]["candidates"].get("permissions").is_none());
        let candidate_text = serde_yaml::to_string(&doc["jobs"]["candidates"]).unwrap();
        for gate in [
            "trusted release requires an exact checksum pin",
            "--expect-version 1.2.3",
            "Repackage and prove reproducibility",
            "--from-package",
            "--require-sbom",
            "--require-provenance",
            "--minimum-trust trusted",
            "Release corpus smoke",
        ] {
            assert!(
                candidate_text.contains(gate),
                "missing {gate}:\n{candidate_text}"
            );
        }
        assert!(!candidate_text.contains("secrets."));
        assert!(!candidate_text.contains("artifact push"));

        let publisher = &doc["jobs"]["publish"];
        assert_eq!(publisher["needs"], "candidates");
        assert_eq!(publisher["environment"], "release");
        assert_eq!(publisher["permissions"]["contents"], "read");
        assert_eq!(publisher["permissions"]["id-token"], "write");
        assert_eq!(publisher["permissions"]["packages"], "write");
        let publisher_text = serde_yaml::to_string(publisher).unwrap();
        assert!(publisher_text.contains("secrets.GITHUB_TOKEN"));
        assert!(publisher_text.contains("artifact push"));
        assert!(publisher_text.contains("oci://ghcr.io/owner/plugin:1.2.3-$cell"));
        assert!(publisher_text.contains("actions/download-artifact@"));
        // The registry credential is step-scoped, never job-wide.
        assert!(publisher["env"]["OST_REGISTRY_PASSWORD"].is_null());
        let publish_step = publisher["steps"]
            .as_sequence()
            .unwrap()
            .iter()
            .find(|step| step["name"] == "Re-verify and publish candidates")
            .expect("publish step");
        assert!(publish_step["env"]["OST_REGISTRY_PASSWORD"]
            .as_str()
            .unwrap()
            .contains("secrets.GITHUB_TOKEN"));

        let workflows = generate_github(&matrix);
        assert_eq!(workflows.last().unwrap().path, RELEASE_WORKFLOW_PATH);
    }

    #[test]
    fn draft_release_renders_candidates_without_a_publisher_or_secrets() {
        let mut matrix = release_matrix();
        matrix.release.as_mut().unwrap().mode = ReleaseMode::Draft;
        matrix.validate().unwrap();

        let yaml = generate_release(&matrix).expect("release workflow");
        let doc: serde_yaml::Value = serde_yaml::from_str(&yaml).expect("valid YAML");
        assert!(doc["jobs"]["candidates"].is_mapping());
        assert!(doc["jobs"]["publish"].is_null());
        assert!(!yaml.contains("secrets."));
        assert!(!yaml.contains("artifact push"));
        assert!(!yaml.contains("id-token"));
    }
}
