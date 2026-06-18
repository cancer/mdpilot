# Chat UI 仕様

## 1. 概要

右ペインに配置する Claude Code 連携の chat UI。mdpilot は `claude` CLI を **非対話モード**で子プロセスとして spawn し、stdin/stdout 上の JSON Lines プロトコル（stream-json）でメッセージをやり取りする。ターミナルエミュレータは持たない。

### 1.1 採用判断

| 判断項目 | 採用 | 根拠 |
|---|---|---|
| セッションモデル | 1 セッション / ウィンドウ。次回起動時は `--continue` で再開 | ユーザー判断、`docs/plan.md` Phase 0.5 のユーザー回答 |
| ツール呼び出しの表示 | Collapsible ブロック（ツール名 + 入力 + 出力） | 同上 |
| パーミッション | `--dangerously-skip-permissions` で全許可（MVP） | 同上。MVP 後の安全モードは `requirements.md` F-28 |
| 履歴永続化 | プロジェクトルートごとに session-id を 1 つディスク保存、`--continue` で再開 | 同上 |

## 2. claude 子プロセスの起動

### 2.1 起動コマンド

```
claude
  --print
  --verbose
  --input-format=stream-json
  --output-format=stream-json
  --include-partial-messages
  --dangerously-skip-permissions
  --session-id <uuid> [--continue]
  [--model <model>]
```

| オプション | 役割 |
|---|---|
| `--print` | 非対話モード（標準入出力で JSON ストリーミング） |
| `--verbose` | **`--output-format=stream-json` と `--print` を併用するときに必須**（エラー: "When using --print, --output-format=stream-json requires --verbose"。Phase 2.0 で実機検証） |
| `--input-format=stream-json` | 標準入力を stream-json として受け付け |
| `--output-format=stream-json` | 標準出力を stream-json として吐き出し |
| `--include-partial-messages` | 部分メッセージ（`stream_event` 系）を含める。これが無いとテキストは `assistant` イベントとしてメッセージ完了時に 1 行で流れ、ストリーミング表示にならない |
| `--dangerously-skip-permissions` | ツール呼び出しの許可ダイアログをスキップ。MVP の前提 |
| `--continue` | 同一 cwd の直近セッションを再開（`--session-id` と併用すると特定セッションを継続） |
| `--session-id <uuid>` | セッション ID を明示。**新規 UUID を渡すと新規セッションを作る**（Phase 2.0 で実機検証） |

### 2.2 起動時の cwd と環境変数

| 項目 | 値 |
|---|---|
| cwd | プロジェクトルート（`claude-integration.md` 2 章） |
| 環境変数（追加） | `MDPILOT_PROJECT_ROOT=<絶対パス>` |
| 環境変数（継承） | mdpilot プロセスの環境変数を継承（`PATH` 等） |
| 標準入出力 | パイプ（OS の標準 `Stdio::piped()` 相当） |

### 2.3 起動時のセッション選択

Phase 2.0 で実機検証した内容を反映：

1. プロジェクトルートと session-id の対応をディスクから読む（`session_store`、後述 5 章）
2. 対応がある場合: `--session-id <existing-uuid> --continue` を付けて起動
3. 対応が無い場合: 新規 UUID（`uuid` クレートで v4 生成）を作り、`--session-id <new-uuid>` のみを付けて起動。claude は新規セッションを作り、`system/init` イベントで同じ UUID を session_id として返してくる。mdpilot はこの session_id をディスクに保存
4. 起動に失敗した場合: chat ペインにエラー表示、ユーザーが手動で「再接続」できるボタンを出す（MVP の最小エラー UI）

### 2.4 子プロセス終了時の挙動

- `claude` プロセスが正常終了（exit 0）した場合: 「Claude セッションが終了しました」を chat ペインに表示。再接続ボタン
- 異常終了（exit != 0）: stderr の最後の数行と exit code を表示。再接続ボタン
- mdpilot 終了時: 子プロセスを `kill`（macOS/Linux は SIGTERM → 数秒後 SIGKILL、Windows は `Child::kill()` 相当）

## 3. stream-json プロトコル

`claude --output-format=stream-json` は JSON Lines 形式で標準出力にイベントを流す。**入力側のスキーマは公式ドキュメントに未文書化**であり、Phase 2 着手時に実機検証で確定する。

### 3.1 入力（mdpilot → claude）

**確定スキーマ（Phase 2.2 で実機確認、2026-06-01）**:

1 メッセージ = 1 JSON Lines（行末 `\n`）：

```json
{"type":"user","message":{"role":"user","content":"say hi in 3 words"}}
```

| フィールド | 値 |
|----------|----|
| `type` | 文字列 `"user"`（claude 側は他の type も受け付ける可能性があるが mdpilot が送るのは user のみ） |
| `message.role` | 文字列 `"user"` |
| `message.content` | 文字列（プレーンテキスト）。実機テストで string 形式が受理されることを確認 |

実機ログ（抜粋）: 上記 JSON を stdin に流すと claude が assistant 応答（`{"type":"assistant","message":{"content":[{"type":"text","text":"Hi there, friend!"}], ...}}`）を返した。`docs/spike-report.md` に詳細。

**未検証（Phase 3 着手時に必要なら調査）**:

- Anthropic API 風の配列 content `[{"type":"text","text":"..."}]` 形式が受理されるか
- 添付画像（`{"type":"image","source":...}`）対応
- `parent_tool_use_id` などの拡張フィールド

### 3.2 出力（claude → mdpilot）

**パーサ実装方針（重要）**: `system/init` イベントは実機で 20 以上のフィールド（`cwd`, `tools`, `mcp_servers`, `model`, `permissionMode`, `slash_commands`, `apiKeySource`, `claude_code_version`, `output_style`, `agents`, `skills`, `plugins`, `analytics_disabled`, `uuid`, `memory_paths`, `fast_mode_state` 等）を持ち、claude のバージョンとともに増減する。**型付き `serde::Deserialize` の `struct` は使わず**、`serde_json::Value` の `.get(...)` 抽出で mdpilot が実際に使うフィールド（`session_id` など 5〜6 個）のみを取り出す。これにより claude が新フィールドを足しても壊れない。

Phase 2.0 の実機検証および公式ドキュメント（agent-sdk/streaming-output.md, headless.md, CLI reference）から確認できているイベント：

実機で観測した順序（`--include-partial-messages` 無し）：
1. `system/hook_started`（SessionStart hook 起動）
2. `system/hook_response`（同上完了）
3. `system/init`（session_id, cwd, tools, mcp_servers, model, plugins などの起動情報）
4. `assistant`（完全な assistant メッセージ 1 行、`message.content` 配列に `{type: "text", text: ...}` や `tool_use` を含む）
5. `rate_limit_event`（レート制限ステータス）
6. `result`（`subtype: "success"`, `terminal_reason: "completed"`, `duration_ms`, `total_cost_usd` などを含む完了マーカー）

`--include-partial-messages` を付けると、`assistant` の代わりに / に加えて `stream_event/content_block_delta/text_delta` 等の差分イベントが流れる（公式ドキュメント記載、Phase 2 で実機検証予定）。

#### 3.2.1 `system / init`（初期化）

```json
{
  "type": "system",
  "subtype": "init",
  "session_id": "<uuid>",
  "plugins": [],
  "plugin_errors": []
}
```

mdpilot はこの `session_id` を `session_store` に保存し、以後 `--continue` の対象とする。

#### 3.2.2 `stream_event / content_block_delta / text_delta`（テキスト断片）

```json
{
  "type": "stream_event",
  "event": {
    "type": "content_block_delta",
    "delta": { "type": "text_delta", "text": "..." }
  },
  "session_id": "<uuid>",
  "uuid": "<msg-uuid>"
}
```

mdpilot は同一 `uuid` のテキストを連結して assistant メッセージを構築する。

#### 3.2.3 `stream_event / content_block_start / tool_use`（ツール使用開始）

```json
{
  "type": "stream_event",
  "event": {
    "type": "content_block_start",
    "content_block": {
      "type": "tool_use",
      "name": "Edit",
      "id": "<tool-id>",
      "input": { /* ツール固有 */ }
    }
  }
}
```

mdpilot は新しい collapsible ブロックを作り、`name` と `input` を表示する。

#### 3.2.4 ツール結果（推定）

公式ドキュメントに明示はないが、Claude API の `tool_result` メッセージタイプに対応する出力イベントが流れる想定。Phase 2 着手時に実機で確認し、本書を更新する。

#### 3.2.5 `system / api_retry`（API リトライ）

```json
{
  "type": "system",
  "subtype": "api_retry",
  "attempt": 1,
  "max_retries": 3,
  "retry_delay_ms": 1000,
  "error_status": 429,
  "error": "rate_limit"
}
```

mdpilot は chat ペインの最下部にリトライ表示を出す（「rate_limit でリトライ中… 1/3」）。

#### 3.2.6 `result`（完了）

```json
{
  "type": "result",
  "subtype": "success",
  "result": "...",
  "session_id": "<uuid>",
  "total_cost_usd": 0.05
}
```

`subtype` は `success` / `error_max_turns` / `error_max_budget_usd` などがあり得る。mdpilot は `subtype != success` のとき chat に注釈を出す。

#### 3.2.7 終了マーカー

`result` イベント以降にイベントが流れない、を「ストリーム終了」と解釈する。公式ドキュメントに明示の終了マーカーは記載なし。Phase 2 で実機検証して必要なら本書を更新する。

### 3.3 スキーマ安定性

stream-json は公式に「安定した公開契約」と明言されていない。mdpilot は以下で防御する：

- パース失敗（未知の `type` / `subtype`）を**無視せず**ログに残し、ユーザーには「未対応のイベントを受信しました」と通知
- 既知イベントだけを描画する。未知は無視
- `claude --version` を起動時に取得し、ログに記録（再現用）
- claude のメジャー更新後は本書および実装の追従が必要

## 4. UI レイアウト

```
┌──────────────────────────────────────────┐
│ Chat ペイン                              │
├──────────────────────────────────────────┤
│                                          │
│  メッセージ本文…                          │ ← Assistant: 装飾なし高コントラスト本文
│                                          │
│  ⚙ Edit ▽                                │ ← Collapsible tool block (折りたたみ状態)
│                                          │
│  ┌──────────────────────────────────┐   │ ← User: tint Frame + 控えめ色 (Phase 10.18)
│  │ もっと詳しく説明して               │   │
│  └──────────────────────────────────┘   │
│                                          │
│  ストリーミング中の Assistant 本文…       │
│                                          │
│  ⚙ Bash ▽                                │ ← 展開時
│    Input: {"command": "ls docs/"}        │ ← 引数 (input_json_delta accumulate)
│    Output: chat.md  preview.md  ...      │ ← user 側 tool_result から
│                                          │
├──────────────────────────────────────────┤
│ ┌────────────────────────────────────┐   │
│ │ プロンプトを入力…                   │   │ ← 入力欄 (egui::TextEdit, 複数行)
│ └────────────────────────────────────┘   │
│                              [中断]      │ ← in_flight 時のみ表示 (Phase 10.25)
└──────────────────────────────────────────┘
```

### 4.1 メッセージ表示

| 種別 | 描画 |
|---|---|
| Assistant テキスト | プレーンテキストとして高コントラスト色 (`body_color`) で描画。見出しなし。ストリーミング中は `text_delta` が逐次 append される |
| User テキスト | tint Frame (`user_bubble_bg`) で囲み、本文は muted 色 (`user_text_color`)。見出しなし — Frame そのものが speaker 標識 (Phase 10.18 / 10.24) |
| Tool 呼び出し | Collapsible ブロック：ヘッダーに `⚙ <tool name>`、展開で `input` (JSON、`input_json_delta` で組み立て) と `output` (`tool_result` 由来) を表示 |
| API リトライ | 黄系ステータス行：「API リトライ中: <error> (1/3)」 |
| `result.subtype != success` | 赤系注釈：「Claude のレスポンスがエラーで終了しました: <subtype>」。abort 時は subtype = `aborted_by_user` |
| `Disconnected` | 子プロセスが切断したとき。赤系注釈 |
| `SpawnFailed` | claude CLI が見つからない / 起動失敗。赤系注釈 + 案内文 (PATH 解決の指示) |
| `StderrError` (Phase 10.19) | `error`/`fatal`/`panic`/`unauthor` を含む claude stderr 行を `claude stderr: ...` 接頭辞付きで赤系表示 |

### 4.2 入力欄

- `egui::TextEdit::multiline` をベース、`Enter` で送信、`Shift+Enter` で改行
- 送信トリガは `extract_send_enter` で `ui.add_sized` の **前** に Enter を pre-consume する。TextEdit がそもそも Enter を見ないので改行混入を防ぐ (Phase 10.21)
- 送信ボタンは廃止 (Phase 10.25) — Enter 専用。`in_flight` でも送信可能だが、内部的には現 turn を abort してから新 turn を開始する形 (`aborted_by_user` system marker が挟まる)
- 「中断」ボタンは `in_flight` のときだけ右に表示。`Esc` (chat focus 中) でも同じ動作 (Phase 10.14)
- IME（日本語入力）は `egui::TextEdit` の標準サポートに委ねる

### 4.3 スクロール

- chat 履歴は縦スクロール。`ScrollArea::vertical().stick_to_bottom(true)` で末尾追従
- 送信時は `ChatHistory.scroll_to_bottom_pending` one-shot フラグ → ScrollArea closure 内で `scroll_to_cursor(Align::Max)` を呼び、過去履歴を遡って見ているときに送信しても自分のメッセージが見えるよう強制ジャンプ (Phase 10.26)
- 履歴は仮想化しない（MVP）

### 4.4 コピー

- ドラッグ選択 + `Cmd+C`/`Ctrl+C` で OS クリップボードにコピー
- メッセージ単位のコピーボタンは MVP 後

## 5. セッション ID の永続化（session_store）

### 5.1 保存形式

- 保存先: `config::paths::AppPaths::data_dir / "sessions.json"`
- フォーマット:

```json
{
  "version": 1,
  "entries": {
    "/Users/cancer/projects/blog": {
      "session_id": "0123abcd-...",
      "claude_version": "1.x.y",
      "last_used": "2026-05-29T12:34:56Z"
    },
    "/Users/cancer/projects/docs": { ... }
  }
}
```

### 5.2 ライフサイクル

| タイミング | 動作 |
|---|---|
| mdpilot 起動 | `entries[<project_root>]` を読む。あれば `--session-id` + `--continue`、なければ新規 UUID 生成 |
| `system/init` 受信 | レスポンスの `session_id` で `entries` を上書き、`last_used` を更新 |
| mdpilot 終了 | `entries` を atomic write（一時ファイル → rename） |

### 5.3 不一致時の挙動

- ディスク保存の session-id と claude が返した session-id が異なる: claude が新規セッションを開始したと判断、ディスクを上書き
- セッションファイル破損: 警告ログを出し、`entries = {}` で再スタート

## 6. パーミッション（MVP）

- 起動時に `--dangerously-skip-permissions` を付ける。claude のあらゆるツール呼び出しは自動許可
- ユーザーは Markdown 編集中心の用途を想定し、Bash 等を含む全ツールを許可する責任を引き受ける（`requirements.md` 7 章）
- MVP 後（F-28）にモーダルで都度許可する安全モードを追加し、`--dangerously-skip-permissions` をオプション化する

## 7. キーバインド

| キー | アクション |
|---|---|
| `Enter`（入力欄フォーカス時） | 送信 (in_flight でも可、mid-stream の場合は abort + 新 turn) |
| `Shift+Enter` | 入力欄に改行を挿入 |
| `Esc`（応答中） | turn を abort (Phase 10.14) |
| `Cmd+C` / `Ctrl+C` | 選択範囲をコピー (egui 既定の OS クリップボード経由) |

`ui.md` 6 章の全体キーバインドと整合する。

## 8. テーマ・フォント

- 入力欄・本文ともに `ui.md` 8 章のテーマ追従に合わせる
- フォントは `egui` の既定（日本語フォールバックも `egui` 設定）

## 9. エラー処理

| 状況 | UI |
|---|---|
| `claude` バイナリが PATH に無い | `SystemMessage::SpawnFailed` を chat に表示 (PATH 解決の案内付き)。再接続は新規タブ起動で行う |
| claude 子プロセスが切断 (stdout EOF / 子プロセス死) | `SystemMessage::Disconnected` を chat に表示 |
| claude stderr に error 系の行 | `SystemMessage::StderrError` で chat に赤字表示 (`claude stderr: <line>`)。フィルタは `error`/`fatal`/`panic`/`unauthor` を含む行 (Phase 10.19) |
| stream-json パース失敗 / 未知イベント | tracing にログ (`claude::stdout` target) |
| `system/api_retry` | 4.1 参照 |
| `result.subtype != success` | 4.1 参照 (`aborted_by_user` を含む) |
| ネットワーク断 (API タイムアウト) | claude 側で `api_retry` が走る想定。retry 上限超過は `result.subtype != success` で表現される |

## 10. 既知の制限と未確定事項

### 10.1 中断機構 (Phase 10.14)

Phase 3.6 では「claude CLI 2.1 に protocol interrupt がない」ことを理由に中断 UI を永続 disabled としていたが、Phase 10.14 で **kill + `--resume` で再生成する迂回路** を実装した。フローは以下:

1. ユーザーが Esc / 中断ボタンを押す
2. `Tab::abort_current_turn` が子プロセスを drop → Drop が SIGTERM (timeout → SIGKILL)
3. `SystemMessage::ResultError { subtype: "aborted_by_user" }` を chat 履歴に push
4. 同じ session-id で `--resume` で spawn し直し、続きの会話ができる状態にする

副作用:

- 進行中の tool 呼び出しは強制終了 (state を失う)
- 再 spawn まで一瞬空白がある
- `aborted_by_user` の system marker が履歴に残る

これは「リアルタイム interrupt」ではなく「turn 強制終了 + 再 spawn」だが、UX としては interrupt 相当に機能する。claude CLI が `{"type":"interrupt"}` を stdin で受け付けるようになったら protocol レベルの interrupt に置き換える。

調査ソース:

- [Claude Code Streaming Input docs](https://code.claude.com/docs/en/agent-sdk/streaming-vs-single-mode)
- [GitHub anthropics/claude-code#41665 — Support an interrupt message on stdin](https://github.com/anthropics/claude-code/issues/41665) (CLOSED, duplicate)

### 10.2 mid-stream 送信 (Phase 10.25)

`in_flight` 中に Enter を押した場合、内部的には:

1. `Tab::abort_current_turn` で現 turn を kill + `--resume` で再 spawn
2. 直後に新規メッセージを `handle_send` で送る

`aborted_by_user` system marker が turn1 と turn2 の間に挟まる。message_id 単位での delta routing は未実装なので、複数 turn を真の意味で並行ストリーミングはできない。

### 10.3 その他

| 項目 | 状態 |
|---|---|
| stream-json **入力**スキーマ | Phase 2.2 で実機検証済み (`{"type":"user","message":{"role":"user","content":"<text>"}}`) |
| `tool_result` イベント | Phase 10.28 で実装。`"user"` イベント内の `tool_result` 配列を `ChatEvent::ToolResult { id, content }` に展開し、`record_tool_result(id, content)` で対応する `ToolBlock.output` に書き込む |
| `input_json_delta` (tool 引数の streaming) | Phase 10.28 で実装。`current_tool_input_buffer` で fragment を蓄積、`serde_json::from_str` が成功した時点で `set_last_tool_input` で `ToolBlock.input` に反映 |
| ストリーム終了マーカー | Phase 2.0 で `result` イベントを確認、追加マーカーは観測されず |
| Bash 系ツールの出力長制限 | 大きな出力は collapsible ブロック内でスクロールできるよう将来検討 |
| chat 履歴のセッション内ナビゲーション (過去メッセージへの参照) | MVP では無し |
| Markdown 中の画像をチャット内で表示する範囲 | MVP は preview 側に限定。chat 側は claude が返したテキストのみ |
| モデル選択 UI | MVP では `--model` 引数指定なし。F-23 設定ファイルで指定可に |
