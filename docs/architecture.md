# アーキテクチャ

## 1. 全体像

mdpilot は **単一プロセス・複数スレッド** のネイティブ GUI アプリケーション。`eframe` のイベントループをメインスレッドで回し、`claude` 子プロセスとの stdin/stdout I/O とファイル監視をバックグラウンドスレッドに分離する。

```
┌─────────────────────────────────────────────────────────────────┐
│ mdpilot (単一プロセス)                                           │
│                                                                  │
│  ┌──────────────────────────┐    ┌──────────────────────────┐    │
│  │ メインスレッド            │    │ バックグラウンドスレッド  │    │
│  │ (eframe イベントループ)   │    │                          │    │
│  │                          │    │  ┌────────────────────┐  │    │
│  │  ┌────────────────────┐  │    │  │ claude stdout      │  │    │
│  │  │ ui::App            │◄─┼────┼──┤ 読込スレッド       │  │    │
│  │  │  - layout          │  │    │  │ (stream-json)      │  │    │
│  │  │  - preview pane    │  │    │  └──────────┬─────────┘  │    │
│  │  │  - chat pane       │  │    │             │            │    │
│  │  └─────────┬──────────┘  │    │             ▼            │    │
│  │            │             │    │      ┌──────────────┐    │    │
│  │            │  mpsc       │    │      │ claude 子PRC │    │    │
│  │            │  Sender/Recv│    │      │  (--print    │    │    │
│  │            │             │    │      │   stream-json)│   │    │
│  │            │             │    │      └──────▲───────┘    │    │
│  │            │ stdin write │    │             │            │    │
│  │            └─────────────┼────┼─────────────┘            │    │
│  │                          │    │  ┌────────────────────┐  │    │
│  │                          │    │  │ ファイル監視タスク  │  │    │
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
│   └── chat_pane.rs         // 右ペイン (chat モジュールに描画委譲)
├── chat/
│   ├── mod.rs
│   ├── session.rs           // claude 子プロセス起動・stdin/stdout 配線・状態保持
│   ├── stream.rs            // stream-json イベントのパース・モデル化
│   ├── history.rs           // チャット履歴の保持・描画用データ
│   ├── view.rs              // chat UI の描画（メッセージ・ツール展開ブロック・入力欄）
│   └── session_store.rs     // プロジェクトルートと session-id の対応をディスクに保存
├── preview/
│   ├── mod.rs
│   ├── loader.rs            // ファイル読込
│   ├── watcher.rs           // notify によるファイル監視
│   └── render.rs            // egui_commonmark による描画
├── claude/
│   ├── mod.rs               // Claude CLI の起動引数組み立て (claude-integration.md)
│   └── (詳細は実装着手時)
├── config/
│   ├── mod.rs               // 設定読込・既定値
│   └── paths.rs             // OS 別の設定/データディレクトリ解決
└── error.rs                 // クレート共通エラー型
```

注: モジュール分割は MVP の出発点。実装中に再編する可能性あり。

## 3. データフロー

### 3.1 入力（キーボード・マウス）

```
ユーザー入力 → egui イベント → App::update() → chat_pane / preview_pane
   └─ chat_pane:    入力欄 (egui::TextEdit) の確定で claude stdin にメッセージ JSON を書き込み
   └─ preview_pane: スクロール等の UI 状態更新のみ
```

### 3.2 claude 応答の反映

```
claude が API レスポンスをストリーミング
   → claude stdout (JSON Lines = stream-json)
   → 読込スレッドが 1 行ごとに parse (serde_json)
   → mpsc チャネルで AppEvent::ChatChunk(...) を送信
   → メインスレッドが履歴に追記し ctx.request_repaint()
   → 次フレームで chat::view が新しいメッセージを描画
```

stream-json には少なくとも以下のイベントタイプが流れる（詳細は `chat.md`）：

- `system`（init イベント、session_id を含む）
- `assistant`（テキスト断片）
- `assistant` の `content` 内の `tool_use`
- `user` の `content` 内の `tool_result`
- `result`（完了マーカー）

### 3.3 F-08: 表示中ファイルの自動再レンダリング

```
claude や任意のエディタがファイルを書き換え
   → notify がイベント検出 (バックグラウンドスレッド)
   → mpsc チャネルで AppEvent::FileChanged(path) 通知
   → メインスレッドが該当ファイルを再読込
   → egui_commonmark が AST 再構築・描画
```

F-08 は claude の存在に依存しない（任意の外部エディタによる編集にも追従する）。

### 3.4 F-09: 表示対象ファイルの自動切替

`claude-integration.md` 5 章「自動追従」案 A：プロジェクトルート以下の `.md` 監視で、現在表示中以外の `.md` 書換を検出したら対象切替。stream-json の `tool_use`/`tool_result` を直接解釈する必要は無く、ファイルシステムイベントだけで成立する。

## 4. スレッド・並行モデル

| スレッド | 役割 | 通信 |
|---|---|---|
| メインスレッド | egui のイベントループ・UI 描画・claude stdin への書込み | mpsc::Receiver で他スレッドからイベント受信 |
| claude stdout 読込スレッド | claude 子プロセスの stdout を 1 行ずつ読み、serde_json で stream-json イベントにパース | mpsc::Sender でメインスレッドに `ChatChunk` を送る |
| claude stderr 読込スレッド | claude 子プロセスの stderr をログに出す（tracing 経由） | mpsc::Sender でエラーイベントを送る |
| ファイル監視スレッド | `notify` の Watcher | mpsc::Sender で `FileChanged` を送る |

`tokio` の採用可否は MVP 着手時に判断する。`std::thread` + `std::sync::mpsc` で足りる見込みだが、claude 子プロセスの再接続や多重 IO が増えてきたら再評価する。

## 5. 状態管理

`App` 構造体に全状態を集約し、`eframe::App::update()` で毎フレーム参照・更新する。

```rust
struct App {
    layout: LayoutState,           // ペイン幅の比率など
    chat: ChatSession,             // claude 子プロセスハンドル・履歴・入力欄状態
    preview: PreviewState,         // 表示中ファイルパス・本文・AST キャッシュ
    watcher: FileWatcher,          // notify Watcher
    config: Config,                // 設定
    events: Receiver<AppEvent>,    // 他スレッドからのイベント
}
```

注: 上記は構造の意図を示す擬似コード。実際の型名・分割は実装時に確定する。

## 6. エラーハンドリング

- クレート共通エラー型 `mdpilot::error::Error` を `thiserror` で定義
- 起動時の致命的エラー（claude プロセス起動失敗、eframe 初期化失敗）はダイアログを出して終了
- 実行時の非致命的エラー（ファイル読込失敗、notify エラー、claude プロセス予期せぬ終了）は UI 上のトースト/ステータスバーで通知
- claude 子プロセスが終了した場合、chat ペインに「Claude 接続が切れました。再接続するには…」のような明示メッセージを出す
- panic は `std::panic::set_hook` で捕捉してログに残し、可能ならダイアログを出す

## 7. 主要依存ライブラリ

| クレート | 用途 | 備考 |
|---|---|---|
| `eframe` / `egui` | GUI フレームワーク | |
| `egui_commonmark` | Markdown プレビュー | 暫定 |
| `pulldown-cmark` | Markdown パーサ | `egui_commonmark` の依存 |
| `syntect` | コードブロックのシンタックスハイライト | `egui_commonmark` の `better_syntax_highlighting` feature 経由 |
| `notify` | ファイル監視 | F-08 / F-09 |
| `serde`, `serde_json` | stream-json パース・設定シリアライズ | |
| `thiserror` | エラー型定義 | |
| `directories` | OS 別の設定/データ/キャッシュディレクトリ解決 | |
| `tracing`, `tracing-subscriber` | ロギング | |
| `tokio` または `async-std` | 採用可否は未確定（MVP は `std::thread`） | |

ターミナル系（`egui_term`, `alacritty_terminal`, `portable-pty`）は採用しない（`requirements.md` 6 章スコープ外）。

## 8. ビルド・配布

- ビルドツール: `cargo`
- バンドル: `cargo-bundle`（macOS の `.app` 作成）と Windows 向けのスクリプト or `cargo-dist`
- ターゲット: `aarch64-apple-darwin`, `x86_64-apple-darwin`, `x86_64-pc-windows-msvc`
- macOS のコード署名・公証、Windows の署名は当面行わない（ユーザーが自前でビルドする想定）

## 9. 未確定事項

| 項目 | 関連 |
|---|---|
| `tokio` を採用するか std::thread で進めるか | claude 子プロセスとの IO で必要に応じて判断 |
| 設定ファイルの形式（TOML / JSON） | 別途決定 |
| ロギング出力先（標準エラー / ファイル） | MVP は標準エラー固定（`docs/plan.md` 6 章） |
| syntect のテーマセット | 別途決定 |
| stream-json のスキーマ追従戦略 | `chat.md` で詳細化 |
