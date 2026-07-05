// SPDX-License-Identifier: Apache-2.0
//! GitHub Actions workflow generation from a support matrix (Phase 5).
//!
//! Cells render by **lane** into up to two workflows:
//!
//! - **source CI** (`pull_request` / `main` lanes): checkout → materialize a
//!   digest-pinned runtime SDK artifact → build/test/package the bundle from
//!   source. Never publishes, never uses secrets (fork-PR safety).
//! - **support matrix** (`scheduled` / `workflow_dispatch` lanes): re-verify a
//!   pinned runtime×plugin artifact pair from the runner's local registry.
//!
//! Cells reference named runner profiles; the renderer maps a profile to
//! `runs-on` (`github-hosted.image` → the image, `self-hosted.labels` → the
//! label list) and emits a billing notice on GitHub-hosted jobs. Generation is
//! deterministic (same matrix in, same YAML out) and every third-party action
//! is pinned to a full commit SHA (SEC-004), reusing the SHAs this repository
//! itself pins.

use crate::matrix::{Lane, SupportCell, SupportMatrix};

/// Default path of the generated support-matrix workflow.
pub const WORKFLOW_PATH: &str = ".github/workflows/ost-support-matrix.yml";

/// Default path of the generated source-CI workflow.
pub const SOURCE_WORKFLOW_PATH: &str = ".github/workflows/ost-source-ci.yml";

/// `actions/checkout`, pinned (SEC-004). Matches ci.yml.
const CHECKOUT: &str = "actions/checkout@9c091bb21b7c1c1d1991bb908d89e4e9dddfe3e0 # v7.0.0";

/// `actions/upload-artifact`, pinned (SEC-004). Matches release.yml.
const UPLOAD_ARTIFACT: &str =
    "actions/upload-artifact@043fb46d1a93c77aae656e7c1c64a875d1fc6a0a # v7";

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

/// Render the matrix into its workflows (support first, then source CI).
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
         \x20     OST_CI_RUNTIME_ARTIFACT: ${{ matrix.runtime_artifact }}",
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
\x20     - name: Check the pinned artifacts are in the local registry
        run: |
          ost artifact show ${{{{ matrix.runtime_artifact }}}}
          ost artifact show ${{{{ matrix.plugin_artifact }}}}
      - name: Verify artifact integrity
        run: |
          ost artifact verify ${{{{ matrix.runtime_artifact }}}}
          ost artifact verify ${{{{ matrix.plugin_artifact }}}}
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

/// The shared step list of a source-CI job.
fn source_steps() -> String {
    format!(
        "\
\x20   steps:
      - name: Check out the repository
        uses: {CHECKOUT}
{BILLING_NOTICE}\
\x20     - name: Check ost is available
        shell: bash
        run: ost --version
      - name: Validate the CI manifest
        shell: bash
        run: ost ci validate
      - name: Verify and materialize the pinned runtime SDK
        shell: bash
        run: |
          ost artifact verify ${{{{ matrix.runtime_artifact }}}}
          ost runtime pull ${{{{ matrix.platform }}}} --profile ${{{{ matrix.profile }}}} --from-artifact ${{{{ matrix.runtime_artifact }}}} --force
      - name: Build the plugin from source
        shell: bash
        run: ost plugin build ${{{{ matrix.bundle }}}} --target ${{{{ matrix.platform }}}} --profile ${{{{ matrix.profile }}}}
      - name: Run the verification pyramid
        shell: bash
        run: ost plugin test ${{{{ matrix.bundle }}}} --target ${{{{ matrix.platform }}}} --profile ${{{{ matrix.profile }}}} --up-to ${{{{ matrix.up_to }}}} --json
      - name: Package the plugin (never published from this workflow)
        shell: bash
        run: ost plugin package ${{{{ matrix.bundle }}}} --target ${{{{ matrix.platform }}}} --profile ${{{{ matrix.profile }}}}
      - name: Upload the verification report
        if: always()
        uses: {UPLOAD_ARTIFACT}
        with:
          name: report-${{{{ matrix.name }}}}
          path: ${{{{ matrix.bundle }}}}/.strata/reports/
"
    )
}

/// One source-CI job (`pr` or `mainline`) over the given cells.
fn source_job(matrix: &SupportMatrix, id: &str, event: &str, cells: &[&SupportCell]) -> String {
    let mut include = String::new();
    for cell in cells {
        let bundle = cell.bundle.as_deref().unwrap_or(".");
        include.push_str(&include_entry(
            matrix,
            cell,
            &format!("            bundle: {bundle}\n"),
        ));
    }
    format!(
        "\
\x20 {id}:
    if: github.event_name == '{event}'
    name: ${{{{ matrix.name }}}}
    runs-on: ${{{{ matrix.runs_on }}}}
{env}
    strategy:
      fail-fast: false
      matrix:
        include:
{include}\
{steps}",
        env = ci_env(false),
        steps = source_steps(),
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
# Source CI: each job checks out the repo, materializes a digest-pinned
# runtime SDK artifact from the runner's local registry (OST_HOME), and
# builds/tests/packages the bundle from source. `ost` must be on PATH and
# the registry must hold the pinned runtime artifact.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::matrix::{
        HostOs, HostSpec, Lane, Publish, RunnerKind, RunnerProfile, SupportCell, MATRIX_SCHEMA,
    };
    use std::collections::BTreeMap;

    fn cell(name: &str) -> SupportCell {
        SupportCell {
            name: name.into(),
            lane: Lane::default(),
            runner: None,
            runtime_artifact: format!("sha256:{}", "ab".repeat(32)),
            plugin_artifact: Some(format!("sha256:{}", "cd".repeat(32))),
            bundle: None,
            platform: "cy2026".into(),
            profile: "usd".into(),
            up_to: 5,
            publish: Publish::default(),
            host: HostSpec::default(),
        }
    }

    fn matrix() -> SupportMatrix {
        SupportMatrix {
            schema: MATRIX_SCHEMA,
            runners: BTreeMap::new(),
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
            runners,
            cells: vec![
                SupportCell {
                    lane: Lane::PullRequest,
                    runner: Some("windows-hosted".into()),
                    plugin_artifact: None,
                    bundle: Some("plugins/toy".into()),
                    up_to: 4,
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
}
