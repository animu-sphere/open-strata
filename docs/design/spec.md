\
# OpenStrata / `ost` — 設計・実装方針

> **OpenStrata** は、VFX Reference Platform の年次互換性を、実行・検証・配布可能な不変ランタイムレイヤーへ変換するための、VFX / OpenUSD 向け runtime・build・extension・validation プラットフォームである。
> CLI 名は **`ost`** とする。

---

## 0. このドキュメントの目的

この文書は、OpenStrata の初期実装を開始するための設計方針である。

実装の第一目標は、既存 DCC のランチャーや互換性レイヤーではない。以下を優先する。

1. VFX Reference Platform 年次仕様を機械可読な runtime target として扱う。
2. OpenUSD / MaterialX など、VFX Platform に隣接する重要な依存を controlled extension として統合する。
3. CMake / Ninja による CY 別・OS 別・Python ABI 別のビルド出し分けを行う。
4. USD plugin を build / package / discover / validate / publish できる。
5. `uv` を Python app dependency の第一級 backend として統合する。
6. Jenkins などの CI で build matrix を並列実行できる。
7. host OS / GPU driver / accelerator capability を検出・検証・記録できる。
8. runtime・extension・session を lockfile と digest で再現可能にする。

---

# 1. Product Identity

## 1.1 名称

- Product: **OpenStrata**
- CLI: **`ost`**
- Rust crate / package prefix: `openstrata-*` または `ost-*`
- CLI binary: `ost`

## 1.2 Tagline

> **OpenStrata turns VFX compatibility into executable, validated, distributable runtime layers.**

日本語:

> **OpenStrata は、VFX の互換性を実行・検証・配布可能なランタイムレイヤーに変える。**

## 1.3 コアコンセプト

```text
VFX Reference Platform
  -> machine-readable platform manifest
  -> immutable runtime artifact
  -> controlled extensions
  -> validated capability graph
  -> reproducible build / extension / session
```

OpenStrata は DCC 中心ではなく runtime 中心である。

```text
Old:
  DCC
    -> plugins
    -> studio tools
    -> show config

OpenStrata:
  Certified Runtime
    -> capabilities
    -> extensions
    -> task-specific apps
    -> sessions / sandboxes
```

---

# 2. Scope

## 2.1 初期スコープに含める

- VFX Reference Platform CY manifest の取り込み・表示・比較
- OpenStrata runtime target 解決
- CMake + Ninja build orchestration
- CMake toolchain file / CMake Presets 生成
- OpenUSD を controlled extension として導入
- MaterialX を OpenUSD feature / lookdev capability の依存として導入
- USD file format plugin を中心とした plugin lifecycle
- `PXR_PLUGINPATH_NAME` を含む runtime environment 生成
- `uv` integration
- Python / C++ validation
- artifact packaging
- Jenkins 向け CI / matrix build integration
- Linux x86_64 を最初の primary target とする
- host GPU / driver capability の detect / validate / record

## 2.2 初期スコープから外す

- Maya / Houdini / Nuke / Katana など既存 DCC の **API 直接統合**（独自 node/GUI
  API での抽象化）。ただし既存 DCC を *third-party external host* として
  discover / headless 実行 / package / cross-DCC USD 互換検証する支援は将来
  フェーズで扱う（runtime ネイティブ app が第一級、DCC は副次）。→
  [dcc-hosts.md](proposed/dcc-hosts.md)
- 商用 DCC plugin 配布
- GPU driver のインストール・アップデート
- Kubernetes / render farm の完全管理
- full GUI desktop manager
- marketplace
- 完全な filesystem snapshot sandbox
- remote build service の自前実装
- 独自 OCI registry のフル実装

これらは将来の拡張対象だが、MVP の前提にしない。

---

# 3. Design Principles

## 3.1 Strata 本体は軽く、runtime は重くてよい

```text
ost CLI
  - single binary
  - no Python runtime dependency
  - no USD dependency
  - no MaterialX dependency
  - no VFX library dependency

Strata Runtime
  - Python
  - C++ ABI / compiler
  - OpenUSD
  - MaterialX
  - OpenEXR / OCIO / OpenVDB / etc.
  - extensions / plugins
```

## 3.2 Workflow is portable; artifacts are explicit

ユーザー操作は OS をできるだけ意識しない。

```bash
ost runtime pull cy2026 --profile usd
ost devshell cy2026 --profile lookdev
ost build --target cy2026
```

ただし lockfile / diagnostics には、必ず artifact variant を記録する。

```text
linux-x86_64-glibc228-py313
macos-arm64-py313
windows-x86_64-msvc143-py313
```

## 3.3 Controlled extensibility, not arbitrary dependency freedom

VFX Reference Platform core は厳密に扱う。
拡張は、VFX Platform 周辺の正当な solution dependency に限定する。

```text
Tier 0: VFX Reference Platform Core
Tier 1: Strata Certified Extensions
Tier 2: App-local dependencies managed by uv
```

## 3.4 Runtime contract と validation を第一級にする

「インストールできた」だけでは十分ではない。

```text
runtime artifact
  + feature set
  + capability graph
  + validation result
  + digest
  = certified runtime
```

## 3.5 Package name ではなく capability から解決する

悪い例:

```yaml
dependencies:
  - materialx
  - some-usd-plugin
```

良い例:

```yaml
capabilities:
  - usd-lookdev
  - usd-fileformat:scache
```

resolver が capability から package / extension / environment / validation を導出する。

---

# 4. Core Domain Model

## 4.1 Platform

VFX Reference Platform calendar year を取り込んだ machine-readable definition。

```yaml
id: cy2026
source:
  kind: vfx-reference-platform
  status: final

core:
  python: "3.13.x"
  qt: "6.8.x"
  boost: "1.88.x"
  openexr: "3.4.x"
  ocio: "2.5.x"
  openvdb: "13.x"
  opensubdiv: "3.7.x"
  tbb: "2022.x"
  cxx_standard: "20"
  gcc: "14.2"
  glibc: "2.28"
```

### 必須コマンド

```bash
ost platform list
ost platform show cy2026
ost platform diff cy2025 cy2026
```

## 4.2 Runtime

Platform + OS/arch + Python ABI + profile + resolved artifacts の実体。

```yaml
id: openstrata-cy2026-linux-x86_64-py313-usd
platform: cy2026
variant:
  os: linux
  arch: x86_64
  libc: glibc
  libc_version: "2.28"
  python_abi: cpython-313-x86_64-linux-gnu
profile: usd
digest: sha256:...
validation: passed
```

## 4.3 Profile

よく使う capability 群を runtime layer として表す。

```text
core
dev
usd
lookdev
image
review
render
sim
ai
```

例:

```yaml
id: lookdev
requires:
  capabilities:
    - usd-stage-read
    - usd-shading
    - usd-materialx
    - ocio-display
    - hydra-preview
```

## 4.4 Extension

OpenStrata が controlled に管理する VFX-adjacent component。

例:

- OpenUSD
- MaterialX
- OpenImageIO
- OpenTimelineIO
- OpenAssetIO
- USD file format plugin
- USD schema plugin
- USD resolver plugin
- Hydra render delegate
- shader discovery plugin
- validation plugin

```yaml
id: materialx
type: library.materialx
tier: strata-certified-extension
version: "1.39.x"
provides:
  - materialx-document-io
  - materialx-shadergen
  - materialx-usd-interop
validation:
  - materialx-import
  - shadergen-smoke
```

## 4.5 Capability

アプリ・profile・extension が要求 / 提供する論理機能。

初期 capability 例:

```text
python-tooling
qt-ui
image-io
color-management
usd-stage-read
usd-stage-write
usd-shading
usd-materialx
hydra-preview
usd-fileformat:<extension>
usd-schema:<schema>
usd-asset-resolver
headless-execution
gpu-compute
```

## 4.6 Target

ビルド / package の出力先。

```text
platform year
+ OS
+ architecture
+ Python ABI
+ profile
+ OpenUSD version/features
```

例:

```text
cy2026-linux-x86_64-py313-usd
cy2026-linux-x86_64-py313-lookdev
cy2026-macos-arm64-py313-usd
```

## 4.7 Session

immutable runtime の上に作られる mutable workspace。

```text
runtime: read-only
project config: versioned
session workspace: writable
published artifact: immutable
```

初期は Git / filesystem workspace ベースでよい。

```bash
ost session start
ost session fork try-materialx-update
ost session diff
ost session discard
ost session promote
```

---

# 5. OpenUSD Strategy

## 5.1 OpenUSD は VFX Reference Platform Core ではない

VFX Reference Platform には OpenUSD が直接含まれない。
そのため OpenStrata では OpenUSD を **Strata Certified Extension** として扱う。

```text
VFX Reference Platform CY
  + OpenStrata extension: OpenUSD
  + OpenStrata extension: MaterialX
  = USD / lookdev capable runtime
```

## 5.2 OpenUSD は feature set 付きで扱う

OpenUSD は version だけでなく feature set が重要。

```text
core
python
imaging
usd-imaging
alembic
materialx
usdview
tests
```

例:

```yaml
id: openusd
type: solution.openusd
version: "25.05.01"
features:
  core: {}
  python:
    requires: [python, boost]
  imaging:
    requires: [openexr, opensubdiv]
  materialx:
    requires: [materialx]
  alembic:
    requires: [alembic]
```

## 5.3 Compatible range と certified build point を分ける

```text
compatibility range
  = dependency constraints 上で候補になりうる範囲

certified build point
  = 実際に build + validate 済みの artifact
```

例:

```yaml
openusd:
  allowed_range: ">=25.05,<26.08"
  certified:
    - version: "25.05.01"
      features: [core, python, imaging, materialx, alembic]
      validation: passed
```

## 5.4 OpenUSD version policy

```text
pinned
  指定 version を使う

certified
  当該 runtime における latest certified version を使う

range
  指定 range 内の certified artifact を解決する
```

例:

```yaml
requires:
  openusd: "certified"
```

または:

```yaml
requires:
  openusd: ">=25.05,<26"
```

---

# 6. MaterialX Strategy

## 6.1 MaterialX は controlled extension

MaterialX は VFX Reference Platform 本体には含まれない。
ただし OpenUSD shading / UsdMtlx / lookdev workflow に必要なため、Tier 1 extension として扱う。

## 6.2 MaterialX は OpenUSD feature dependency から導出する

```text
OpenUSD core
  -> MaterialX は必須ではない

OpenUSD[materialx]
  -> MaterialX required

capability: usd-materialx
  -> OpenUSD[materialx]
  -> MaterialX
  -> UsdMtlx discovery / plugin validation
```

## 6.3 Lookdev profile

```yaml
id: lookdev
requires:
  capabilities:
    - usd-stage-read
    - usd-shading
    - usd-materialx
    - ocio-display
    - hydra-preview
```

---

# 7. USD Plugin Lifecycle

## 7.1 USD plugin は Strata Extension として扱う

自作 plugin は単なる `.so` / `.dylib` / `.dll` ではない。

```text
USD plugin
  = Strata Extension
  = capability provider
  = build metadata + runtime metadata + validation + artifact identity
```

## 7.2 初期対応 plugin type

```text
usd.fileformat
usd.schema
usd.resolver
usd.hydra.delegate
usd.shader.discovery
usd.validation
usd.python
```

最初の優先順位:

1. `usd.fileformat`
2. `usd.schema`
3. `usd.resolver`

## 7.3 File format plugin manifest

```yaml
id: studio-cache-fileformat
type: usd.fileformat
version: 0.1.0

provides:
  - usd-fileformat:scache

requires:
  capabilities:
    - usd-stage-read
  packages:
    - openusd

compatibility:
  runtimes:
    - cy2025
    - cy2026

usd:
  plugin:
    resources:
      - resources
    file_formats:
      - extension: scache
        read: true
        write: false
        edit: false

build:
  system: cmake
  generator: ninja

validation:
  fixtures:
    - tests/fixtures/cube.scache
  tests:
    - plugin-discovery
    - fileformat-registered
    - open-layer
    - open-stage
```

## 7.4 Plugin CLI

```bash
ost plugin new usd-fileformat studio-cache --extension scache
ost plugin build --target cy2026
ost plugin validate --target cy2026
ost plugin package --target cy2026
ost plugin publish
```

## 7.5 Plugin environment

OpenStrata が以下を生成する。

```text
PXR_PLUGINPATH_NAME
PATH
LD_LIBRARY_PATH / DYLD_LIBRARY_PATH / PATH
PYTHONPATH
CMAKE_PREFIX_PATH
```

アプリやユーザーが plugin path を手で管理しなくてよい状態を目指す。

## 7.6 Plugin validation

最低限の検証:

```text
1. plugInfo.json が存在する
2. library が存在しロード可能
3. plugin registry が plugin を発見できる
4. target file extension / schema / resolver が登録される
5. fixture を開ける
6. Usd.Stage.Open が成功する
7. expected prim / schema / metadata が確認できる
```

### Python smoke test の概念例

```python
from pxr import Sdf, Usd

fmt = Sdf.FileFormat.FindByExtension("scache")
assert fmt is not None
assert fmt.SupportsReading()

layer = Sdf.Layer.FindOrOpen("tests/fixtures/cube.scache")
assert layer is not None

stage = Usd.Stage.Open(layer)
assert stage is not None
```

## 7.7 Diagnostics

```bash
ost doctor usd
ost doctor usd --extension studio-cache-fileformat
```

必ず以下を表示する。

```text
runtime identity
OpenUSD version/features
PXR_PLUGINPATH_NAME
discovered plugins
file formats
schema plugins
resolver plugins
shared library diagnostics
validation outcome
conflicts
```

## 7.8 Conflict detection

検出すべき衝突:

```text
same file extension provider
same schema typeName
multiple global resolver providers
OpenUSD ABI mismatch
Python ABI mismatch
plugin metadata conflict
library dependency collision
```

例:

```text
Conflict:
  ".abc" is already provided by usdAbc.
  Requested provider: studio-abc-fileformat.
```

---

# 8. Build System

## 8.1 CMake + Ninja を中核にする

OpenStrata は CMake を置き換えない。

```text
CMake
  = how to build

Ninja
  = how to execute build graph efficiently

OpenStrata
  = which target/runtime/dependencies/ABI/profile to build against
```

## 8.2 Build flow

```text
ost build --target cy2026
  -> resolve runtime
  -> pull / locate artifacts
  -> generate toolchain.cmake
  -> generate CMakePresets.json
  -> cmake configure
  -> ninja build
  -> cmake install
  -> validate
  -> package
```

## 8.3 Generated files

```text
.strata/
  targets/
    cy2026-linux-x86_64-py313/
      toolchain.cmake
      env.json
      validation.json
      target.lock.json
```

各 target の `CMakePresets.json` は `.strata/targets/<id>/` に生成される。
project root では既定でツール所有の `CMakeUserPresets.json` がそれらを
include し、利用者の `CMakePresets.json` には触れない。`ost presets install`
で利用者の `CMakePresets.json` へ非破壊的に取り込むこともできる。

## 8.4 Toolchain example

The compiler is chosen by policy (`host` by default, or `runtime`/`explicit`);
the runtime prefix is *prepended* to `CMAKE_PREFIX_PATH` so a project's own
prefixes survive. Example with the default `host` policy:

```cmake
# Compiler: host policy — CMake selects the host toolchain.

set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)

list(PREPEND CMAKE_PREFIX_PATH "/runtime")
set(Python_EXECUTABLE "/runtime/bin/python")
set(Python_ROOT_DIR "/runtime")

set(OpenUSD_ROOT "/runtime")
set(MaterialX_ROOT "/runtime")
```

With `--compiler runtime` the toolchain instead pins the runtime's bundled
`bin/gcc`/`bin/g++` (Linux) or `bin/clang`/`bin/clang++` (macOS); with
`--cc`/`--cxx` it pins the given absolute paths. The resolved compiler (policy,
paths, and `--version`) is recorded in `target.lock.json`, and changing it
invalidates the target's CMake build tree.

## 8.5 Essential CLI

```bash
ost configure --target cy2026
ost build --target cy2026
ost build --targets cy2025,cy2026
ost build --target cy2026 --jobs auto
ost validate --target cy2026
ost package --target cy2026
```

## 8.6 Build matrix

One source tree, many immutable artifacts.

```text
1 source tree
  -> N platform years
  -> N OS/arch variants
  -> N Python ABI variants
  -> N OpenUSD feature/version configurations
  -> 1 logical extension identity
  -> many physical artifacts
```

Example:

```text
dist/
  studio-cache-fileformat/
    0.1.0/
      cy2025-linux-x86_64-py311/
      cy2026-linux-x86_64-py313/
      cy2026-macos-arm64-py313/
```

---

# 9. Python and uv

## 9.1 Responsibility split

```text
OpenStrata:
  certified Python interpreter
  Python ABI
  VFX native bindings
  runtime identity
  core C++ libraries

uv:
  pyproject.toml
  uv.lock
  app-local pure Python dependencies
  venv
  Python tools
  fast dependency sync/run
```

## 9.2 Important rule

```text
1 runtime = 1 Python ABI
1 application process = 1 Python ABI
multiple Python versions = multiple runtime variants or isolated subprocesses
```

## 9.3 Use runtime Python explicitly

OpenStrata must select Python first; uv must not silently replace it.

Concept:

```bash
UV_PYTHON=/runtime/bin/python uv sync --locked
```

## 9.4 Two lockfiles

```text
uv.lock
  Python package dependency resolution

strata.lock
  runtime digest
  platform variant
  extension artifacts
  OpenUSD features
  Python ABI
  uv.lock hash
  validation status
```

Example:

```json
{
  "runtime": {
    "id": "openstrata-cy2026-linux-x86_64-py313-usd",
    "digest": "sha256:..."
  },
  "python": {
    "version": "3.13.x",
    "abi": "cpython-313-x86_64-linux-gnu",
    "manager": "uv",
    "uv_lock_hash": "sha256:..."
  }
}
```

## 9.5 Dependency policy

```text
Allowed through uv:
  pure Python packages
  app-local CLI tools
  test / lint / dev tooling

Allowed with validation:
  native wheels

Must come from Strata runtime:
  USD Python bindings
  MaterialX bindings when runtime-provided
  PySide / Qt
  OpenEXR / OCIO / OIIO / OpenVDB bindings
  core ABI-sensitive packages
```

---

# 10. Artifact Strategy

## 10.1 Initial format

MVP:

```text
tar.zst
+ manifest JSON
+ checksums
+ validation report
```

Later:

```text
OCI layout
OCI registry
oras-compatible push/pull
```

## 10.2 Artifact contents

```text
manifest
libraries
resources / plugInfo.json
Python package files if applicable
fixtures
validation metadata
checksums
provenance
```

## 10.3 Content-addressed identity

All runtime and extension artifacts must have digest identity.

```text
sha256:...
or
blake3:...
```

## 10.4 Local storage

Initial layout:

```text
~/.ost/
  config.toml
  runtimes/
  extensions/
  artifacts/
  cache/
  sessions/
  logs/
```

Use a content-addressed store.

SQLite is optional initially; JSON indexes are acceptable for MVP.
Introduce SQLite when search, cache eviction, install history, and validation history become complex.

---

# 11. OS and Platform Strategy

## 11.1 Initial target order

1. Linux x86_64
2. macOS arm64 devshell / native runtime
3. Windows x86_64 devshell
4. stronger cross-platform app execution
5. stronger cross-platform sandbox

## 11.2 Adapter responsibility

```text
Linux:
  PATH
  LD_LIBRARY_PATH
  XDG paths
  optional namespace / overlayfs later

macOS:
  PATH
  DYLD-related diagnostics
  @rpath diagnostics
  app bundle / codesign later

Windows:
  PATH
  DLL path
  PowerShell activation
  MSVC runtime detection
```

## 11.3 Do not fake identical sandbox semantics

Initial policy:

```text
Linux:
  can gain strong isolation later

macOS / Windows:
  workspace/session based isolation first
```

Never claim identical sandbox capability if the host platform differs.

---

# 12. GPU and AI Expansion

## 12.1 Positioning

GPU drivers are **host capabilities**, not normal runtime dependencies.

```text
Runtime:
  CUDA toolkit
  cuDNN
  PyTorch
  ONNX Runtime
  TensorRT

Host:
  GPU vendor/model
  driver version
  CUDA driver API
  GPU architecture
  VRAM
  kernel / container hooks
```

## 12.2 OpenStrata responsibilities

```text
Detect: yes
Require: yes
Validate: yes
Record in lock/report: yes
Route CI job to capable agent: yes

Install / update GPU driver: no
Manage kernel module: no
Manage Secure Boot: no
Manage MIG: no
Operate Kubernetes GPU stack: no
```

## 12.3 GPU manifest example

```yaml
requires_host:
  gpu:
    vendor: nvidia
    min_driver: "550.54"
    cuda_driver_api: ">=12.4"
    architectures:
      - sm_89
      - sm_90
```

## 12.4 Commands

```bash
ost doctor gpu
ost ai doctor
ost ai validate --profile torch-cuda
```

## 12.5 AI profiles

There is no single AI equivalent of VFX Reference Platform.
Treat AI as OpenStrata Certified Profiles.

```text
ai-cuda124
ai-cuda126
ai-rocm
ai-mps
vfx-ai-lookdev
```

Potential hybrid profile:

```text
cy2026-lookdev-ai
  OpenUSD
  MaterialX
  OCIO
  OpenEXR
  PyTorch
  ONNX Runtime
  CUDA / TensorRT
```

---

# 13. CI and Jenkins

## 13.1 Responsibility split

```text
Jenkins:
  parallel orchestration
  agent scheduling
  credentials
  logs
  JUnit report display
  artifact archive

OpenStrata:
  target resolution
  runtime pull
  CMake preset/toolchain generation
  build
  validation
  package
  publish
```

> Jenkins is orchestration. OpenStrata is build truth.

## 13.2 CI-safe command requirements

All CI commands must support:

```text
--ci
--no-interactive
--report junit
--report json
--jobs auto
deterministic exit codes
machine-readable output
```

> 具体的な出力形は §14.3、安定 error code と category 別 exit code の対応は §14.4 を参照。
> `--json` は `--report json` のグローバル別名として扱う。

## 13.3 Jenkins matrix axes

```text
platform year
OS / arch
profile
OpenUSD version
Python ABI
GPU capability
```

## 13.4 Jenkins example

```groovy
pipeline {
  agent none

  options {
    timestamps()
    parallelsAlwaysFailFast()
  }

  stages {
    stage('OpenStrata Matrix') {
      matrix {
        axes {
          axis {
            name 'OST_TARGET'
            values 'cy2025', 'cy2026'
          }
          axis {
            name 'OST_PROFILE'
            values 'usd', 'lookdev'
          }
        }

        agent {
          label 'linux && x86_64 && openstrata'
        }

        stages {
          stage('Resolve') {
            steps {
              sh 'ost runtime pull ${OST_TARGET} --profile ${OST_PROFILE}'
              sh 'ost runtime explain ${OST_TARGET} --profile ${OST_PROFILE}'
            }
          }

          stage('Build') {
            steps {
              sh 'ost build --target ${OST_TARGET} --profile ${OST_PROFILE} --ci --jobs auto'
            }
          }

          stage('Validate') {
            steps {
              sh 'ost validate --target ${OST_TARGET} --profile ${OST_PROFILE} --report junit --report json'
              junit '.ost/reports/**/*.xml'
            }
          }

          stage('Package') {
            steps {
              sh 'ost package --target ${OST_TARGET} --profile ${OST_PROFILE}'
              archiveArtifacts artifacts: 'dist/**/*', fingerprint: true
            }
          }
        }
      }
    }

    stage('Publish') {
      when {
        branch 'main'
      }
      agent {
        label 'linux && openstrata'
      }
      steps {
        sh 'ost publish --all-targets'
      }
    }
  }
}
```

## 13.5 Two levels of parallelism

```text
Jenkins-level:
  distribute CY / OS / profile / OpenUSD version / GPU profile

Build-level:
  Ninja parallel jobs inside each build agent
```

## 13.6 Cache directories

```text
~/.ost/runtimes
~/.ost/artifacts
~/.cache/uv
~/.cache/sccache
~/.ccache
```

The CI agent image should mount persistent cache volumes.

---

# 14. CLI Design

## 14.1 Top-level commands

```text
ost platform
ost runtime
ost devshell
ost env
ost init
ost app
ost extension
ost plugin
ost configure
ost build
ost validate
ost package
ost publish
ost session
ost doctor
ost cache
ost ci
ost ai
ost self
```

## 14.2 Essential MVP commands

```bash
ost platform list
ost platform show cy2026
ost platform diff cy2025 cy2026

ost runtime list
ost runtime pull cy2026 --profile usd
ost runtime show cy2026
ost runtime explain cy2026 --profile lookdev
ost runtime validate cy2026

ost devshell cy2026 --profile usd
ost env cy2026 --profile usd --shell bash

ost init
ost configure --target cy2026
ost build --target cy2026
ost validate --target cy2026
ost package --target cy2026

ost plugin new usd-fileformat toy-cache --extension toy
ost plugin build --target cy2026
ost plugin validate --target cy2026

ost extension add materialx
ost extension why materialx

ost doctor
ost doctor usd
ost doctor gpu

ost ci init jenkins
ost ci generate jenkins
```

## 14.3 Output contract

機械可読出力は表示形式ではなく **安定した契約** として扱う（§13.2 / §18.3）。
利用者は人間・CI に加えてコーディングエージェントも想定するが、いずれも同じ契約で足りる。

原則:

- 既定は human 向け。`--json`（= `--report json` のグローバル別名）で機械可読出力。
- `--json` 時、結果は stdout に **単一の JSON ドキュメントのみ**。進捗・警告・デバッグは stderr。
  色・スピナー・バナーを stdout に混ぜない（パイプを壊さない）。
- 成功・失敗・no-op いずれも安定した最上位の形を返す。将来の互換のため版を持つ。
- 出力構築は `ost-cli` の output レイヤに集約し、各コマンドが ad-hoc に組み立てない。
- JSON 内のパスは原則絶対パスへ解決する。秘密情報（token 等）は JSON・ログに出さない。

最小エンベロープ:

```text
ok         bool         成功 true / 失敗 false
schema     integer      出力契約の版（初期値 1）
data       object       成功時に必須
error      object       失敗時に必須（§14.4）
warnings   array        無ければ空配列
```

> `status` のような新しい集約コマンドは設けない。プロジェクトと実行可能性の診断は
> `ost doctor` に集約する（§14.5）。task / logs / config レジストリ / build の plan-run
> 分割は本フェーズの対象外とする。

## 14.4 Error codes and exit codes

§13.2 の "deterministic exit codes / machine-readable output" を具体化する。
エラーは文言ではなく **安定した `code` と `category`** で判定できること。
`ost-core::Error` が code と category を導出できる形を持ち、`ost-cli` 境界で正規化する。
子プロセスの生の exit code は error 内（`external_exit_code` 等）に保持する。

category:

```text
usage          引数・使用方法の誤り
configuration  manifest / lock / 設定の不正
precondition   runtime・ツール・ディレクトリ等の前提不足
validation     検証不一致
external_tool  CMake / Ninja / compiler / OpenUSD の失敗
io             ファイルシステム・権限
internal       想定外の内部エラー
```

exit code（category から正規化）:

```text
0   success / no-op
2   usage
3   configuration
4   precondition
5   validation
6   external_tool
7   io
70  internal
```

代表的な code（初期セット。追加は後方互換に行う。実装済みの一覧と各 category の
対応は docs/json-schema.md を正とする）:

```text
INVALID_ARGUMENT  PLATFORM_NOT_FOUND  PROJECT_EXISTS
INVALID_CONFIG    MANIFEST_INVALID    PARSE_FAILED
PROJECT_NOT_FOUND RUNTIME_NOT_FOUND   REQUIRED_TOOL_MISSING
REAL_RUNTIME_REQUIRED  PRECONDITION_FAILED
VALIDATION_FAILED  EXTERNAL_TOOL_FAILED  IO_ERROR
INTERNAL_ERROR
OPERATION_FAILED  # 移行用の総称コード（precondition 既定）
```

## 14.5 doctor as the diagnosis surface

`ost doctor` を「環境・前提・runtime 可否」を一括で返す唯一の診断面とする。
別途 `ost status` は設けない（§14.3）。

要件:

- host / tools / runtime / lock を機械可読（`--json` / `--report junit`）で返す。
- 最初の問題で早期 return せず、複数の問題をまとめて報告する。
- runtime は kind（`mock` / `adopted` / `built` / `downloaded`）と実行可否
  （static validation のみか、実 OpenUSD を実行できるか）を明示する。
  mock runtime で実行型テストを要求したら `REAL_RUNTIME_REQUIRED`。
- 各 issue に次アクション（実行すべきコマンド）を付す。
- 情報的な warning のみなら exit 0。required な前提（必須ツール欠落・runtime 未取得など）が
  満たされない場合は §14.4 の category に応じた失敗コードを返す。

---

# 15. Technical Stack

## 15.1 OpenStrata CLI and core

Use **Rust**.

Reasons:

```text
single binary deployment
cross-platform filesystem/process handling
robust state / lockfile / artifact graph modeling
fast hash/archive operations
does not depend on Python
good fit for CLI / runtime manager / resolver
```

Suggested crates:

```text
clap
serde
serde_json
serde_yaml / serde_yml
toml
anyhow
thiserror
tracing
tracing-subscriber
tokio
reqwest
camino
walkdir
ignore
tempfile
notify
sha2 or blake3
tar
zstd
zip
which
```

## 15.2 Build

```text
CMake
Ninja
CMake Presets
CMake toolchain files
ccache or sccache
```

## 15.3 Python

```text
Python supplied by OpenStrata runtime
uv for app-local dependency management
pytest for validation
```

## 15.4 Native VFX libraries

Managed runtime artifacts, not `ost` CLI dependencies:

```text
OpenUSD
MaterialX
OpenEXR
OpenColorIO
OpenImageIO
OpenVDB
Alembic
OpenSubdiv
Boost
oneTBB
Qt / PySide
```

## 15.5 Artifacts

```text
MVP:
  tar.zst + checksum + JSON manifest

Later:
  OCI layout
  OCI registry
  oras-compatible transport
```

## 15.6 UI

Do not build first.

Later candidates:

```text
management UI:
  Tauri + Svelte

VFX sample apps:
  Python + PySide6

IDE:
  VS Code extension
```

---

# 16. Repository Structure

```text
openstrata/
  Cargo.toml

  crates/
    ost-cli/
    ost-core/
    ost-manifest/
    ost-solver/
    ost-runtime/
    ost-build/
    ost-extension/
    ost-plugin/
    ost-session/
    ost-validation/
    ost-ci/
    ost-platform/

  python/
    openstrata_sdk/
      app/
      validation/
      usd/
      materialx/
      gpu/

  cmake/
    OpenStrata.cmake
    OpenStrataUSD.cmake
    OpenStrataPlugin.cmake

  schemas/
    platform.schema.json
    runtime.schema.json
    extension.schema.json
    project.schema.json
    lock.schema.json

  platforms/
    cy2025.yaml
    cy2026.yaml
    cy2027.yaml

  extensions/
    openusd.yaml
    materialx.yaml
    openimageio.yaml
    usd-fileformat-template.yaml

  profiles/
    core.yaml
    dev.yaml
    usd.yaml
    lookdev.yaml
    image.yaml
    ai.yaml

  templates/
    usd-fileformat-cpp/
    usd-schema-cpp/
    usd-resolver-cpp/
    python-cli/
    python-qt-exr-viewer/
    usd-inspector/

  validation/
    fixtures/
      usd/
      exr/
      mtlx/
      ocio/
    tests/

  ci/
    jenkins/
      Jenkinsfile.template
      shared-library/

  docker/
    ci/

  docs/
```

---

# 17. Deployment Strategy

## 17.1 CLI deployment

`ost` is a standalone Rust binary.

Initial supported installation channels:

```text
GitHub Releases
install.sh
install.ps1
Homebrew tap
cargo install
```

## 17.2 CLI vs runtime

```text
Install ost:
  small / fast

ost runtime pull:
  heavy managed artifacts
```

Example:

```bash
ost doctor
ost bootstrap
ost runtime pull cy2026 --profile usd
ost devshell cy2026 --profile usd
```

## 17.3 Local layout

```text
~/.ost/
  config.toml
  runtimes/
  extensions/
  artifacts/
  cache/
  sessions/
  logs/
```

## 17.4 Studio deployment

```text
/opt/openstrata/bin/ost
/studio/openstrata/runtimes/
/studio/openstrata/extensions/
/studio/openstrata/cache/
```

Use shared read-only runtime roots plus user-writable overlays.

## 17.5 Offline / air-gapped

Support:

```bash
ost runtime export
ost runtime import
ost extension export
ost extension import
ost registry mirror
```

---

# 18. Validation Requirements

## 18.1 Runtime validation

Minimum checks:

```text
Python launch/import
native shared library load
OpenEXR read/write
OCIO transform
OpenUSD stage open
MaterialX document read
UsdMtlx discovery
Hydra smoke test where profile requires it
CMake configure test
```

## 18.2 Extension validation

Minimum checks:

```text
artifact integrity
manifest schema validation
runtime compatibility
plugin discovery
capability provider registration
fixture test
ABI / Python ABI match
conflict check
```

## 18.3 Reports

Support:

```text
human-readable terminal report
JSON
JUnit XML
artifact manifest
provenance metadata
```

---

# 19. Implementation Phases

## Phase 0 — Foundation

Goal:

```text
Rust CLI skeleton
YAML platform manifests
JSON lock schema
platform list/show/diff
basic project initialization
```

Commands:

```bash
ost platform list
ost platform show cy2026
ost platform diff cy2025 cy2026
ost init
```

## Phase 1 — Runtime and devshell

Goal:

```text
runtime manifest resolution
local runtime layout
env generation
devshell
doctor
```

Commands:

```bash
ost runtime pull cy2026 --profile core
ost devshell cy2026
ost env cy2026 --shell bash
ost doctor
```

Initial backend can be local directory / tar.zst artifacts. OCI can follow.

## Phase 2 — CMake target build

Goal:

```text
target resolver
toolchain.cmake generation
CMakePresets generation
Ninja build wrapper
```

Commands:

```bash
ost configure --target cy2026
ost build --target cy2026
ost package --target cy2026
```

## Phase 3 — OpenUSD / MaterialX profiles

Goal:

```text
OpenUSD extension family
feature resolution
MaterialX dependency resolution
usd / lookdev profiles
```

Commands:

```bash
ost runtime pull cy2026 --profile usd
ost runtime explain cy2026 --profile lookdev
```

## Phase 4 — USD file format plugin lifecycle

Goal:

```text
plugin template
plugInfo handling
PXR_PLUGINPATH_NAME generation
discovery validation
fixture validation
doctor usd
```

Commands:

```bash
ost plugin new usd-fileformat toy-cache --extension toy
ost plugin build --target cy2026
ost plugin validate --target cy2026
ost doctor usd
```

## Phase 5 — CI / Jenkins

Goal:

```text
CI-safe commands
JUnit/JSON reports
Jenkinsfile template
matrix generation
artifact archive/publish integration
```

Commands:

```bash
ost ci init jenkins
ost ci generate jenkins
```

## Phase 6 — Artifact registry

Goal:

```text
artifact store
digest pinning
registry transport
OCI layout
publish/pull
```

## Phase 7 — Sessions / sandbox

Goal:

```text
session metadata
fork/diff/promote
workspace isolation
optional Linux namespace/overlayfs integration
```

## Phase 8 — AI / GPU profiles

Goal:

```text
GPU host detection
driver requirement checks
AI runtime profiles
Jenkins GPU routing labels
smoke tests
```

---

# 20. Non-Goals and Guardrails

## 20.1 Avoid creating a replacement for everything

OpenStrata must not become all of these at once:

```text
new package manager
new build system
new container runtime
new DCC
new render farm
new Kubernetes distribution
new GPU driver manager
new generic app marketplace
```

OpenStrata is an orchestration and certification layer that integrates proven primitives.

## 20.2 Reuse existing tools

```text
CMake / Ninja:
  build execution

uv:
  Python dependencies

Jenkins:
  CI orchestration

Docker / Podman / OCI:
  transport / isolation where useful

Git:
  source / workspace history
```

## 20.3 Never silently substitute core ABI-sensitive dependencies

Do not permit app-local `uv` dependencies to silently override:

```text
Python interpreter
PySide / Qt
OpenUSD bindings
MaterialX bindings
OpenEXR / OCIO / OIIO / OpenVDB bindings
```

Emit clear diagnostics and recommend the corresponding OpenStrata extension/profile.

---

# 21. Acceptance Criteria for the First Vertical Slice

The first meaningful vertical slice should demonstrate the following:

```bash
# 1. Inspect platform definition
ost platform show cy2026

# 2. Enter a certified USD development shell
ost runtime pull cy2026 --profile usd
ost devshell cy2026 --profile usd

# 3. Generate a USD file format plugin
ost plugin new usd-fileformat toy-cache --extension toy

# 4. Build for a specific CY target
ost plugin build --target cy2026

# 5. Validate discovery and opening a fixture
ost plugin validate --target cy2026

# 6. Diagnose the USD environment
ost doctor usd

# 7. Package output
ost plugin package --target cy2026
```

Success means:

1. `toy` extension is discovered by OpenUSD.
2. `Sdf.FileFormat.FindByExtension("toy")` succeeds.
3. A fixture `.toy` file can be opened as an `SdfLayer`.
4. A `UsdStage` can be opened from that layer.
5. Artifact manifest records target, runtime digest, OpenUSD features, and validation result.
6. The same source tree can be built for at least two target definitions, even if the first implementation supports Linux only.

---

# 22. Immediate Implementation Tasks

Implement in this order:

1. Create Rust workspace and `ost` CLI skeleton.
2. Add platform manifest schema and CY2025/CY2026 sample manifests.
3. Implement `ost platform list/show/diff`.
4. Add project manifest and lockfile schemas.
5. Implement `ost init`.
6. Implement runtime target model.
7. Implement `ost env` and `ost devshell` with a mock/local runtime.
8. Implement CMake toolchain + preset generation.
9. Implement `ost configure`, `ost build`, `ost validate`, `ost package`.
10. Add OpenUSD / MaterialX extension manifests.
11. Add `usd` profile and capability resolver.
12. Add USD fileformat plugin template.
13. Add `ost doctor usd`.
14. Add pytest-based fileformat validation.
15. Add Jenkinsfile template and JUnit output.
16. Add artifact digest and local store.
17. Add multi-target build matrix support.

---

# 23. Quality Bar

- CLI errors must be actionable.
- All generated manifests must be deterministic.
- Runtime and extension identities must always include version + target + digest.
- No hidden environment mutation outside `ost devshell` / `ost env`.
- Every extension must declare:
  - compatible runtime(s)
  - provided capabilities
  - dependency reason
  - validation requirements
- Every published artifact must include provenance and validation result.
- Build logic must remain in OpenStrata / CMake metadata, not be duplicated across Jenkinsfiles.
- OpenStrata must work without a preinstalled Python environment.
- Linux x86_64 is the first supported implementation target; other OS targets must be modeled from the beginning but can be unavailable initially.

---

# 24. Product Summary

```text
OpenStrata (`ost`)
  = VFX Reference Platform aware runtime manager
  + CMake target build system
  + OpenUSD / MaterialX controlled extension system
  + USD plugin lifecycle tooling
  + uv-integrated Python app environment
  + validation / diagnostics
  + Jenkins matrix build integration
  + reproducible artifact distribution
```

Core statement:

> **VFX Reference Platform provides the target constraints. OpenStrata turns them into certified runtime layers, extension artifacts, and reproducible builds.**
