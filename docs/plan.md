# 実装計画: mdpilot 全機能段階的実装

## 1. Context（背景・目的）

mdpilot は Claude Code と協調して Markdown を書くためのネイティブ GUI アプリ。`claude` CLI を非対話モード（stream-json）で子プロセスとして spawn し、右ペインの chat UI で対話、左ペインで Markdown プレビューを表示する。**内蔵ターミナルエミュレータは持たない**。

設計フェーズ完了・Phase 0 / 0.5 まで実装済みの状態から、スケルトン → MVP → MVP 後の拡張までを段階的に積む全体実装計画を本書にまとめる。

参照する仕様書（すべて `docs/` 配下）：

- [requirements.md](requirements.md): 機能要件 F-01〜F-11 (MVP), F-21〜F-28 (MVP 後), 非機能要件 N-01〜N-07, スコープ外
- [architecture.md](architecture.md): 単一プロセス・複数スレッド構成、モジュール分割、データフロー、依存クレート
- [ui.md](ui.md): 2 ペイン構成、ウィンドウ既定値、メニュー、キーバインド、フォーカス、テーマ
- [chat.md](chat.md): chat UI 仕様、claude 子プロセスの起動、stream-json プロトコル
- [preview.md](preview.md): CommonMark + GFM、syntect、リンク・画像、ファイル監視（F-08）、対象切替
- [claude-integration.md](claude-integration.md): 自動追従（F-09 案 A）、追従 ON/OFF、起動条件
- [spike-report.md](spike-report.md): Phase 0.5 のスパイク結果と方針転換の経緯

設計方針の前提：

- MVP は F-01〜F-11 と各仕様書で「MVP 必須」と明記された項目に限定する
- F-09 は **案 A（自動追従）のみ** で MVP を成立させ、stream-json `tool_use` 解釈（案 B）・MCP（案 C）は MVP 後の拡張余地とする（`claude-integration.md` 5.2）
- パーミッションは MVP は `--dangerously-skip-permissions` で全スキップ。安全モード（F-28）は MVP 後（ユーザー判断）
- セッションは 1 ウィンドウ = 1 セッション、プロジェクトルートごとに session-id を 1 つ保存（F-11）
- 非機能要件 N-01〜N-04 は Phase 7（仕上げ）で測定・確認する
- Linux サポート・WYSIWYG・手動編集・プラグイン・自動アップデート・内蔵ターミナルは恒久的にスコープ外（`requirements.md` 6 章）

## 2. 対応一覧

各フェーズは前フェーズの完了を前提とする。同一フェーズ内のタスクは可能な範囲で並行可。

実装済みは ✓ / superseded（方針転換で廃止）は ✗ で示す。

| Phase | タスク# | タスク名 | 概要 | 依存 | 状態 |
|-------|---------|---------|------|------|------|
| 0 | 0.1 | リポジトリ初期化と Cargo パッケージ作成 | `Cargo.toml`, `src/main.rs`, `.gitignore` 等を作る | — | ✓ |
| 0 | 0.2 | 最小限の eframe アプリ起動 | `eframe::run_native` で空ウィンドウを開く | 0.1 | ✓ |
| 0 | 0.3 | エラー型とロギング基盤 | `src/error.rs` (`thiserror`), `tracing-subscriber` 初期化 | 0.1 | ✓ |
| 0 | 0.4 | 設定ディレクトリ解決 | `directories` で OS 別の config/data/cache パス取得 | 0.1 | ✓ |
| 0.5 | 0.5.1 | ~~egui_term 統合スパイク~~ | **superseded**：chat UI 路線に方針転換、ターミナル widget は不採用 | — | ✗ |
| 0.5 | 0.5.2 | egui_commonmark 統合スパイク | spike/egui_commonmark でビルド・起動を確認 | 0.2 | ✓ |
| 0.5 | 0.5.3 | スパイク結果のレポート | `docs/spike-report.md` を作成、方針転換を記録 | 0.5.2 | ✓ |
| 1 | 1.1 | レイアウト状態とペイン分割（F-01 前半） | `egui::Panel::left` + `CentralPanel`、`size_range(240..=avail-240)` | 0.5.2 | ✓ |
| 1 | 1.2 | 境界リサイズハンドル（F-01 後半） | ペイン境界 8px の hit strip 上でダブルクリック検出、`PanelState` を memory から削除して 50/50 にリセット。`Cmd+\` 配線は Phase 7.3 | 1.1 | ✓ |
| 1 | 1.3 | プレビュー/チャットのプレースホルダ描画 | 「プレビュー未指定」「Claude 接続準備中…」を `centered_and_justified` で表示 | 1.1 | ✓ |
| 1 | 1.4 | 日本語フォントの動的ロードと debug screenshot helper | `src/ui/fonts.rs` で macOS の AquaKana + Hiragino Sans GB を読み込み Proportional/Monospace に登録。`src/app.rs` に `cfg(debug_assertions)` の `MDPILOT_DEBUG_SCREENSHOT=path` トリガーを追加。Phase 1 の実機目視で発覚した tofu 問題を解消 | 1.3 | ✓ |
| 2 | 2.0 | `--session-id` セマンティクスと stream-json 必須オプションの実機検証 | `--session-id <new-uuid>` が新規セッションを作るか、`--verbose` 併用必須などの起動条件を確定 | 0.3 | ✓ |
| 2 | 2.1 | claude 子プロセスの起動 | `std::process::Command` で `claude` を spawn、`Stdio::piped()` で stdin/stdout/stderr。引数は `--print --verbose --input-format=stream-json --output-format=stream-json --include-partial-messages --dangerously-skip-permissions --session-id <uuid> [--continue]` | 2.0 | — |
| 2 | 2.2 | stream-json 入力スキーマの実機検証 | 公式未文書化の入力スキーマを `claude --print --input-format=stream-json` で実機テストし、`chat.md` 3.1 章に反映 | 2.1 | — |
| 2 | 2.3 | stdout 読込スレッドと stream-json パーサ | JSON Lines を 1 行ずつ `serde_json` で読み、未知イベントを除いて `AppEvent::ChatChunk` を送る | 2.1 | — |
| 2 | 2.4 | stderr 読込スレッドと tracing 連携 | claude の stderr 行を `tracing::warn`/`error` に流す | 2.1 | — |
| 2 | 2.5 | stdin 書込み | メインスレッドから claude stdin に JSON Lines で書く | 2.2, 2.3 | — |
| 2 | 2.6 | session-id ディスクストア | `data_dir/sessions.json` を atomic read/write、プロジェクトルートと session-id の mapping | 0.4 | — |
| 2 | 2.7 | 子プロセスのライフサイクル | 正常終了 / 異常終了の検出、mdpilot 終了時の SIGTERM → SIGKILL | 2.1 | — |
| 3 | 3.1 | チャット UI の枠（メッセージリスト + 入力欄 + ボタン） | `chat_pane.rs` で縦スクロール領域 + `TextEdit::multiline` + 送信/中断ボタン | 1.3 | — |
| 3 | 3.2 | テキストメッセージのストリーミング描画 | `text_delta` イベントを assistant メッセージに追記、ストリーミング中はカーソル表示 | 2.3, 3.1 | — |
| 3 | 3.3 | ツール呼び出しの collapsible 表示 | `tool_use` / `tool_result` を collapsible ブロックで描画（既定折りたたみ）、`F-04` | 3.2 | — |
| 3 | 3.4 | API リトライ・結果イベント表示 | `system/api_retry` を控えめバナー、`result.subtype != success` を赤系注釈 | 3.2 | — |
| 3 | 3.5 | 送信フローと IME 動作確認 | Enter で送信、Shift+Enter で改行、`egui::TextEdit` の IME を実機検証（N-07） | 3.1, 2.5 | — |
| 3 | 3.6 | 中断ボタンの実装方式確定 | claude プロセスへの SIGINT / `--max-turns` / 別 RPC のいずれかを実機検証して確定 | 3.5 | — |
| 3 | 3.7 | メッセージ選択・コピー（F-05） | ドラッグ選択、`Cmd+C` / `Ctrl+Shift+C` で OS クリップボードに | 3.2 | — |
| 4 | 4.1 | ファイル読込ローダー | 指定パスから UTF-8 読込、サイズしきい値（1MB/10MB）でモード切替 | 0.3 | — |
| 4 | 4.2 | egui_commonmark 描画 | CommonMark + GFM (テーブル / タスクリスト / 取り消し線) 確認、不足機能の補強方針決定（F-06） | 1.3, 4.1 | — |
| 4 | 4.3 | syntect シンタックスハイライト統合 | `better_syntax_highlighting` feature 経由、ダーク/ライト 2 テーマ、1MB 超ブロックはフォールバック（F-07） | 4.2 | — |
| 4 | 4.4 | リンク挙動 | 外部 URL は OS 既定ブラウザ、相対 `.md` は対象切替、その他は OS 既定アプリ | 4.2 | — |
| 4 | 4.5 | 画像・相対パス解決 | ローカル相対/絶対パスを `egui` 画像 API で表示、HTTP/HTTPS は MVP 非対応 | 4.2 | — |
| 4 | 4.6 | プレビューのスクロール位置保持 | 再読込前の最上端行を記憶、ベストエフォートで復元 | 4.2 | — |
| 5 | 5.1 | `notify` Watcher セットアップ | バックグラウンドスレッドで `RecommendedWatcher`、mpsc でメインに通知 | 0.3 | — |
| 5 | 5.2 | 単一ファイル監視と再レンダリング（F-08） | プレビュー対象 1 個を監視、100ms デバウンス、ファイル削除時の「見つかりません」表示 | 4.2, 5.1 | — |
| 5 | 5.3 | 監視エラーのステータス表示 | 監視開始失敗をステータスバー/トーストに出す、`Cmd+R`/`Ctrl+R` で手動再読込 | 5.2 | — |
| 6 | 6.1 | プロジェクトルート解決 | 起動引数（`mdpilot <dir>` / `<file>` / 引数なし）から root を決定 | 0.4 | — |
| 6 | 6.2 | プロジェクト配下 `.md` の再帰監視 | 除外ディレクトリ（`.git`, `node_modules`, `target` 等）を除き再帰監視 | 5.1, 6.1 | — |
| 6 | 6.3 | 自動追従ロジック（F-09 案 A） | 「現在表示中以外の `.md` 書き換え」で対象切替、200ms デバウンス | 5.2, 6.2 | — |
| 6 | 6.4 | 起動直後の対象選択 | `<file>` 指定時はそのファイル、`<dir>` 指定時は `README.md` 検索、なければ空ペイン | 6.1, 6.3 | — |
| 6 | 6.5 | `MDPILOT_PROJECT_ROOT` 環境変数の付与 | claude 子プロセスに絶対パスを渡す | 2.1, 6.1 | — |
| 7 | 7.1 | `Cmd+O`/`Ctrl+O` ファイル選択ダイアログ | `rfd` 等で `.md` 選択 | 4.2, 6.3 | — |
| 7 | 7.2 | 自動追従モード ON/OFF | `Cmd+O` で OFF、パスバーのボタンで再 ON | 6.3, 7.1 | — |
| 7 | 7.3 | キーバインド統合 | `ui.md` 6 章のキーバインドをフォーカスペインに応じて解釈、`Esc` を中断に | 3.6, 4.2 | — |
| 7 | 7.4 | ウィンドウタイトル動的更新 | 「mdpilot - <ファイル名>」を対象切替に応じて変更 | 6.3 | — |
| 7 | 7.5 | macOS メニューバー | mdpilot / ファイル / 表示 / ウインドウ / ヘルプ（`ui.md` 5.1） | 7.1, 7.3 | — |
| 7 | 7.6 | Windows ツールバー | 開く / 再読込 / 情報 を最小ツールバーで提供 | 7.1, 7.3 | — |
| 7 | 7.7 | パスバーとステータス表示 | プレビューファイルのフルパス、監視状態、claude 接続状態、エラートースト | 5.3, 6.3, 2.7 | — |
| 7 | 7.8 | テーマ追従 | OS のダーク/ライトに追従、コードブロックテーマも連動 | 4.3 | — |
| 7 | 7.9 | 非機能要件の測定 | N-01〜N-04 を測定、超過があれば最適化 | 全 Phase | — |
| 8 | 8.1 | macOS バンドル | `cargo-bundle` で `.app` 生成、`aarch64`/`x86_64` 両対応 | 7.* | — |
| 8 | 8.2 | Windows バイナリ | `x86_64-pc-windows-msvc` ターゲットでビルドスクリプト整備 | 7.* | — |
| 8 | 8.3 | CI（GitHub Actions） | macOS + Windows の build/test/clippy/fmt を回す | 0.1 | — |
| 8 | 8.4 | リリース手順ドキュメント | `docs/release.md` に手順記述 | 8.1, 8.2 | — |
| 9 | 9.1 | F-21 リンク・画像（相対パス含む）の解決の精緻化 | HTTP 画像対応、画像の自動リロード | 4.4, 4.5 | — |
| 9 | 9.2 | F-22 スクロール位置の編集追従 | 編集差分から該当位置にスクロール | 4.6 | — |
| 9 | 9.3 | F-23 設定ファイル | フォント・配色・キーバインド・ペイン比率・行数・モデル選択等 | 0.4 | — |
| 9 | 9.4 | F-24 アプリメニュー拡充 | macOS の環境設定、Windows の正式メニューバー | 7.5, 7.6 | — |
| 9 | 9.5 | F-25 複数チャット・複数プレビューのタブ | タブ UI、複数 session-id 管理 | 7.* | — |
| 9 | 9.6 | F-26 拡張: 数式・Mermaid・脚注 | KaTeX 相当・Mermaid・脚注を順次対応 | 4.2 | — |
| 9 | 9.7 | F-27 テーマ切替 | OS 追従に加え強制ライト/ダーク選択 | 7.8 | — |
| 9 | 9.8 | F-09 案 B（stream-json `tool_use` 解釈） | claude の `tool_use` から `file_path` 抽出 → 編集前にプレビュー対象を切替 | 6.3 | — |
| 9 | 9.9 | F-09 案 C（MCP サーバ） | mdpilot を MCP サーバとして公開、`mdpilot__open` などのツールを claude から呼べる | 6.3 | — |
| 9 | 9.10 | F-28 安全モード（パーミッション GUI モーダル） | `--dangerously-skip-permissions` を外し、ツール許可要求をモーダルで都度確認 | 3.4 | — |
| 9 | 9.11 | プレビュー内検索（`Cmd+F`） | プレビュー側の文字列検索とハイライト | 4.2 | — |
| 9 | 9.12 | チャット内検索 | チャット履歴内の検索 | 3.2 | — |
| 9 | 9.13 | ペイン比率の永続化・前回ウィンドウ位置復元 | 設定ファイル経由 | 9.3 | — |

## 3. 各タスクの詳細

### Phase 0 / 0.5（完了済み）

`docs/spike-report.md` および各コミット履歴を参照。Phase 0.5.1（egui_term）は方針転換で廃止、`spike/egui_term/` はリファレンスとして git 履歴に残す。

### Phase 1: 2 ペインレイアウト (F-01)

#### タスク 1.1: レイアウト状態とペイン分割
- **対象ファイル**: `src/ui/mod.rs`（新規）, `src/ui/layout.rs`（新規）, `src/app.rs`（更新）
- **作業内容**: `LayoutState { left_ratio: f32 }` を `App` に持たせ、`egui::SidePanel` で左右分割。最小幅 `240.0`。
- **参考パターン**: egui の `SidePanel`、`ui.md` 3 章。

#### タスク 1.2: 境界リサイズハンドル
- **対象ファイル**: `src/ui/layout.rs`（更新）
- **作業内容**: マウスドラッグで比率変更（標準）、境界ダブルクリックで 1:1 リセット、`Cmd+\`/`Ctrl+\` でも 1:1 リセット（後で 7.3 で配線）。

#### タスク 1.3: プレビュー/チャットのプレースホルダ描画
- **対象ファイル**: `src/ui/preview_pane.rs`（新規）, `src/ui/chat_pane.rs`（新規）
- **作業内容**: 各ペインに「プレビュー未指定」「Claude 接続準備中…」のラベルを置く。

### Phase 2: claude 子プロセスとの IO 基盤 (F-02, F-11)

#### タスク 2.1: claude 子プロセスの起動
- **対象ファイル**: `src/chat/mod.rs`（新規）, `src/chat/session.rs`（新規）
- **作業内容**:
  - `Command::new("claude")` に `--print --input-format=stream-json --output-format=stream-json --include-partial-messages --dangerously-skip-permissions` を渡し、`Stdio::piped()` 3 本（stdin/stdout/stderr）で spawn
  - cwd はプロジェクトルート（Phase 6.1 で確定するまで `current_dir()`）
  - 環境変数 `MDPILOT_PROJECT_ROOT` を Phase 6.5 で付与する余地を残す
- **参考パターン**: `chat.md` 2 章、`claude-integration.md` 3 章。

#### タスク 2.2: stream-json 入力スキーマの実機検証
- **対象ファイル**: `docs/chat.md`（更新）, `spike/` または小 Rust テストでよい
- **作業内容**:
  - `claude --print --input-format=stream-json --output-format=stream-json` を起動し、いくつかの JSON 形式（例: `{"type":"user","content":"hello"}`, Anthropic API 風 `{"role":"user","content":[{"type":"text","text":"hello"}]}` 等）を試して受理される形式を特定
  - 結果を `docs/chat.md` 3.1 章に反映
- **判定基準**: ユーザーメッセージを送信して assistant 応答が返ってくる入力 JSON 形式を 1 つ確定。

#### タスク 2.3: stdout 読込スレッドと stream-json パーサ
- **対象ファイル**: `src/chat/stream.rs`（新規）, `src/chat/session.rs`（更新）, `src/events.rs`（新規）
- **作業内容**:
  - `BufRead::lines` で 1 行ずつ読む
  - `serde_json::from_str::<serde_json::Value>` で生 Value にし、`type` フィールドで分岐
  - 既知イベント（`system/init`, `stream_event/content_block_delta/text_delta`, `stream_event/content_block_start/tool_use`, `system/api_retry`, `result`）を `ChatEvent` enum に変換
  - 未知イベントは `tracing::warn!` でログのみ
  - mpsc::Sender<AppEvent> でメインスレッドに送る
- **参考パターン**: `chat.md` 3.2 章。

#### タスク 2.4: stderr 読込スレッドと tracing 連携
- **対象ファイル**: `src/chat/session.rs`（更新）
- **作業内容**: stderr を別スレッドで `BufRead::lines` 読み、各行を `tracing::warn!` に流す。終了時にスレッドを join。

#### タスク 2.5: stdin 書込み
- **対象ファイル**: `src/chat/session.rs`（更新）
- **作業内容**:
  - 入力欄から送信ボタンが押されたら、`chat.md` 3.1 章で確定した JSON 形式を `serde_json::to_writer` で書く
  - 改行 `\n` を最後に付ける
- **参考パターン**: `chat.md` 4.2 章。

#### タスク 2.6: session-id ディスクストア
- **対象ファイル**: `src/chat/session_store.rs`（新規）, `src/config/paths.rs`（更新）
- **作業内容**:
  - `data_dir/sessions.json` を atomic write（一時ファイル + rename）
  - 構造は `chat.md` 5.1 章のスキーマ
  - `entries[project_root]` の get/set
- **参考パターン**: `chat.md` 5 章。

#### タスク 2.7: 子プロセスのライフサイクル
- **対象ファイル**: `src/chat/session.rs`（更新）
- **作業内容**:
  - `Child::try_wait` で終了監視
  - 正常終了（exit 0）/ 異常終了（exit != 0）を chat ペインに表示
  - mdpilot 終了時に SIGTERM → 数秒待って SIGKILL（Windows は `kill()` のみ）
- **参考パターン**: `chat.md` 2.4 章。

### Phase 3: チャット UI (F-03, F-04, F-05, N-07)

#### タスク 3.1: チャット UI の枠
- **対象ファイル**: `src/chat/view.rs`（新規）, `src/chat/history.rs`（新規）, `src/ui/chat_pane.rs`（更新）
- **作業内容**:
  - 上半分: `egui::ScrollArea::vertical` でメッセージ履歴
  - 下半分: `egui::TextEdit::multiline` + 送信/中断ボタン
  - `ChatHistory` 構造体で `Vec<ChatMessage>` を保持、`ChatMessage` は `User { text }` / `Assistant { id, text, tools: Vec<ToolBlock> }` / `System(SystemEvent)` などのバリアント
- **参考パターン**: `chat.md` 4 章。

#### タスク 3.2: テキストメッセージのストリーミング描画
- **対象ファイル**: `src/chat/history.rs`（更新）, `src/chat/view.rs`（更新）
- **作業内容**:
  - `ChatEvent::TextDelta { id, text }` を該当 Assistant メッセージに追記
  - ストリーミング中は末尾に `▌` のようなカーソル表示
  - `egui_commonmark` で Markdown としてレンダリング（cache 共有）
- **参考パターン**: `chat.md` 4.1 章。

#### タスク 3.3: ツール呼び出しの collapsible 表示
- **対象ファイル**: `src/chat/history.rs`（更新）, `src/chat/view.rs`（更新）
- **作業内容**:
  - `ChatEvent::ToolUse { id, name, input }` で `ToolBlock { name, input, output: None }` を該当 Assistant メッセージに追加
  - `ChatEvent::ToolResult { id, content }` で `output` を埋める
  - `egui::CollapsingHeader` で折りたたみ、既定は閉
- **参考パターン**: `chat.md` 4.1 章。

#### タスク 3.4: API リトライ・結果イベント表示
- **対象ファイル**: `src/chat/view.rs`（更新）
- **作業内容**:
  - `ChatEvent::ApiRetry { attempt, max_retries, error }` は chat 下部にバナー、retry 成功で消す
  - `ChatEvent::Result { subtype, .. }` で `subtype == "success"` 以外は赤系注釈
- **参考パターン**: `chat.md` 4.1 章。

#### タスク 3.5: 送信フローと IME 動作確認
- **対象ファイル**: `src/chat/view.rs`（更新）
- **作業内容**:
  - `Enter` で送信（`event.key_pressed(Key::Enter)`、Shift 押下時は改行）
  - `Shift+Enter` で改行
  - macOS / Windows 双方で日本語 IME 入力を実機検証
- **参考パターン**: `requirements.md` N-07、`chat.md` 4.2 章。

#### タスク 3.6: 中断ボタンの実装方式確定
- **対象ファイル**: `src/chat/session.rs`（更新）, `docs/chat.md`（更新）
- **作業内容**:
  - 候補: (a) claude プロセスに SIGINT、(b) `Esc` で stdin に何か送る（プロトコル未確定）、(c) stdin を `--max-turns` 風に閉じる
  - 実機検証で挙動を確定、`docs/chat.md` 10 章を更新
- **判定基準**: 進行中の応答を「人間が認知できる速度」で止められる手段が 1 つあれば OK。

#### タスク 3.7: メッセージ選択・コピー
- **対象ファイル**: `src/chat/view.rs`（更新）
- **作業内容**: egui の標準ドラッグ選択 + クリップボードコピーを使う。Markdown レンダリング部分の選択挙動を確認。

### Phase 4-9

`docs/plan.md` v1 と概ね同じ（preview / watcher / 自動追従 / UX 仕上げ / 配布 / MVP 後）。差分：
- Phase 7.3: `Esc` で中断（3.6 で確定した方式）を配線
- Phase 7.7: claude 接続状態の表示を追加
- Phase 9.10: F-28 安全モード（パーミッション GUI モーダル）を追加
- Phase 9.5: 複数チャット + 複数プレビューのタブ管理

詳細は各仕様書を参照。

## 4. 修正対象ファイル一覧

### 新規作成（Phase 1-7 で順次）

- `src/app.rs`（既存、Phase 1 以降に拡張）
- `src/events.rs`（新規、`AppEvent` enum）
- `src/ui/mod.rs`, `src/ui/layout.rs`, `src/ui/preview_pane.rs`, `src/ui/chat_pane.rs`
- `src/ui/menu_macos.rs`, `src/ui/toolbar_windows.rs`（Phase 7）
- `src/chat/mod.rs`, `src/chat/session.rs`, `src/chat/stream.rs`, `src/chat/history.rs`, `src/chat/view.rs`, `src/chat/session_store.rs`
- `src/preview/mod.rs`, `src/preview/loader.rs`, `src/preview/watcher.rs`, `src/preview/render.rs`
- `src/claude/mod.rs`（claude 起動引数組み立てヘルパ）
- `assets/Info.plist`, `scripts/build-macos.sh`, `scripts/build-windows.ps1`, `.github/workflows/ci.yml`
- `docs/perf.md`, `docs/release.md`

### 既存更新

- `docs/plan.md` — 本書（実装進捗に応じて更新）
- `README.md` — ステータス更新
- `Cargo.toml` — 段階的に依存追加（`serde`, `serde_json`, `notify`, `egui_commonmark`, `syntect`(transitive) など）

## 5. 検証方法

### テスト戦略

Phase 0 〜 1.4 は当初テストを書かずに進めていたが、ユーザー判断で **遡及的に unit test を追加**し、Phase 2 以降は **TDD 並行（テストを先または同時に書く）** で進める。

**書く対象**:

- ロジック層（純粋関数・I/O ラッパー）: 必ず unit test を書く
- 設定読込・パース・session_store のラウンドトリップ: 必ず unit test
- GUI 描画コード: `cargo test` での自動テストは難しいので、`cfg(debug_assertions)` の screenshot helper（`MDPILOT_DEBUG_SCREENSHOT`）で目視確認
- IO 系（stream-json パース・watcher デバウンス等）: 必ず unit test
- claude 子プロセスとの I/O: モック化可能な範囲で unit test、それ以外は実機検証 + screenshot

**書かない対象**:

- egui の API 呼び出し自体（egui が既にテスト済み）
- スパイク（`spike/*`）
- main 関数のセットアップコード（eframe::run_native の呼び出し方）

**遡及テスト（Phase 0 〜 1.4 で追加済み）**:

| ファイル | テスト数 | 対象 |
|------|------|------|
| `src/error.rs` | 2 | `Error::Io` の From 変換・Display |
| `src/config/paths.rs` | 1 | `AppPaths::resolve()` が `mdpilot` を含むパスを返す |
| `src/ui/fonts.rs` | 3 | `try_install_font` の存在しないパス・先頭挿入・末尾挿入 |
| `src/ui/layout.rs` | 2 | `reset()` が `PanelState` を削除する・状態不在時の no-op |
| 計 | 8 | |

### コマンドベース検証（各 Phase 共通）

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release
```

### Phase 別の手動検証

- **Phase 0 / 0.5**: 完了済み（`spike-report.md` 参照）
- **Phase 1**: ウィンドウが 1400x900 で開き、ペイン境界をドラッグして比率変わる、最小幅 240px
- **Phase 2**: `claude` 子プロセスが起動し、`system/init` を受信、session-id がディスクに保存される。stdin に最小ユーザーメッセージを送って assistant 応答が流れる
- **Phase 3**: chat 入力欄に日本語が IME で打てる、Enter で送信、ストリーミング表示、Edit ツールが collapsible で表示、`Esc` で中断
- **Phase 4**: `mdpilot README.md` でプレビュー描画、GFM / コードブロックハイライト、リンク / 画像
- **Phase 5**: 外部編集で再レンダリング、削除で「見つかりません」
- **Phase 6**: プロジェクト指定起動、別 `.md` 作成で自動切替
- **Phase 7**: `Cmd+O` で追従 OFF、メニュー / ツールバー日本語、テーマ追従、N-01〜N-04 を `docs/perf.md` に記録
- **Phase 8**: macOS `.app` / Windows `.exe` が起動、CI green

### 機能要件カバレッジ確認

実装完了時点で `requirements.md` の F-01〜F-11 をチェックリスト形式で全項目満たすことを確認。非機能要件は以下：

- **N-05（異常終了で編集データを失わない）**: mdpilot に `.md` 書込み経路が無いことを Phase 7 のアーキテクチャレビューで `grep` 等で確認
- **N-06（単一実行可能ファイル配布）**: Phase 8 のビルド・配布時点
- **N-07（IME 日本語入力）**: Phase 3.5 で実機確認
- **N-01〜N-04（性能要件）**: Phase 7.9 で測定し `docs/perf.md` に記録

## 6. 推奨する実行方法

本計画は **10 フェーズ・約 60 タスク**（うち Phase 0/0.5 の 7 タスク済）。以下を推奨する：

### 全体方針

- **`/team-manager` の使用を推奨**：フェーズ単位でマネージャーが進捗管理し、フェーズ内の独立タスクをサブエージェントに割り振る運用に適している
- 各タスク完了ごとに `git commit`、機能ごとに作業粒度とコミット粒度を合わせる（CLAUDE.md の指示）
- フェーズ完了ごとに `code-review` / `pair-review` でセルフレビュー → PR

### フェーズごとの推奨運用

| Phase | 推奨実行手段 | 理由 |
|-------|------------|------|
| 0 | （完了） | |
| 0.5 | （完了） | |
| 1 | `/implement-issue` または `/write-code` を直列で | レイアウトは逐次依存が強い |
| 2 | `/implement-issue` 直列、ただし 2.2（スキーマ検証）を最優先 | claude IO の地盤を 1 つずつ |
| 3 | `/team-manager`（一部並列） | UI 要素は独立。ただし 3.6 (中断方式確定) は実機検証の単独タスク |
| 4 | `/team-manager` | 4.1〜4.6 は描画基盤の上に独立タスクが乗る |
| 5, 6 | `/implement-issue` 直列 | 監視ロジックは順序依存が強い |
| 7 | `/team-manager` | UX 機能は独立性が高く並列化に向く |
| 8 | `/implement-issue` 直列 | ビルド整備は構成変更が衝突しやすい |
| 9 | フェーズ完了後にユーザー需要に応じて issue 起票 | 拡張は需要ドリブンで進める |

### 着手前に確定すべき項目（実装着手時に決める）

仕様書の「未確定事項」から本計画に影響するもの。**いずれも仕様書では未確定**であり、エージェント側で勝手に確定させず、対応フェーズの着手前にユーザー判断を仰ぐ。

| 項目 | 関連タスク | エージェント暫定案 | 仕様書の根拠 | 確定タイミング |
|------|----------|------------------|------------|--------------|
| stream-json 入力スキーマ | 2.2 | 実機テスト後に確定 | `chat.md` 3.1 章 | Phase 2 のスパイク中 |
| 中断ボタンの実装方式 | 3.6 | 実機テスト後に確定 | `chat.md` 10 章 | Phase 3 のスパイク中 |
| `egui_commonmark` の GFM カバレッジと採用継続可否 | 4.2 | Phase 0.5.2 で問題なし、Phase 1 着手前の実機目視待ち | `preview.md` 3 章, `spike-report.md` | Phase 1 着手前 |
| `tokio` 採用可否 | 全 Phase | `std::thread` で開始 | `architecture.md` 4, 9 章 | 必要が出たら再評価 |
| 設定ファイル形式 (TOML / JSON) | 9.3 | （Phase 9 で検討） | `architecture.md` 9 章 | Phase 9.3 着手前 |
| syntect ダーク/ライトテーマ名 | 4.3 | `preview.md` 4 章の例示値 | `preview.md` 4 章 | Phase 4 着手前 |
| アプリアイコン | 8.1 | 未準備 | `requirements.md` 8 章 | Phase 8 着手前 |
| ライセンス | 8.4 | ユーザー判断 | `requirements.md` 8 章 | Phase 8 完了前 |
| プロジェクト選択ダイアログ UI 仕様 | 6.1 | `rfd` のディレクトリ選択 1 回 | `claude-integration.md` 2 章 | Phase 6 着手前 |
| F-28 安全モードの UI 設計 | 9.10 | 後日設計 | `requirements.md` F-28 | Phase 9 着手前 |

各項目は対応フェーズの最初のサブタスクとして **ユーザー確認のステップ** を置く。
