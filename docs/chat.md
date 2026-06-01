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

| 項目 | 状態 |
|---|---|
| スキーマ | **TBD（未文書化）**。Phase 2 着手時に `claude --print --input-format=stream-json` を実機テストし、最小ユーザーメッセージ JSON を確定 |
| 改行区切り | JSON Lines（1 メッセージ = 1 行）と推定 |

実機検証の最初のタスクとして、`echo '{"type":"user","content":"..."}' | claude --print --input-format=stream-json --output-format=stream-json` のようなテストを試し、受理される形式を特定する。

### 3.2 出力（claude → mdpilot）

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
│  [Assistant]                             │
│  メッセージ本文…                          │
│                                          │
│  ▶ Edit ▽                                │ ← Collapsible ツールブロック (折りたたみ状態)
│                                          │
│  [User]                                  │
│  もっと詳しく説明して                     │
│                                          │
│  [Assistant]                             │
│  ストリーミング中…                        │
│                                          │
│  ▼ Bash ▽                                │ ← 展開状態
│    Input: ls docs/                       │
│    Output: chat.md  preview.md  ...      │
│                                          │
├──────────────────────────────────────────┤
│ ┌────────────────────────────────────┐   │
│ │ プロンプトを入力…                   │   │ ← 入力欄 (egui::TextEdit, 複数行)
│ └────────────────────────────────────┘   │
│            [送信] [中断]                  │ ← MVP は 2 ボタンのみ
└──────────────────────────────────────────┘
```

### 4.1 メッセージ表示

| 種別 | 描画 |
|---|---|
| Assistant テキスト | Markdown としてレンダリング（`egui_commonmark` を再利用、ただし chat 向けに簡素化を検討）。ストリーミング中は末尾にカーソル表示 |
| User テキスト | 太字のヘッダー「User」+ 本文（プレーンテキスト or 簡易 Markdown） |
| Tool 呼び出し | Collapsible ブロック：ヘッダーに `▶ <tool name>`、展開で `input` JSON と `output` を表示。MVP は出力をプレーン表示、Phase 9 で出力の言語別ハイライト |
| API リトライ | ステータス行：「rate_limit でリトライ中 (1/3) …」を控えめに表示。retry 成功で消える |
| エラー (`result.subtype != success`) | 赤系の注釈ブロック：subtype と stderr 末尾 |

### 4.2 入力欄

- `egui::TextEdit::multiline` をベース、`Enter` で送信、`Shift+Enter` で改行
- IME（日本語入力）は `egui::TextEdit` の標準サポートに委ねる（`requirements.md` N-07）
- 送信ボタンは入力空でない時のみ有効
- 「中断」ボタンは在進行中の応答ストリームをキャンセル（送信中に claude プロセスへ Ctrl+C 相当のシグナル？または `--max-turns` 強制？）。**実装方式は Phase 2 着手時に実機検証**

### 4.3 スクロール

- chat 履歴は縦スクロール。末尾追従モード：スクロール位置が末尾にあるとき、新着メッセージで自動追従。スクロールアップすると追従停止
- 履歴は仮想化しない（MVP）。長くなりすぎる前にユーザーが新ウィンドウを開く運用を想定

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
| `Enter`（入力欄フォーカス時） | 送信 |
| `Shift+Enter` | 入力欄に改行を挿入 |
| `Esc`（応答中） | 中断ボタン相当 |
| `Cmd+C` / `Ctrl+Shift+C` | 選択範囲をコピー |

`ui.md` 6 章の全体キーバインドと整合する。

## 8. テーマ・フォント

- 入力欄・本文ともに `ui.md` 8 章のテーマ追従に合わせる
- フォントは `egui` の既定（日本語フォールバックも `egui` 設定）

## 9. エラー処理

| 状況 | UI |
|---|---|
| `claude` バイナリが PATH に無い | 起動時にダイアログを出してアプリ終了 |
| claude 子プロセスが起動直後に終了 | chat ペインに「Claude の起動に失敗しました。stderr: …」+ 再接続ボタン |
| stream-json パース失敗（未知イベント） | tracing にログ。chat 下部に小さな警告 |
| `system/api_retry` | 4.1 参照 |
| `result.subtype != success` | 4.1 参照 |
| ネットワーク断（API タイムアウト） | claude 側で `api_retry` が走る想定。retry 上限超過は `result.subtype != success` で表現される |

## 10. 既知の制限と未確定事項

| 項目 | 状態 |
|---|---|
| stream-json **入力**スキーマ | 公式未文書化。Phase 2 着手時に実機検証で確定 |
| 中断ボタンの実装方式 | Phase 2 着手時に確定（シグナル / `--max-turns` / 別 RPC） |
| ツール `tool_result` イベントの正確なスキーマ | Phase 2 着手時に実機検証 |
| ストリーム終了マーカー | Phase 2 着手時に実機検証 |
| Bash 系ツールの出力長制限 | 大きな出力は collapsible ブロック内でスクロールできるよう将来検討 |
| `--include-partial-messages` 無効時の挙動差分 | Phase 2 で `--include-partial-messages` のオン / オフを切り替えて比較 |
| chat 履歴のセッション内ナビゲーション（過去のメッセージへの参照） | MVP では無し |
| Markdown 中の画像をチャット内で表示する範囲 | MVP は preview 側に限定。chat 側は claude が返したテキストのみ |
| モデル選択 UI | MVP では `--model` 引数指定なし（claude 既定モデルを使う）。F-23 設定ファイルで指定可に |
