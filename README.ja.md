# Nekod

[简体中文](README.md) | [English](README.en.md) | **日本語**

> ✨ **本プロジェクトは Gemini Neon モデルのみで開発されました :p**

**Modding games, together.**  
*Open-source Community Mod Platform (`v1.0.0`) — オフラインシングルプレイゲーム向けのオープンソース Mod・トレーナー統合プラットフォーム*

[![License: MIT](https://img.shields.io/badge/License-MIT-38B788.svg)](LICENSE)
[![Release](https://img.shields.io/badge/Release-v1.0.0-111514.svg)](.github/workflows/release.yml)

Nekod は一般プレイヤーと Mod クリエイター双方のために設計されたオープンソースのシングルプレイゲーム支援プラットフォームです。**Deep Emerald Obsidian** 配色の高密度ワークベンチと即時多言語切替（**简体中文 / English / 日本語**）を備え、単独トレーナー、Cheat Engine `.CT` テーブル、Unity（BepInEx Mono / IL2CPP）、Unreal Engine（RE-UE4SS）向け Mod を検証可能な `.nmod` オープン規格に統合します。

> **2つの体験を明確に分離:**
> - **一般プレイヤー向け（AI 不要・ノーコード）：** Nekod デスクトップを起動するだけでゲーム自動検出、オンラインストアからのワンクリック導入、複数 Mod 同時注入、グローバルホットキー設定、クリックスルー RGB HUD 表示が利用できます。**AI アカウント・API キー・MCP・CLI は一切不要です。**
> - **クリエイター &「MCP で AI に“お願い”して Mod 作成」（任意）：** オプションの `nekod-mcp.exe` を MCP 対応 AI に接続すると、プログラミング不要の自然な会話でゲーム構造を安全に調査させ、SHA-256 検証済み `.nmod` パッケージを生成・梱包できます。

---

## 主な特徴 / Highlights

- **Deep Emerald Obsidian ワークベンチ：** `#0B1B16` 深森林グリーンサイドバー + `#0B0E0D` オブシディアン作業領域 + 最小限の `1px` 境界線と `#38B788` ミントエメラルド強調色。起動直後から 2 カラム作業画面を表示し、**简体中文 / English / 日本語**を即時切替可能。
- **公式 Mod リポジトリ標準対応：** 公式バケット [`OmaruKoma/Nekod-Mods`](https://github.com/OmaruKoma/Nekod-Mods)（ソース公開・SHA-256 検証・`game-manifest.json` 自動登録）を初期購読し、任意のサードパーティ `registry.json` も追加できます。
- **複数 Mod 同時併用 & LIFO ロールバック：** 1 つのゲームに複数 `.nmod`（例：ゲームプレイ改変 Mod + UI 日本語/中国語化 Mod）を重ねて注入でき、`PatchLedger` により LIFO 順で安全に復元します。
- **快適なプレイ体験：** **起動して自動注入（Play & Inject）**、自動アタッチ、キー押下録音ホットキーカプセル、トレイ常駐、ゲーム右上クリックスルー **RGB ArrayList HUD オーバーレイ** を搭載。
- **内蔵コンパニオンブリッジ & ローダー管理（`DeploymentLedger`）：**
  - **Unity Mono / IL2CPP（`v0.5.5`）：** UnityExplorer 不要。BepInEx + `NekodUnityBridge` を自動配備し、フィールド固定、Harmony フック、クォータニオン SpinBot、DamageAura、GameObject 表示切替、パッケージ内 SHA-256 検証済みテクスチャ/マテリアル置換を実現。
  - **Unreal Engine 4/5（`v0.7.3`）：** `RE-UE4SS` + `NekodBridge` を自動配備し、シングルスレッド・ゲームスレッド同期で実行時 UMG 翻訳、粘着 UProperty ロック、エイムロック、自動射撃、三人称 SpinBot、`ApplyDamage` オーラなどを実行。
- **Fail-Closed `.CT` 静的コンバータ：** モジュール相対オフセット・ポインタチェーン・`aobscanmodule + db` のみを SHA-256 付き宣言的パッケージへ安全に変換し、`{$lua}`・`alloc()`・外部スクリプトは確実に拒否します。

---

## クイックスタート（プレイヤー向け / Windows x64）

### 1. ダウンロードとインストール

**Releases** ページから取得できます（必要に応じて `SHA256SUMS.txt` でハッシュ検証可能）：

- **インストーラー版（推奨）：** `nekod-windows-x64-setup.exe`（NSIS ユーザーインストーラー）
- **ポータブル版：** `nekod-windows-x64-portable.zip`（`nekod-desktop.exe`、`nekod-validator.exe`、任意利用の `nekod-mcp.exe` を同梱）

```powershell
# ハッシュ確認（任意）
Get-FileHash .\nekod-windows-x64-setup.exe -Algorithm SHA256
```

### 2. 4 ステップで使い始める

1. **ゲームを検出：** 左側のサイドバーに Steam / Epic / GOG ゲームが自動検出されます。ポータブル版などは左上の `+` からフォルダや `.exe` を追加できます。
2. **`.nmod` を入手：**
   - 上部タブの **「Mod ストア」** → **「すべて同期」** から `OmaruKoma/Nekod-Mods` の Mod をインストール；
   - またはローカルの `.nmod` / `.ct` ファイルを Nekod ウィンドウへ**ドラッグ＆ドロップ**するだけでも導入できます。
3. **ブリッジ配備（必要な場合）：** Unity / Unreal ゲームで必要な場合、詳細メニュー（`···`）または下部ドロワーから BepInEx / RE-UE4SS とコンパニオンブリッジをワンクリック配備します。
4. **権限承認して起動：** Default-Deny 権限一覧を確認して承認後、**「起動して自動注入」**（または起動済みゲームに**「ワンクリック注入」**）を押すだけです。

---

## MCP で AI に「お願い」して Mod を作る（ノーコード Mod 工房）

> **コードを書けなくても専用 Mod を作れます！**  
> ストアにお目当てのマイナーゲーム用 Mod がまだない場合でも、同梱の **`nekod-mcp.exe`** を MCP 対応 AI アシスタントへ接続すれば、日常会話で「無敵と所持金無限の Mod を作って」と頼むだけで、AI がゲーム構造を調査して `.nmod` パッケージを組み上げてくれます。

### ステップ 1：`nekod-mcp.exe` を用意

- **ポータブル版（`nekod-windows-x64-portable.zip`）：** 解凍フォルダ内に `nekod-mcp.exe` が同梱されています。
- **ソースからビルド：** `cargo build --release -p nekod-mcp` で `target\release\nekod-mcp.exe` が生成されます。

### ステップ 2：AI クライアントの MCP 設定に追加

お使いのクライアント設定に以下を追加してください（`"command"` は実在する絶対パスに書き換え、`\` は `\\` とエスケープしてください）：

#### Claude Desktop / Antigravity / Qoder / Windsurf

```json
{
  "mcpServers": {
    "nekod": {
      "command": "D:\\Tools\\Nekod\\nekod-mcp.exe",
      "args": [
        "--data-dir",
        "C:\\Users\\ユーザー名\\AppData\\Roaming\\org.nekod.desktop"
      ]
    }
  }
}
```

#### Cursor（`.cursor/mcp.json`）

```json
{
  "mcpServers": {
    "nekod": {
      "command": "D:\\Tools\\Nekod\\nekod-mcp.exe"
    }
  }
}
```

> **ヒント：** `--data-dir` に Nekod デスクトップのデータフォルダを指定すると登録済みゲームをそのまま参照できます（省略時は一時サンドボックスを使用）。設定後に AI クライアントを再起動し、**24 個の `nekod` ツール**が認識されていれば準備完了です。

### ステップ 3：ゲーム側の下準備（1 分）

1. **Nekod デスクトップ**に対象のシングルプレイゲームを追加して選択します。
2. Unity / Unreal ゲームの場合、ヘッダーの `···` メニューから**ブリッジ配備**を行い、ゲームを起動してプレイ画面に入ります。

### ステップ 4：「お願いテンプレート」をコピーして AI に送信

#### お願い例 1：Unity ゲーム用トレーナー（無敵 / 通貨 / 移動速度）
> 「`nekod` MCP を使って追加済みの Unity シングルプレイゲーム（パス：`D:\Games\MyUnityGame`）を調べてください：
> 1. まず `inspect_game_environment` と `inspect_unity_assemblies`（読み取り専用）で HP・所持金/弾薬・移動速度に関わるクラスとフィールドを特定してください。
> 2. ゲームと Nekod ブリッジは起動済みなので、必要なら `probe_unity_bridge` で実際のインスタンス名を確認してください。
> 3. `forge_nmod_package` で【無敵（F5）/ 所持金 99999（F6）/ 移動速度倍率スライダー】を持つ `.nmod` を作成し、`verify_forged_mod_package` で検証後に `pack_nmod_archive` で出力した `.nmod` パスを教えてください。」

#### お願い例 2：Unreal Engine 4/5（`ue4ss_bridge_v1`）Mod 作成
> 「UE4SS ブリッジ導入済みの起動中 Unreal ゲームを `nekod` MCP で確認してください：
> 1. `probe_ue4ss_bridge`（読み取り専用）でプレイヤーやインベントリの UProperty / UFunction 名を調査してください。
> 2. `forge_nmod_package`、`verify_forged_mod_package`、`pack_nmod_archive` を使って、スタミナ無限・無反動・アイテム補充を備えた `ue4ss_bridge_v1` `.nmod` を作成してください。」

#### お願い例 3：Cheat Engine `.CT` テーブルを安全な `.nmod` に変換
> 「`D:\Downloads\Game.CT` を `analyze_ct_file` で解析し、安全な静的オフセット・ポインタ・AOB パッチだけを抽出して `.nmod` パッケージとして生成・梱包してください（危険な Lua や外部スクリプトは除外すること）。」

### なぜ安全なのか？（読み書き完全分離 & Default-Deny）

| ツールグループ | 読み取り専用プローブ（副作用なし） | 制御された生成・検証（明示的承認必須） |
| :--- | :--- | :--- |
| **ゲーム・エンジン検出** | `list_installed_games`, `get_game_info`, `inspect_game_environment`, `recommend_runtime_adapter`, `inspect_unity_assemblies` | — |
| **ライブブリッジ調査** | `probe_unity_bridge`（`ping` / `list_types` / `list_members` / `find_instances` / `dump_hierarchy` / `get_field` 限定）, `probe_ue4ss_bridge`, `inspect_authorized_game_process`, `inspect_trainer_state`, `get_execution_logs` | — |
| **Mod 鍛造 & `.CT` 解析** | `analyze_ct_file`, `validate_trainer_manifest`, `inspect_trainer` | `forge_nmod_package`, `pack_nmod_archive`, `create_trainer_manifest`, `generate_trainer_template` |
| **互換性 & リリース検証** | `check_game_version`, `validate_compatibility`, `inspect_runtime_diagnostics`, `validate_repository`, `inspect_registry_entry` | `verify_forged_mod_package`, `prepare_trainer_release` |

AI は権限の自動昇格、信頼ティアの改ざん、アンチチート回避を一切行えません。生成された `.nmod` も Nekod 導入時に JSON Schema 検証・SHA-256 確認・ユーザー承認ゲートを必ず通過します。

---

## 6 層アーキテクチャ & セキュリティ方針

| Layer | 名称 | パス |
| :--- | :--- | :--- |
| **L1** | Desktop Application（Tauri 2 + React、AI / MCP 非依存） | [`apps/desktop/`](apps/desktop/README.md) |
| **L2** | Application Core（ゲーム管理・権限・レジストリ） | [`crates/nekod-core/`](crates/nekod-core/Cargo.toml) |
| **L3** | Trainer Runtime（Win32/Linux、Unity/UE4SS、`PatchLedger`、`DeploymentLedger`） | [`crates/trainer-runtime/`](crates/trainer-runtime/Cargo.toml) |
| **L4** | SDK & Protocol（`GameManifest` / `TrainerManifest` / `ModManifest` v2） | [`sdk/`](sdk/manifest/Cargo.toml), [`schemas/`](schemas/README.md) |
| **L5** | Community Registry（`OmaruKoma/Nekod-Mods` 仕様 & `nekod-validator`） | [`community/`](community/README.md), [`tools/trainer-validator/`](tools/trainer-validator/Cargo.toml) |
| **L6** | AI Developer Tooling（任意接続 `nekod-mcp`、Agent Skills） | [`crates/nekod-mcp/`](crates/nekod-mcp/Cargo.toml), [`skills/`](skills/README.md) |

- **Default-Deny 権限モデル：** 承認は `(trainer_id, version, manifest_sha256)` に束縛され、改変時は再承認を強制します。
- **シングルプレイ専用：** アンチチート回避・ステルス注入・オンライン不正行為は厳禁です。検出時は即座に処理を中断します（[`SECURITY.md`](SECURITY.md)）。

---

## コントリビュート & ソースからのビルド

実ゲーム用 `.nmod` パッケージは [**`OmaruKoma/Nekod-Mods`**](https://github.com/OmaruKoma/Nekod-Mods) で管理されています。本体 `examples/` には検証用最小スターターのみを収録しています。

```bash
# 1. フロントエンドのビルド
npm --prefix apps/desktop install
npm --prefix apps/desktop run build

# 2. フォーマット・Clippy・全テストの実行
cargo fmt --all -- --check
cargo check --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace

# 3. スターターパッケージの検証
cargo run -p nekod-validator -- package examples/sample-singleplayer-rpg
cargo run -p nekod-validator -- nmod examples/unity-standalone-starter
cargo run -p nekod-validator -- nmod examples/unity-explorer-mcp-starter

# 4. 開発起動 / リリースバイナリ生成
cd apps/desktop && npm run tauri dev
cargo build --release -p nekod-desktop -p nekod-mcp -p nekod-validator
```

---

## License

Licensed under the [MIT License](LICENSE) (see [ADR-009](TECHNICAL_DECISIONS.md#adr-009-repository-license-mit)).
