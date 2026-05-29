# Claude Code 連携仕様

## 1. 概要

mdpilot は Claude Code（`claude` CLI）を **GUI から spawn する子プロセス**として扱い、`--print --input-format=stream-json --output-format=stream-json` の JSON Lines プロトコル経由でメッセージをやり取りする。Claude Code の動作そのものには手を加えない。両者の連携で解決すべき統合課題は以下に限られる。

| 課題 | 関連要件 | 解決状況 |
|---|---|---|
| プレビュー対象ファイルが書き換わったら自動で再レンダリングする | F-08 | `notify` で解決。詳細は `architecture.md` 3.3 |
| Claude Code が書いた／編集したファイルをプレビューに表示する（対象切替） | F-09 | 自動追従（5 章、案 A） |
| Claude Code を起動する cwd・環境変数・引数 | F-02 / F-11 | 本ドキュメント 3 章 |
| chat UI からのメッセージ送受信プロトコル | F-02 / F-03 / F-04 | `chat.md` |

## 2. 前提

- mdpilot は **プロジェクトディレクトリ単位** で起動する
  - 起動形態: `mdpilot <project-dir>` / `mdpilot <file.md>`（後者は親ディレクトリをプロジェクトルートとみなす）/ 引数なしで起動した場合はプロジェクト選択ダイアログを表示
  - 起動後の「プロジェクトルート」は実行中に変わらない（別プロジェクトに切替えるには新規ウィンドウで起動する）
- mdpilot は **プロジェクトルートを cwd として** `claude` を子プロセスとして spawn する
- mdpilot は Claude Code の内部実装に依存しない（CLI として動作することのみを前提）
- ターミナルエミュレータは持たない（`requirements.md` 6 章）

## 3. claude 子プロセスの起動条件

### 3.1 起動コマンド

`chat.md` 2.1 章で定義する。

```
claude
  --print
  --input-format=stream-json
  --output-format=stream-json
  --include-partial-messages
  --dangerously-skip-permissions
  [--session-id <uuid>] [--continue]
```

### 3.2 起動環境

| 項目 | 値 |
|---|---|
| cwd | プロジェクトルート |
| 環境変数（追加） | `MDPILOT_PROJECT_ROOT=<絶対パス>` |
| 環境変数（継承） | mdpilot プロセスの環境変数を継承（`PATH` 等） |
| 標準入出力 | `Stdio::piped()` 相当（mdpilot がパイプを保持し、JSON Lines を読み書き） |
| stderr | mdpilot が読み取り、tracing 経由でログに残す |

`MDPILOT_PROJECT_ROOT` は将来の追加 IPC（MCP サーバ等）のための足場として確保する。MVP では mdpilot が claude に渡す唯一の追加環境変数。

### 3.3 セッション ID と `--continue`

詳細は `chat.md` 5 章。要点：

- mdpilot は **プロジェクトルートごとに session-id を 1 つ** ディスク（`config::paths::AppPaths::data_dir / sessions.json`）に保存する
- 起動時にディスクから取得し、`--session-id <uuid>` + `--continue` を付ける
- 保存がない場合は新規 UUID を生成し、`--session-id <uuid>` のみで起動。最初の `system/init` イベントが流れてきた時点で session-id をディスクに保存

## 4. F-08 の再確認（表示中ファイルの自動再レンダリング）

「現在表示中のファイル」が外部書き換えで更新されたら、プレビューを自動再レンダリングする。

- 監視ライブラリ: `notify`
- フロー: `architecture.md` 3.3
- デバウンス: 100ms（`preview.md` 7 章）

F-08 は Claude Code の存在に依存しない（任意の外部エディタによる編集にも追従する）。

## 5. F-09 の実現方式（プレビュー対象の指定）

「Claude Code がどの `.md` を扱っているか」を mdpilot がどう知るか、の問題。

### 5.1 候補

| 案 | 仕組み | 必要設定 | Claude 側の協力 |
|---|---|---|---|
| A. 自動追従 | mdpilot がプロジェクト配下の `*.md` を監視し、**直近に書き換わった `.md`** をプレビュー対象に切り替える | なし | 不要（普通にファイルを書けばよい） |
| B. stream-json の `tool_use` を読む | mdpilot が claude stdout の `tool_use`（Edit/Write）イベントから `file_path` を抽出して切替 | なし | 不要（自然に流れてくる） |
| C. MCP サーバ | mdpilot が stdio MCP サーバを兼ね、Claude Code から MCP ツール（例: `mdpilot__open`）を呼べるようにする | `.mcp.json` または `claude mcp add` で登録 | MCP ツール経由で呼ぶ |

### 5.2 採用方針

**MVP は A（自動追従）のみ**を実装する。理由：

- 中心ユースケース（「Claude にドキュメントを書かせ、結果を見る」）は編集が起点であり、A で十分に成立する
- 実装コストが最も小さい（`notify` を F-08 用にすでに導入しているため、再利用できる）
- B（stream-json の `tool_use` 解釈）は精度が高い候補だが、Claude Code の出力スキーマ（特に Edit/Write 以外のファイル書込み系ツール、たとえば NotebookEdit や将来追加されるツール）に追従するコストが伴う。MVP 後の改善で B を追加する余地は残す

**MVP 後の拡張余地**：

- B: stream-json の `tool_use` 解釈で精度向上（編集前にプレビュー対象を切替えられる）
- C: 明示指定が必要なユースケース（複数 `.md` を並行編集中に明示指定したい）に対応

## 6. 自動追従の詳細仕様（採用案 A）

### 6.1 監視対象

- プロジェクトルート以下を再帰的に監視
- 対象拡張子: `.md`（大文字小文字区別なし、`.markdown` も含む）
- 除外ディレクトリ（既定）: `.git/`, `node_modules/`, `target/`, `dist/`, `build/`, `.next/`, `.svelte-kit/`, `.venv/`, `__pycache__/`
- 除外設定の上書きは MVP 後

### 6.2 切替ロジック

| 状況 | 動作 |
|---|---|
| 現在の表示ファイルが書き換わった | 切替なし。F-08 として再レンダリング |
| 現在表示中以外の `.md` が書き換わった | プレビュー対象を**その新しいファイルに切替**、Watcher を張り替え |
| 新しい `.md` が作成された | 上記と同じ扱い |
| 表示中ファイルが削除された | `preview.md` 7 章に従い「ファイルが見つかりません」表示。Watcher 維持 |
| ユーザーが `Cmd+O`/`Ctrl+O` で明示選択 | 自動追従は一時停止（6.3） |

### 6.3 自動追従の一時停止

ユーザーが `Cmd+O` 等で明示的にファイルを選んだ場合、その後の Claude の編集で**勝手にプレビューが飛ばないようにする**。

- 明示選択後は「自動追従モード OFF」
- 自動追従に戻すには、左ペイン上部のパスバーに「Claude の編集を追従」ボタンを配置（MVP）
- 自動追従 ON 時に新規 `.md` 編集を検出すると再び自動切替する

### 6.4 競合・順序

- 短時間に複数の `.md` が書き換わった場合、**最も新しい mtime のものに切り替える**
- デバウンス: 200ms（F-08 の 100ms より長め。複数ファイル更新を 1 回の切替判定にまとめる）

### 6.5 起動直後の挙動

| 状況 | 動作 |
|---|---|
| `mdpilot <file.md>` で起動 | そのファイルを表示。自動追従モード ON |
| `mdpilot <project-dir>` で起動 | プロジェクトルートに `README.md`（大文字小文字区別なし）があれば表示、なければ空ペイン |
| 引数なし | プロジェクト選択ダイアログ → 上記のいずれか |

空ペイン状態でも自動追従は有効。Claude が最初に `.md` を編集した時点でプレビューが起動する。

## 7. ユーザーから見たフロー

典型シナリオ：

1. `mdpilot ~/projects/blog` で起動
2. プロジェクトルートに `README.md` がなければ左ペインは空。右ペインで chat UI が立ち上がる
3. mdpilot が背後で `claude --print --input-format=stream-json --output-format=stream-json --dangerously-skip-permissions` を spawn（session-id をディスクから取得 → あれば `--continue`、なければ新規 UUID）
4. chat 入力欄に「README.md を書いて」と打って Enter
5. mdpilot が JSON でメッセージを claude stdin に書込み
6. claude のレスポンスが stream-json で流れる：テキスト断片、Edit ツール呼び出し → README.md 書込み、ツール結果、result
7. 右ペインは assistant 応答を Markdown レンダリング、Edit ツールは collapsible ブロックとして表示
8. mdpilot の `notify` が `README.md` 作成を検出 → 自動追従で左ペインに表示
9. ユーザーが「もっと詳しく」と chat で送る → claude が `README.md` を Edit → F-08 で再レンダリング
10. claude が `docs/install.md` を新規作成 → 自動追従で左ペインが切替
11. README を見直したくなり `Cmd+O` で開く → 追従モード OFF
12. その後の claude 編集ではプレビューは飛ばない
13. パスバーの「Claude の編集を追従」ボタンを押すと再び ON
14. mdpilot を閉じる → session-id が `sessions.json` に保存される
15. 翌日 `mdpilot ~/projects/blog` で再起動 → session-id から `--continue` で前回の会話を継続

## 8. セキュリティ・サンドボックス

- MVP は `--dangerously-skip-permissions` で起動するため、claude のツール呼び出しは**プロジェクト内外問わず**自動許可される。プロジェクト外のファイル編集制御は Claude 側のポリシーとユーザーの責任に委ねる
- mdpilot 自体はネットワーク通信を行わない（MVP 範囲）。claude プロセスは内部で Anthropic API と通信する
- 安全モードは `requirements.md` F-28（MVP 後）

## 9. 既知の制限・未確定事項

| 項目 | 状態 |
|---|---|
| MCP サーバ機能（5 章案 C）の実装可否・タイミング | MVP 後 |
| stream-json の `tool_use` 解釈による F-09 強化（5 章案 B） | MVP 後 |
| 除外ディレクトリのデフォルトリスト最終確定 | 実装着手時 |
| 自動追従モードの ON/OFF を UI 上でどう示すか（バッジ・色等） | UI 設計時 |
| プロジェクトルート変更（同一ウィンドウで別プロジェクトを開く） | MVP では新規ウィンドウ起動。同一ウィンドウ切替は MVP 後 |
| Claude Code 以外のエージェント連携 | スコープ外（`requirements.md` 6 章） |
| `notify` の再帰監視で大規模リポジトリのパフォーマンス問題 | 実装着手時に検証 |
| stream-json 入力スキーマ・終了マーカー | `chat.md` 3 章に従い Phase 2 で実機検証 |
