# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## プロジェクト現在地（2026-06-02）

- mdpilot は Rust + eframe (egui) のネイティブ GUI アプリ。左ペインに Markdown プレビュー、右ペインに **chat UI**（`claude` CLI を子プロセスとして spawn し、`stream-json` で対話）
- 内蔵ターミナルエミュレータは持たない。`egui_term` / `alacritty_terminal` / `portable-pty` 系は 2026-05-29 のユーザー判断で廃止（`spike/egui_term/` は履歴として残る）
- 設計フェーズ完了 → 実装フェーズ。`docs/plan.md` 対応一覧表で各タスクの状態（`✓` 完了 / `✗` superseded / `—` 未着手）を管理
- 完了: Phase 0.1〜0.4, 0.5.2/0.5.3, 仕様改訂, 2.0〜2.7, 1.1〜1.4, 3.1〜3.7（Phase 3 完了）
- 次の着手対象: **Phase 4.1（ファイル読込ローダー）** からプレビュー側に着手
- Phase 3.5 の残課題（いずれもコード反映済みだが手動検証必要）:
  - IME 実機確認（N-07）: plain Enter のみ `chat::view::extract_send_enter` で event queue から除去、Shift+Enter は TextEdit に通す。egui の input contract 上 IME composition 中の Enter は届かないので別途 composing guard 不要。macOS / Windows での実機タイピング検証はユーザー操作必須
  - end-to-end の手動検証: spawn 経路は debug screenshot で確認済み（プレースホルダが正しく描画、SpawnFailed バナーなし）だが、user → claude → assistant message → ストリーミング描画 のフルフローは外部 process テスト禁止のためコードでは担保していない。実機で動作確認が必要
- Phase 3.6 の決定: claude CLI 2.1 にリアルタイム中断手段が無いことを公式 docs / GitHub issue #41665 で確認。MVP は 中断 ボタンを `add_enabled(false, ...)` で常時 disabled、tooltip で説明。claude CLI に `{"type":"interrupt"}` が実装されたら Phase 9 で有効化（フックは `src/chat/view.rs` に残置）
- Phase 3.6 の残課題: disabled ボタン + on_disabled_hover_text の見た目はコード上は egui 0.34 の正しい API（clippy / test green）だが、`MDPILOT_DEBUG_SCREENSHOT` の 2 回目以降ハング問題で実機目視は未確認。Phase 3.7 / 4.1 着手時にスクリーンショット問題と合わせて検証する

## 開発コマンド

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test                  # 59 件パス
cargo build                 # debug
cargo build --release

# 単一テスト
cargo test chat::stream::tests::parses_text_delta -- --exact
```

## debug screenshot helper

`src/app.rs` の `cfg(debug_assertions)` ブロックに、起動 30 フレーム後に viewport をスクリーンショットしてプロセスを正常終了するヘルパが入っている。release ビルドでは消える。

```sh
MDPILOT_DEBUG_SCREENSHOT=/tmp/mdpilot.png cargo run
```

GUI の自動テストは egui の上に薄く、これが現状唯一の描画回帰確認手段。Phase 1.4 で「日本語が tofu 化」、Phase 3.1 で「chat UI レイアウト確認」がこれで検出/確認できた。同じパターンが Phase 3.5 以降でも使える。重要: `Drop` で `ViewportCommand::Close` を送るので、Phase 3.5 以降で claude 子プロセスを App に持たせても孤児化しない（`std::process::exit` ではない）。

## ブランチと git 運用

- 現在のブランチ `worktree-dapper-mapping-rainbow` で作業中、`main` への直接コミット運用（feature ブランチを切らない、ユーザー指示）
- 「機能ごとに作業粒度とコミット粒度を合わせる」「こまめにコミット」（ユーザー指示・グローバル CLAUDE.md）
- force push 禁止、`--no-verify` などフック skip 禁止

## アーキテクチャ要点

### スレッド構成

| スレッド | 役割 |
|---|---|
| メイン | eframe イベントループ、UI 描画、claude stdin への書込み（`logic()` で mpsc を drain → `ChatHistory::apply`） |
| BG | claude stdout 読込 → `serde_json::Value` パース → `ChatEvent` → mpsc に流す + `egui::Context::request_repaint` で UI 起動（`ChatSession::start` で spawn）|
| BG | claude stderr 読込 → `tracing::warn`（`target = "claude::stderr"`） |
| BG（Phase 5+） | `notify` ファイル監視 |

### `src/chat/` モジュール

- `session.rs` — `ChatSession`（child process owner）、`SpawnOptions`、`build_args` (pure fn)、`send_user_message`、`pipe_lines_to_tracing`。`ChatSession::start(opts, events_tx, wake_ui)` で stdout drain thread を spawn し `ChatEvent` を mpsc に流す。`Drop` は **stdin drop → SIGTERM → 500ms 待機 → SIGKILL**（Unix）/ Windows は `Child::kill()` → stdout join → stderr join（順序重要：child exit 前に join すると hang）。`libc` は `[target.'cfg(unix)'.dependencies]`
- `stream.rs` — `ChatEvent` enum + `parse_event(&Value)` + `pipe_stdout_to_channel<R: BufRead, F: Fn()>`。**typed `serde::Deserialize` は使わない**（`system/init` に 20+ フィールドあり、claude バージョンで増減するため `Value::get(...)` 抽出のみ）。`wake` は send 成功ごとに呼ばれる（App は `ctx.request_repaint`）
- `history.rs` — `ChatHistory` (Vec<ChatMessage> + input)、`apply(ChatEvent)` で TextDelta/AssistantMessage/ToolUse/ApiRetry/Result を内部状態にマップ。`SystemMessage` には `ApiRetry / ResultError / Disconnected / SpawnFailed`
- `session_store.rs` — `<data_dir>/sessions.json` の atomic write（tmp + rename）。プロジェクトルートごとに session-id を永続化、chrono タイムスタンプ付き。**App には未配線**（Phase 6.1 で `current_dir()` → 正式 project root 解決と同時に統合予定）
- `view.rs` — chat pane の描画（メッセージ履歴 + 入力欄 + 送信ボタン + 中断ボタン（常時 disabled）+ tool collapsible block）。`show(ui, history, session_alive, on_send)` で送信意図を `App` にコールバック。Plain Enter は `extract_send_enter(&mut Vec<Event>) -> bool` で event queue から取り除いて submit を発火、Shift+Enter は素通しで改行。Label の selectable は `header_label` (User/Assistant/Input/Output 等の構造マーカー) で false、`body_label` と system error label で true を明示（egui 0.34 の `LabelSelectionState` plugin が `Cmd+C`/`Ctrl+C` → OS clipboard をハンドル）

### `src/ui/` モジュール

- `layout.rs` — `egui::Panel::left("preview_pane") + CentralPanel`。境界 8px の hit strip で double-click → `reset()` で `PanelState` を memory から削除（**実機動作は未確認**、Phase 7.3 で `Cmd+\` 配線時に検証予定）
- `fonts.rs` — macOS: `/System/Library/Fonts/AquaKana.ttc`（ひらがな/カタカナ）+ `Hiragino Sans GB.ttc`（CJK 漢字）を動的ロード。Hiragino Sans 日本語版は最近の macOS では on-demand で標準パスに無いため、これが現状の妥協点。Windows は Phase 8 で対応
- `preview_pane.rs` / `chat_pane.rs` — 各ペインのトップレベル（中身はそれぞれ preview/chat モジュールに委譲）

### claude CLI コントラクト（Phase 2.0/2.2 で実機確認、`docs/chat.md` §2-3 / `docs/spike-report.md` に記録）

起動コマンド（必須）:

```
claude --print --verbose \
  --input-format=stream-json --output-format=stream-json \
  --include-partial-messages --dangerously-skip-permissions \
  --session-id <uuid> [--continue]
```

- `--verbose` は `--print + --output-format=stream-json` の **必須前提**（公式ヘルプには未記載）
- `--session-id <新規 UUID>` は新規セッションを作る（既存要求ではない）
- `--include-partial-messages` 無しでは `text_delta` は流れず、`assistant` イベントが完了時に 1 行で来る

入力（mdpilot → claude、Phase 2.2 で確定）:

```json
{"type":"user","message":{"role":"user","content":"<text>"}}
```

出力イベント（Phase 2.0/2.2 で実機観測した順）:
`system/hook_started` → `system/hook_response` → `system/init` → `assistant`（完全メッセージ） or `stream_event/content_block_delta/text_delta`（partial 時）→ `rate_limit_event` → `result`

## 重要な仕様判断（変更時はユーザー確認）

- パーミッション: MVP は `--dangerously-skip-permissions` で全スキップ、安全モードは F-28 で MVP 後
- セッション: 1 ウィンドウ = 1 セッション、プロジェクトルートごとに session-id を保存して `--continue` 再開
- ツール呼び出し表示: `CollapsingHeader` で折りたたみ
- F-09（プレビュー対象の追従）: 案 A（`notify` 経由のファイルシステムイベントのみ）。stream-json `tool_use` 解釈（案 B）と MCP（案 C）は MVP 後
- 日本語フォント: macOS は AquaKana + Hiragino Sans GB の組み合わせ。Hiragino Sans 日本語版は標準パスに無い
- chat UI 路線: `claude` を子プロセスとして JSON Lines 経由でやり取り。ターミナルエミュレータは持たない

## テスト戦略

`docs/plan.md` §5 にも記載。

- ロジック層（pure fn / I/O ラッパー / パーサ / store） → unit test 必須
- GUI 描画 → `MDPILOT_DEBUG_SCREENSHOT` で目視 + 描画ロジックは `cfg(test)` で `egui::Context::default()` を作って状態 assert
- 外部プロセス（claude）依存テストは書かない（CI で動かない）。ChatSession::start の挙動は実機確認で担保
- macOS 固有テスト（フォント等）は `#[cfg(target_os = "macos")]` でガード

## docs/ の優先読書順

PR・仕様確認・新タスク着手前に読む順:

1. `docs/plan.md` — 全体実装計画、現在地、次タスク、ユーザー確定済みの判断
2. `docs/chat.md` — chat UI / claude プロトコル / セッション仕様（Phase 2/3 の核）
3. `docs/architecture.md` — モジュール構成・スレッドモデル
4. `docs/requirements.md` — `F-XX` 番号と要件
5. `docs/preview.md` — プレビュー仕様（Phase 4-5 の核）
6. `docs/claude-integration.md` — 自動追従（F-09）、プロジェクトルート
7. `docs/ui.md` — ペイン構成・キーバインド・テーマ
8. `docs/spike-report.md` — Phase 0.5 + 2.0 + 2.2 のスパイク実機結果

## 未確認 / 棚上げ事項

- ダブルクリックでのペイン 1:1 リセット動作（`src/ui/layout.rs::reset`）が egui Panel の resize drag と入力競合しないか — 実機目視未確認、Phase 7.3 で確認予定
- `--include-partial-messages` オン時の `stream_event/text_delta` 順序の詳細
- IME 入力中の Enter 挙動（kana → kanji の確定 Enter が UI 側に届かないこと、確定後の Enter が送信を発火すること）— macOS / Windows で実機タイピング検証が必要（Phase 3.5 の残課題、N-07）
- Windows での実機検証全般（Phase 8）

## グローバル CLAUDE.md からの確認指示（要約）

- 日本語で答える
- 不明確な情報は推論・憶測せず「わからない」と答える、裏付けできない情報を返答しない
- 判断には根拠を添える（コード参照・公式ドキュメント URL・issue/PR/仕様書・会話中のユーザー指示）
- 指示が曖昧な場合は自分で解釈せずユーザーに確認
- テストが通らないことを理由にテストを skip/disable/削除して green にすることは禁止
