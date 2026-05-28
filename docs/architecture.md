# アーキテクチャ

## 1. 全体像

mdpilot は **単一プロセス・複数スレッド** のネイティブ GUI アプリケーション。`eframe` のイベントループをメインスレッドで回し、PTY I/O とファイル監視をバックグラウンドスレッドに分離する。

```
┌─────────────────────────────────────────────────────────────────┐
│ mdpilot (単一プロセス)                                           │
│                                                                  │
│  ┌──────────────────────────┐    ┌──────────────────────────┐    │
│  │ メインスレッド            │    │ バックグラウンドスレッド  │    │
│  │ (eframe イベントループ)   │    │                          │    │
│  │                          │    │  ┌────────────────────┐  │    │
│  │  ┌────────────────────┐  │    │  │ PTY 読込タスク      │  │    │
│  │  │ ui::App            │◄─┼────┼──┤ (alacritty_terminal│  │    │
│  │  │  - layout          │  │    │  │  / portable-pty)   │  │    │
│  │  │  - preview pane    │  │    │  └──────────┬─────────┘  │    │
│  │  │  - terminal pane   │  │    │             │            │    │
│  │  └─────────┬──────────┘  │    │             ▼            │    │
│  │            │             │    │      ┌──────────────┐    │    │
│  │            │  mpsc::Sender│    │      │ シェル子PRC  │    │    │
│  │            │  / Receiver  │    │      │ (zsh/pwsh    │    │    │
│  │            │             │    │      │  /claude...) │    │    │
│  │            │             │    │      └──────────────┘    │    │
│  │            │             │    │                          │    │
│  │            │             │    │  ┌────────────────────┐  │    │
│  │            └─────────────┼────┼──┤ ファイル監視タスク  │  │    │
│  │                          │    │  │ (notify)           │  │    │
│  │                          │    │  └────────────────────┘  │    │
│  └──────────────────────────┘    └──────────────────────────┘    │
└─────────────────────────────────────────────────────────────────┘
```

## 2. モジュール構成

クレートは単一バイナリ `mdpilot`。内部モジュールは以下に分ける。

```
src/
├── main.rs                  // エントリポイント (eframe::run_native)
├── app.rs                   // App 構造体 (eframe::App 実装) と状態統合
├── ui/
│   ├── mod.rs
│   ├── layout.rs            // 2ペイン分割、リサイズハンドル
│   ├── preview_pane.rs      // 左ペイン (preview モジュールに描画委譲)
│   └── terminal_pane.rs     // 右ペイン (terminal モジュールに描画委譲)
├── terminal/
│   ├── mod.rs
│   ├── session.rs           // PTY 起動・読み書き・状態保持
│   └── view.rs              // egui_term による描画
├── preview/
│   ├── mod.rs
│   ├── loader.rs            // ファイル読込
│   ├── watcher.rs           // notify によるファイル監視
│   └── render.rs            // egui_commonmark による描画
├── claude/
│   ├── mod.rs               // Claude Code 連携 (詳細は claude-integration.md)
│   └── (実装は未確定)
├── config/
│   ├── mod.rs               // 設定読込・既定値
│   └── paths.rs             // OS 別の設定/データディレクトリ解決
└── error.rs                 // クレート共通エラー型
```

注: モジュール分割は MVP の出発点。実装中に再編する可能性あり。

## 3. データフロー

### 3.1 入力（キーボード・マウス）

```
ユーザー入力 → egui イベント → App::update() → terminal_pane / preview_pane
   └─ terminal_pane: キー入力を PTY に書き込み
   └─ preview_pane:  スクロール等の UI 状態更新のみ
```

### 3.2 ターミナル出力の反映

```
シェル/claude 出力
   → PTY マスタ (バックグラウンドスレッド)
   → alacritty_terminal が VTE パース・グリッド更新
   → mpsc チャネルで「更新あり」通知
   → メインスレッドが ctx.request_repaint()
   → 次フレームで terminal::view が新しいグリッドを描画
```

### 3.3 F-08: ファイル変更追従

「既に表示中のファイル」が外部から書き換わった際にプレビューを更新するフロー。連携方式によらず必ず必要。

```
Claude Code がファイルを書き換え
   → notify がイベント検出 (バックグラウンドスレッド)
   → mpsc チャネルで「ファイル変更」通知
   → メインスレッドが該当ファイルを再読込
   → egui_commonmark が AST 再構築・描画
```

### 3.4 F-09: 表示対象ファイルの指定（実現方式は未確定）

「どのファイルをプレビュー対象にするか」を Claude Code 側から指定するフロー。**3.3 とは独立した別問題**であり、実現方式は `claude-integration.md` で MCP サーバ案・ファイル監視で直近編集ファイルを推定する案・明示コマンド案などを比較検討する。

```
Claude Code の発話/ツール呼び出し
   → mdpilot が何らかの方式で「このファイルを表示」要求を受信 (方式未確定)
   → preview pane の対象ファイルを切り替え
   → 以後 3.3 のフローに乗る
```

## 4. スレッド・並行モデル

| スレッド | 役割 | 通信 |
|---|---|---|
| メインスレッド | egui のイベントループ・UI 描画 | mpsc::Receiver で他スレッドからイベント受信 |
| PTY 読込スレッド | PTY マスタからの読込 → VTE パース → グリッド更新 | mpsc::Sender でメインスレッドに repaint 要求 |
| ファイル監視スレッド | `notify` の Watcher | mpsc::Sender で変更通知 |

`tokio` の採用可否は MVP 着手時に判断する。`std::thread` + `std::sync::mpsc` で足りる見込みだが、Claude Code との非同期 IPC を入れる段で必要なら再評価する。

## 5. 状態管理

`App` 構造体に全状態を集約し、`eframe::App::update()` で毎フレーム参照・更新する。

```rust
struct App {
    layout: LayoutState,           // ペイン幅の比率など
    terminal: TerminalSession,     // PTY ハンドル・端末グリッド・スクロールバック
    preview: PreviewState,         // 表示中ファイルパス・本文・AST キャッシュ
    watcher: FileWatcher,          // notify Watcher
    config: Config,                // 設定
    events: Receiver<AppEvent>,    // 他スレッドからのイベント
}
```

注: 上記は構造の意図を示す擬似コード。実際の型名・分割は実装時に確定する。

## 6. エラーハンドリング

- クレート共通エラー型 `mdpilot::error::Error` を `thiserror` で定義
- 起動時の致命的エラー（PTY 確保失敗・eframe 初期化失敗等）はダイアログを出して終了
- 実行時の非致命的エラー（ファイル読込失敗・notify エラー）は UI 上のトースト/ステータスバーで通知
- panic は `std::panic::set_hook` で捕捉してログに残し、可能ならダイアログを出す

## 7. 主要依存ライブラリ

| クレート | 用途 | 備考 |
|---|---|---|
| `eframe` / `egui` | GUI フレームワーク | |
| `egui_term` | ターミナル UI ウィジェット | `alacritty_terminal` をラップ |
| `alacritty_terminal` | 端末状態管理・VTE パース | `egui_term` の依存 |
| `portable-pty` | クロスプラットフォーム PTY | `egui_term` の依存 |
| `egui_commonmark` | Markdown プレビュー | 暫定 |
| `pulldown-cmark` | Markdown パーサ | `egui_commonmark` の依存 |
| `syntect` | コードブロックのシンタックスハイライト | 検討中 |
| `notify` | ファイル監視 | F-08（表示中ファイルの変更追従）のため。F-09 の連携機構とは独立 |
| `serde`, `serde_json` | 設定・IPC のシリアライズ | |
| `thiserror` | エラー型定義 | |
| `directories` | OS 別の設定/データ/キャッシュディレクトリ解決 | |
| `tracing`, `tracing-subscriber` | ロギング | |

## 8. ビルド・配布

- ビルドツール: `cargo`
- バンドル: `cargo-bundle`（macOS の `.app` 作成）と Windows 向けのスクリプト or `cargo-dist`
- ターゲット: `aarch64-apple-darwin`, `x86_64-apple-darwin`, `x86_64-pc-windows-msvc`
- macOS のコード署名・公証、Windows の署名は当面行わない（ユーザーが自前でビルドする想定）

## 9. 未確定事項

| 項目 | 関連 |
|---|---|
| `tokio` を採用するか std::thread で進めるか | Claude Code との IPC 設計確定後に決定 |
| 設定ファイルの形式（TOML / JSON） | 別途決定 |
| ロギング出力先（標準エラー / ファイル） | 別途決定 |
| syntect のテーマセット | 別途決定 |
