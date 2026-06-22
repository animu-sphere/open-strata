# OpenStrata — OpenUSD Plugin Verification Harness 設計方針

> **Status:** Draft / Directional Design
>
> **Scope:** OpenStrata 初期プロダクトおよび OpenUSD プラグイン開発・検証体験

---

## 1. 概要

OpenStrata は、VFX Reference Platform と OpenUSD 周辺の互換性を、**開発・検証・配布可能なランタイムレイヤー**として扱うためのプラットフォームである。

初期段階では、OpenUSD プラグイン開発において最も摩擦が大きい領域――プラグインの発見、動的ライブラリ解決、ランタイム整合、`usdcat` / `usdview` による動作確認、診断、再現可能なテスト――を解消する。

OpenStrata は単なるパッケージマネージャや環境変数セットではない。プラグインとVFXネイティブツールを、固定・保証されたランタイム上で安全に起動し、検証し、将来は配布できるようにする**実行ハーネス**である。

### プロダクト定義

> OpenStrata is a reproducible runtime and verification harness for OpenUSD plugins and VFX-native tools.

日本語では次のように定義する。

> OpenStrata は、OpenUSD プラグインと VFX ネイティブツールを、再現可能な Runtime 上で開発・起動・検証・配布するための実行ハーネスである。

---

## 2. 背景と課題

OpenUSD プラグインの開発では、機能実装そのものよりも、実行環境の差異と不透明さが問題になりやすい。

主な課題は以下である。

- OpenUSD 本体とプラグイン間の ABI / C++ 標準ライブラリ整合性
- OpenUSD、Python、MaterialX、OCIO、OIIO などの依存バージョン差異
- `plugInfo.json` とプラグイン探索パスの構成
- 共有ライブラリおよびその依存ライブラリの解決
- `usdcat`、`usdview`、Python API、DCC ホストでの挙動差
- 利用者ごとに異なるシェル環境・環境変数・ローカル依存
- 失敗時に「プラグインが見つからない」「ライブラリが読めない」「ファイル形式として登録されない」などの原因を切り分けにくいこと
- CI とローカルで同じ検証を再現しにくいこと

特に、プラグインをビルドした後に、どの環境で、どのコマンドを使い、どのパスを設定して確認すべきかが属人的になりやすい。

OpenStrata はこの問題を、以下のように置き換える。

> 開発者が環境を手作業で組み立てるのではなく、OpenStrata が Runtime と Plugin Bundle を合成し、検証目的に応じた実行セッションを生成する。

---

## 3. 長期ビジョン

OpenStrata が将来的に目指すのは、既存DCCを唯一の中心に置かない VFX / AI 実行基盤である。

> 不変かつ保証された Runtime の上に、小さく目的特化した VFX / AI アプリケーションを自在に生やせる基盤。

この世界では、DCCは重要なホストではあるが、唯一の実行環境ではない。以下のようなツールが共通の Runtime Contract 上で動作する。

- USD アセット検証 CLI
- OpenUSD File Format Plugin
- Asset Resolver
- Schema Plugin
- Hydra Render Delegate
- MaterialX 変換ツール
- 専用オーサリングアプリケーション
- AI 推論ワーカー
- バッチ変換ジョブ
- Web / Wasm 向けの補助ランタイム
- DCC 向けブリッジプラグイン

初期の OpenUSD Plugin Verification Harness は、この構想の最小かつ重要な足がかりとする。

---

## 4. 初期プロダクトの目的

最初の OpenStrata は、以下の体験を成立させることに集中する。

```bash
# プラグインをビルド
ost plugin build usdluma

# Bundle と Runtime の整合性を確認
ost plugin inspect usdluma

# プラグインが OpenUSD に発見されるか診断
ost plugin doctor usdluma

# usdcat を、正しい Runtime + Plugin 構成で実行
ost plugin run usdluma -- usdcat fixtures/basic.lumagraph

# 正規化出力と golden file を比較
ost plugin verify usdluma fixtures/basic.lumagraph

# usdview を起動して目視確認
ost plugin view usdluma fixtures/basic.lumagraph

# 段階化された検証一式を実行
ost plugin test usdluma --json
```

利用者は `PXR_PLUGINPATH_NAME`、`PYTHONPATH`、`LD_LIBRARY_PATH`、`PATH` などを手作業で設定しない。

OpenStrata が、指定された Runtime と Plugin Bundle から一時的な実行セッションを構成し、必要な環境だけを子プロセスへ渡す。

---

## 5. 設計原則

### 5.1 検証体験を最優先する

初期の価値は「配布」よりも「すぐ正しく動くか確認できること」に置く。

最初に強くすべきコマンドは `ost plugin test` と `ost plugin doctor` である。

### 5.2 グローバル環境を汚さない

環境変数を利用者のログインシェルへ恒久設定する設計は避ける。

標準の実行方法は `ost run` とし、Runtime Session ごとに環境を合成する。

### 5.3 人間とコーディングエージェントの双方に最適化する

人間には読みやすい標準出力、エージェントには構造化 JSON レポートを提供する。

### 5.4 プラグインを自己記述的な Bundle として扱う

共有ライブラリ単体ではなく、Manifest、`plugInfo.json`、依存情報、テスト、fixture を含む Plugin Bundle を基本単位にする。

### 5.5 検証は段階化し、失敗地点を明確にする

`usdview` が起動しないという最終症状だけでなく、Bundle 構造、依存解決、Plugin Discovery、Stage Open などのどこで失敗したかを特定できるようにする。

### 5.6 DCC 統合は後段にする

まず Standalone OpenUSD Runtime 上で成功を再現可能にする。

DCC 対応は Host Adapter として後から積み上げ、DCC 固有の問題と Runtime / Plugin の問題を切り分ける。

---

## 6. 全体アーキテクチャ

```text
┌─────────────────────────────────────────────┐
│ Purpose-built Applications                   │
│ CLI / DCC Bridge / VFX Tool / AI Worker      │
├─────────────────────────────────────────────┤
│ OpenStrata Developer Interface               │
│ ost CLI / launcher / reports / generated shim│
├─────────────────────────────────────────────┤
│ OpenStrata Runtime Session                   │
│ Runtime + Plugin Bundle + Env + Diagnostics  │
├─────────────────────────────────────────────┤
│ Compatibility & Validation Layer             │
│ ABI / discovery / smoke / golden / matrix    │
├─────────────────────────────────────────────┤
│ Runtime & Artifact Distribution Layer        │
│ fixed runtimes / plugin bundles / cache      │
├─────────────────────────────────────────────┤
│ Foundation Stack                             │
│ OpenUSD / Python / MaterialX / OCIO / OIIO   │
└─────────────────────────────────────────────┘
```

OpenStrata の中心は、Runtime と Plugin Bundle を組み合わせて作る **Runtime Session** である。

```text
OpenUSD Runtime
+ Plugin Bundle
+ dependent plugin bundles
+ plugin discovery paths
+ library search paths
+ Python search paths
+ resolver / OCIO / MaterialX settings
+ diagnostics and report destination
= OpenStrata Runtime Session
```

---

## 7. Runtime の扱い

### 7.1 Runtime は固定された名前付き成果物として扱う

Runtime は「その時のローカル環境」ではなく、再現可能な識別子を持つ成果物として扱う。

例:

```text
openusd-24.11-linux-x86_64-clang18-libcxx
openusd-25.02-linux-x86_64-clang18-libcxx
```

Runtime Manifest の例:

```yaml
runtime:
  name: openusd-24.11-linux-x86_64-clang18-libcxx
  platform: linux-x86_64
  compiler: clang-18
  cxx_abi: libcxx
  python: "3.11"

components:
  openusd: "24.11"
  materialx: "1.39"
  ocio: "2.4"
  oiio: "2.5"
  opensubdiv: "3.6"

capabilities:
  - usd-file-format
  - usd-schema
  - usd-asset-resolver
  - usdview
  - hydra
```

### 7.2 Runtime が提供するべきもの

初期 Runtime は少なくとも以下を含む。

- OpenUSD CLI 群（`usdcat`, `usdview`, `usdchecker` など）
- OpenUSD Python bindings
- プラグインロードに必要な共有ライブラリ
- `usdview` 実行に必要な Python / Qt 周辺依存
- MaterialX、OCIO、OIIO など、対象 Runtime が保証する周辺依存
- Runtime Manifest と診断用メタデータ

### 7.3 最初のサポート対象

MVP では Linux x86_64 を主対象にする。

Windows / macOS は Runtime Contract と CLI を壊さない形で後から追加する。OS ごとの差異は Launcher / Session Builder 側に閉じ込める。

---

## 8. Plugin Bundle Contract

プラグインは共有ライブラリだけでなく、OpenStrata が検証・実行できる自己記述的 Bundle として構成する。

```text
usdluma/
├─ openstrata.plugin.yaml
├─ plugin/
│  ├─ lib/
│  │  └─ usdluma.so
│  └─ resources/
│     └─ usdluma/
│        └─ plugInfo.json
├─ python/
│  └─ usdluma_tools/
├─ tests/
│  ├─ fixtures/
│  │  ├─ basic.lumagraph
│  │  └─ invalid.lumagraph
│  ├─ expected/
│  │  └─ basic.usda
│  └─ test_plugin.py
└─ scripts/
   └─ post_install.py
```

### 8.1 Plugin Manifest 例

```yaml
plugin:
  name: usdluma
  version: 0.1.0
  kind: usd-file-format

runtime:
  openusd: ">=24.11,<25.0"
  platform:
    - linux-x86_64
  cxx_abi: libcxx

provides:
  file_formats:
    - luma
    - lumagraph

entrypoints:
  plug_info:
    - plugin/resources/usdluma/plugInfo.json

requires:
  capabilities:
    - usd-file-format
  components:
    materialx: ">=1.39,<1.40"

tests:
  smoke:
    - fixtures/basic.lumagraph
  roundtrip:
    - fixtures/basic.lumagraph
  negative:
    - fixtures/invalid.lumagraph
```

### 8.2 Bundle が担う責務

- プラグイン種別の宣言
- 対象 Runtime と ABI 条件の宣言
- `plugInfo.json` の位置の宣言
- 提供する File Format / Schema / Resolver などの能力の宣言
- 依存コンポーネントの宣言
- テスト fixture と期待出力の保持
- パッケージング時に必要なメタデータの保持

---

## 9. Runtime Session と Launcher

### 9.1 標準実行は `ost run`

```bash
ost run \
  --runtime openusd-24.11-linux-x86_64-clang18-libcxx \
  --plugin ./dist/usdluma \
  -- usdcat fixtures/basic.lumagraph
```

`ost run` は以下を行う。

1. Runtime を解決する
2. Plugin Bundle を検査する
3. Runtime と Bundle の互換性を確認する
4. 一時 Session Directory を作成する
5. Plugin Discovery Path を構成する
6. Library / Python / Asset 検索パスを構成する
7. 診断ログと実行レポートの出力先を準備する
8. 指定コマンドを子プロセスとして実行する
9. 実行結果と環境要約を保存する

### 9.2 Session が管理する環境

OS ごとの実装差異はあるが、概念上は以下を OpenStrata が管理する。

```text
PXR_PLUGINPATH_NAME
PYTHONPATH
PATH
LD_LIBRARY_PATH / DYLD_LIBRARY_PATH / PATH
USD_PLUGIN_PATH
MATERIALX_SEARCH_PATH
OCIO
```

これらはユーザーの恒久環境ではなく、Session に限定して設定する。

### 9.3 `ost shell` は補助機能とする

インタラクティブな調査のために `ost shell` は提供してよい。

```bash
ost shell --runtime openusd-24.11 --plugin usdluma
```

ただし、ドキュメント・CI・自動化の標準経路は `ost run` とする。シェル状態に依存した手順を正規フローにしない。

### 9.4 起動バッチは生成物として扱う

`.sh` / `.bat` / `.command` をソースオブトゥルースにしない。必要な場合のみ、Manifest から生成する。

```bash
ost launch-script generate \
  --runtime openusd-24.11 \
  --plugin usdluma \
  --format sh \
  --output build/launch-usdluma.sh
```

Windows 用:

```bash
ost launch-script generate \
  --runtime openusd-24.11 \
  --plugin usdluma \
  --format bat \
  --output build/launch-usdluma.bat
```

---

## 10. CLI 方針

### 10.1 Runtime 操作

```bash
ost runtime list
ost runtime install openusd-24.11-linux-x86_64-clang18-libcxx
ost runtime inspect openusd-24.11-linux-x86_64-clang18-libcxx
ost runtime verify openusd-24.11-linux-x86_64-clang18-libcxx
```

### 10.2 Plugin 開発操作

```bash
ost plugin init usdluma --kind usd-file-format
ost plugin build usdluma
ost plugin inspect usdluma
ost plugin doctor usdluma
ost plugin package usdluma
```

### 10.3 Plugin 実行・確認操作

```bash
ost plugin run usdluma -- usdcat fixtures/basic.lumagraph
ost plugin view usdluma fixtures/basic.lumagraph
ost plugin test-view usdluma fixtures/basic.lumagraph
ost plugin snapshot usdluma fixtures/basic.lumagraph
ost plugin verify usdluma fixtures/basic.lumagraph
ost plugin test usdluma
```

### 10.4 JSON 出力

エージェント・CI 向けに主要コマンドは JSON を提供する。

```bash
ost plugin doctor usdluma --json
ost plugin test usdluma --json
```

---

## 11. 検証ピラミッド

検証は、UI 起動の成否だけに依存させず、次の順に段階化する。

```text
Level 0: Bundle 構造検査
Level 1: Runtime / ABI 互換性検査
Level 2: Plugin Discovery 検査
Level 3: usdcat による最小読込検査
Level 4: Python API による Stage Open 検査
Level 5: 正規化 USD の golden comparison / round-trip 検査
Level 6: usdview 起動検査
Level 7: Hydra / Renderer 検査
Level 8: DCC Host 統合検査
```

### 11.1 Level 0 — Bundle 構造検査

確認対象:

- `openstrata.plugin.yaml` の妥当性
- 宣言された `plugInfo.json` が存在すること
- 共有ライブラリが存在すること
- Fixture / Expected Output の存在
- Bundle 内パスがポータブルであること

### 11.2 Level 1 — Runtime / ABI 互換性検査

確認対象:

- OpenUSD バージョン範囲
- OS / Architecture
- Compiler family
- C++ ABI
- Python ABI
- 宣言された依存コンポーネントの充足

### 11.3 Level 2 — Plugin Discovery 検査

確認対象:

- `PXR_PLUGINPATH_NAME` に正しいルートが含まれること
- `plugInfo.json` が解析可能であること
- 共有ライブラリをロードできること
- 対象 File Format / Schema / Resolver が Registry に現れること

### 11.4 Level 3 — `usdcat` 最小読込検査

File Format Plugin における最初の真実とする。

```bash
ost plugin run usdluma -- usdcat fixtures/basic.lumagraph
```

確認対象:

- 拡張子が認識されること
- Stage が開けること
- 致命的な USD Diagnostic が出ないこと
- USDA へ展開可能であること

### 11.5 Level 4 — Python Stage Open 検査

```bash
ost plugin run usdluma -- python tests/test_plugin.py
```

確認対象:

- `pxr.Usd.Stage.Open()` が成功すること
- 期待する Prim が存在すること
- 属性・メタデータ・参照が期待値を満たすこと

### 11.6 Level 5 — Golden Comparison / Round-trip 検査

`usdcat` の出力を正規化し、期待する USDA と比較する。

```bash
ost plugin snapshot usdluma fixtures/basic.lumagraph
ost plugin verify usdluma fixtures/basic.lumagraph
```

確認対象:

- Prim 階層
- TypeName
- 属性値
- Relationship
- Material Binding
- File Format 固有のメタデータ

出力順や一時的メタデータなど、比較に不適切な差分は正規化ルールで除外する。

### 11.7 Level 6 — `usdview` 起動検査

目視確認と自動検査を分ける。

開発者向け:

```bash
ost plugin view usdluma fixtures/basic.lumagraph
```

自動検査向け:

```bash
ost plugin test-view usdluma fixtures/basic.lumagraph
```

初期段階の自動検査では、以下を確認できればよい。

- `usdview` プロセスが起動すること
- 指定 Stage を開けること
- 起動直後に致命的なエラーで終了しないこと
- stderr に明確な Plugin / Python / Qt エラーがないこと

スクリーンショット比較や Qt 操作は将来の拡張とし、初期の真実は `usdcat + Python Stage Open + golden comparison` に置く。

### 11.8 Level 7 — Hydra / Renderer 検査

必要に応じて Render Delegate、MaterialX、テクスチャ解決、GPU 初期化を確認する。

これは Runtime Capability によって有効化される任意検証とする。

### 11.9 Level 8 — DCC Host 統合検査

Maya、Houdini、Blender などの Host Adapter を用いて検証する。

Standalone Runtime の成功後にのみ実施し、DCC 固有の問題を独立して扱えるようにする。

---

## 12. `ost plugin doctor` の設計

`ost plugin doctor` は OpenStrata の差別化ポイントである。

プラグインロード失敗を単一の曖昧なエラーで終わらせず、原因候補・観測事実・次の修正操作を示す。

```bash
ost plugin doctor usdluma --runtime openusd-24.11
```

想定出力:

```text
PASS  runtime.openusd.version          24.11 satisfies >=24.11,<25.0
PASS  runtime.cxx_abi                  libcxx
PASS  bundle.plug_info                 found plugin/resources/usdluma/plugInfo.json
PASS  session.plugin_path              configured
PASS  plugin.shared_library            found lib/usdluma.so
PASS  plugin.discovery                 file format registered: lumagraph
FAIL  dependency.materialx             libMaterialXCore.so.1.39 not found

Likely cause:
  usdluma.so was linked against MaterialX 1.39,
  but the selected Runtime does not provide it.

Suggested actions:
  - Select a Runtime that provides MaterialX 1.39
  - Rebuild the plugin against the Runtime-provided MaterialX version
```

### 12.1 診断で扱う主な項目

- Runtime と Manifest のバージョン一致
- Compiler / C++ ABI / Python ABI
- `plugInfo.json` の配置と内容
- Plugin Discovery Path
- 共有ライブラリの存在と依存関係
- 対象 Extension / Type / Resolver の登録
- Python module import
- MaterialX / OCIO / OIIO / Resolver などの追加依存
- 実行ログから抽出した OpenUSD Diagnostic

### 12.2 JSON レポート例

```json
{
  "plugin": "usdluma",
  "runtime": "openusd-24.11-linux-x86_64-clang18-libcxx",
  "status": "failed",
  "failed_checks": [
    {
      "id": "dependency.materialx",
      "reason": "Shared library dependency could not be resolved",
      "missing_dependency": "libMaterialXCore.so.1.39",
      "suggested_actions": [
        "Select a Runtime that provides MaterialX 1.39",
        "Rebuild usdluma against the Runtime-provided MaterialX"
      ]
    }
  ]
}
```

---

## 13. レポートと成果物

各実行は、再現・比較・CI 収集に必要な情報を保存する。

推奨出力先:

```text
.openstrata/reports/
└─ usdluma/
   └─ 2026-06-22T120000Z/
      ├─ summary.txt
      ├─ report.json
      ├─ environment.json
      ├─ diagnostics.log
      ├─ stdout.log
      ├─ stderr.log
      ├─ normalized-output.usda
      └─ diff.txt
```

保存対象:

- 実行した Runtime ID
- Plugin Bundle ID / version
- Session に適用した環境要約
- 検証項目ごとの結果
- stdout / stderr
- USD Diagnostic
- 正規化 USDA 出力
- Golden Comparison の差分
- 実行時間

これにより、ローカル失敗と CI 失敗を同じ情報構造で比較できる。

---

## 14. コーディングエージェント向け設計

OpenStrata は、人間の補助だけでなく、コーディングエージェントが自律的に修正ループを回すためのインターフェースを持つ。

エージェントの基本ループ:

```text
1. ost plugin build
2. ost plugin doctor --json
3. ost plugin test --json
4. failed_checks と diagnostics を読む
5. 実装・Manifest・依存設定を修正する
6. 再実行する
```

最低限提供するコマンド:

```bash
ost plugin init usdluma --kind usd-file-format
ost plugin build usdluma
ost plugin doctor usdluma --json
ost plugin test usdluma --json
ost plugin package usdluma
```

設計上の要点:

- Exit Code を明確にする
- エラー ID を安定化する
- JSON Schema を公開する
- ログを長文の自由文だけにしない
- Manifest の不備と実行時の不備を分離する
- 推奨修正を機械可読な `suggested_actions` として出す

---

## 15. CI / Jenkins との統合

OpenStrata は Jenkins などの CI 上で同じ CLI を実行する。

CI は特別な検証ロジックを持たず、Runtime Matrix を展開して `ost plugin test` を実行するオーケストレータに留める。

例:

```text
usdluma × openusd-24.11 × linux-x86_64
usdluma × openusd-25.x  × linux-x86_64
animus-resolver × openusd-24.11 × linux-x86_64
```

CI の基本フロー:

```bash
ost runtime install openusd-24.11-linux-x86_64-clang18-libcxx
ost plugin build usdluma
ost plugin test usdluma --runtime openusd-24.11-linux-x86_64-clang18-libcxx --json
ost plugin package usdluma
```

保存すべき成果物:

```text
Plugin Bundle
report.json
compatibility-result.json
diagnostics.log
normalized-output.usda
diff.txt
```

将来的には、検証済みの Runtime / Plugin の組み合わせを Compatibility Matrix として公開できるようにする。

```text
                    USD 24.11   USD 25.x
usdluma 0.1             PASS       PASS
animus-resolver 0.1     PASS       WARN
custom-hydra 0.1        SKIP       PASS
```

---

## 16. DCC Host Adapter 方針

DCC 対応は OpenStrata 本体に直接埋め込まない。Host ごとに Adapter を分離する。

```text
openstrata-host-maya
openstrata-host-houdini
openstrata-host-blender
```

共通の体験:

```bash
ost host run maya --plugin usdluma -- scene.ma
ost host test houdini --plugin usdluma
ost host doctor blender --plugin usdluma
```

Host Adapter の責務:

- DCC ごとの Runtime / USD 同梱構成の検出
- DCC 固有の Plugin Path 構成
- Headless 起動方法の抽象化
- Host 側ログの収集
- OpenStrata Report への正規化

ただし MVP では実装しない。先に Standalone Runtime が明確な成功基準を持つことを優先する。

---

## 17. MVP スコープ

### 17.1 対象プラグイン種別

初期対象は以下の3種とする。

1. `UsdFileFormatPlugin`
2. Asset Resolver
3. Schema Plugin

これらは OpenUSD 拡張の典型的な課題をカバーし、OpenStrata の Runtime / Discovery / ABI / Test 設計を十分に検証できる。

### 17.2 初期実装機能

```text
Runtime
- runtime list / install / inspect / verify

Plugin
- plugin init / build / inspect / doctor / run / test / view / package

Verification
- Bundle validation
- Runtime compatibility validation
- Plugin discovery validation
- usdcat smoke test
- Python Stage Open test
- normalized USDA golden comparison
- basic usdview launch test

Reports
- text summary
- JSON report
- logs and normalized outputs

Platform
- Linux x86_64 only
```

### 17.3 MVP でやらないこと

- 複数OSの完全対応
- DCC Host Adapter の本格対応
- GUI ベースのテスト自動化
- 分散ビルドシステムの全機能
- 一般化されたパッケージエコシステム
- GPU / Hydra の広範な組み合わせ保証
- AI Runtime の統合

これらは Runtime Contract と Plugin Bundle Contract を壊さない形で後続フェーズに追加する。

---

## 18. 段階的ロードマップ

### Phase 1 — Standalone Plugin Harness

目的: OpenUSD Plugin をローカルおよび CI で再現可能に検証できる状態を作る。

成果物:

- `ost` CLI
- 固定 Runtime
- Plugin Bundle Manifest
- Runtime Session Builder
- `usdcat` / Python / `usdview` 基本検証
- `doctor` と JSON report

### Phase 2 — Artifact と Compatibility Matrix

目的: 検証済み Bundle と Runtime の組み合わせを配布・再利用できる状態を作る。

成果物:

- `ost plugin package`
- `ost plugin publish`
- `ost plugin fetch`
- Artifact Registry 連携
- Runtime / Plugin Compatibility Matrix
- Jenkins による Matrix 検証

Vitrakiln は、ビルド済み Runtime / Plugin Bundle の成果物配布先として連携候補になる。

### Phase 3 — Host Adapter

目的: Standalone 成功状態を、DCC の実行環境へ持ち込む。

成果物:

- Maya / Houdini / Blender Adapter
- Headless host tests
- Host-specific diagnostics
- DCC version matrix

### Phase 4 — VFX / AI Application Runtime

目的: OpenUSD Plugin Harness で確立した Runtime Contract を、専用VFXツール・AIワーカー・Web / Wasm へ拡張する。

---

## 19. 成功基準

MVP の成功は、次の状態を実現できることとする。

1. 新しい OpenUSD File Format Plugin をテンプレートから作れる
2. 開発者が手作業で環境変数を設定せずに `usdcat` で読込確認できる
3. `usdview` を同じ Runtime Session 上で起動できる
4. Plugin Discovery 失敗時に `doctor` が原因候補を提示できる
5. Python Stage Open と USDA golden comparison を自動実行できる
6. 同じコマンドをローカルと Jenkins で実行できる
7. エージェントが JSON report を読んで修正ループを回せる
8. Runtime / Plugin の対応状況を将来的に Matrix として蓄積できる

最初の勝ち筋は、以下の一文に集約される。

> `ost plugin test` が、OpenUSD プラグイン開発で「まずこれを叩けば、動作状況と失敗原因が分かる」標準コマンドになること。

---

## 20. 実装上の注意

### 20.1 Runtime の中身と Manifest の乖離を許さない

Manifest に書かれたコンポーネント情報は、ビルド・パッケージング・検証時に自動生成または実測で検証する。手書きのバージョン表だけを信頼しない。

### 20.2 ライブラリ探索パスは最小化する

グローバルな `LD_LIBRARY_PATH` 依存は副作用が大きい。Session 内だけで設定し、必要であれば rpath / install name / bundle layout も活用する。

### 20.3 `plugInfo.json` は検証可能な成果物とする

Plugin の存在確認はファイルの存在だけで終わらせない。Registry で実際に発見・ロード可能であることを検証する。

### 20.4 Golden Test の差分ノイズを管理する

USDA のテキスト比較をそのまま採用すると、順序や非本質的メタデータで壊れやすい。正規化ルール、比較対象の選別、差分表示を初期から設計する。

### 20.5 `usdview` は便利だが主判定にしない

GUI、Qt、GPU、Display Server の影響を受けるため、コアの正しさは `usdcat` と Python API の検証で担保する。`usdview` は開発者体験と追加ヘルスチェックとして位置づける。

---

## 21. 結論

OpenStrata の最初の実装は、VFX Runtime 全体を一度に管理するものではない。

まずは、OpenUSD Plugin を確実に **組み込み、発見し、実行し、診断し、検証し、CI で再現する**ための標準ハーネスを作る。

この範囲を高い完成度で成立させれば、OpenStrata は単なる USD 開発補助ツールではなく、将来の VFX / AI Runtime Platform の核になる。

> OpenStrata は、互換性をドキュメント上の注意事項ではなく、実行可能で検証可能な Runtime Asset に変える。
