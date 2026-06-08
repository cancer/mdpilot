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
| 2 | 2.1 | claude 子プロセスの起動 | `std::process::Command` で `claude` を spawn、`Stdio::piped()` で stdin/stdout/stderr。引数は `--print --verbose --input-format=stream-json --output-format=stream-json --include-partial-messages --dangerously-skip-permissions --session-id <uuid> [--continue]` | 2.0 | ✓ |
| 2 | 2.2 | stream-json 入力スキーマの実機検証 | `{"type":"user","message":{"role":"user","content":"<text>"}}` で受理されると実機確認、`chat.md` §3.1 に反映 | 2.1 | ✓ |
| 2 | 2.3 | stdout 読込スレッドと stream-json パーサ | `serde_json::Value` で部分抽出（typed struct は使わない）、ChatEvent enum に変換、未知は tracing | 2.1 | ✓ |
| 2 | 2.4 | stderr 読込スレッドと tracing 連携 | claude の stderr 行を `tracing::warn` (target = `claude::stderr`) に流す | 2.1 | ✓ |
| 2 | 2.5 | stdin 書込み | `send_user_message(text)` で 1 行 JSON を書込み + flush。`write_user_message<W: Write>` でテスト可能 | 2.2, 2.3 | ✓ |
| 2 | 2.6 | session-id ディスクストア | `data_dir/sessions.json` を atomic write、`SessionStore::load_or_default/save_atomic/get/upsert` を chrono タイムスタンプ付きで実装 | 0.4 | ✓ |
| 2 | 2.7 | 子プロセスのライフサイクル | `status()` で try_wait ラップ、Drop で stdin drop → SIGTERM → 500ms 待機 → SIGKILL | 2.1 | ✓ |
| 3 | 3.1 | チャット UI の枠（メッセージリスト + 入力欄 + ボタン） | `chat_pane.rs` で縦スクロール領域 + `TextEdit::multiline` + 送信/中断ボタン | 1.3 | ✓ |
| 3 | 3.2 | テキストメッセージのストリーミング描画 | `text_delta` イベントを assistant メッセージに追記、ストリーミング中はカーソル表示 | 2.3, 3.1 | ✓ |
| 3 | 3.3 | ツール呼び出しの collapsible 表示 | `tool_use` / `tool_result` を collapsible ブロックで描画（既定折りたたみ）、`F-04` | 3.2 | ✓ |
| 3 | 3.4 | API リトライ・結果イベント表示 | `system/api_retry` を控えめバナー、`result.subtype != success` を赤系注釈 | 3.2 | ✓ |
| 3 | 3.5 | 実機 claude 統合と Enter 送信 / IME | App::new で `ChatSession` を起動、mpsc で `ChatEvent` を main に流し `ChatHistory::apply` で畳み込み。Enter で送信（Shift+Enter は改行）、`extract_send_enter` で plain Enter のみイベント queue から取り除く。IME 入力は実機検証（N-07） | 3.1, 2.5 | ✓ (IME / e2e は要実機検証) |
| 3 | 3.6 | 中断ボタンの実装方式確定 | claude CLI 2.1 で stream-json mode にリアルタイム中断手段が存在しないことを公式 docs と GitHub issue で確認。MVP は 中断 ボタンを `add_enabled(false, ...)` で永続 disabled + tooltip 表示。`docs/chat.md` §10.1 に詳細記録 | 3.5 | ✓ |
| 3 | 3.7 | メッセージ選択・コピー（F-05） | egui 0.34 の `LabelSelectionState` plugin（context.rs:741、デフォルト有効）でドラッグ選択 + `Cmd+C`/`Ctrl+C` → OS クリップボードが既に動く。`view.rs` で `Label::selectable(true/false)` を明示し、User/Assistant ヘッダーは非選択、本文 / tool input/output / system error は選択可能に。ショートカットは egui 既定の `Cmd+C` / `Ctrl+C`（旧 `Ctrl+Shift+C` 仕様は egui_term 時代の名残のため `ui.md` を訂正） | 3.2 | ✓ |
| 4 | 4.1 | ファイル読込ローダー | `src/preview/loader.rs` で `load_markdown(path)` を提供。`fs::metadata` でサイズ取得 → 10MB 以上は body を読まずに `TooLarge` 返却、それ以外は `read_to_string` → 1MB しきい値で `SizeClass::Small` / `Large` 分類。エラーは `NotFound` / `PermissionDenied` / `NotUtf8` / `Io` に正規化。`load_with_limits` でテストにしきい値注入可能。10 件の unit test 追加 | 0.3 | ✓ |
| 4 | 4.2 | ~~egui_commonmark 描画~~ | **superseded** (2026-06-05): markdown プレビュー機能を omit、左ペインを read-only ソースビューに置き換え（Phase 9.20 参照）。egui_commonmark + CommonMarkCache + render_override_for は撤去 | 1.3, 4.1 | ✗ |
| 4 | 4.3 | syntect シンタックスハイライト統合 | （元）`egui_commonmark` の `better_syntax_highlighting` feature 経由で fenced code block を色付けし、Large は info-string strip で plain にフォールバック。**Phase 9.20 で書き換え**: 直接 `syntect` 5 を依存に持ち、markdown ソース全体を行ごとに `HighlightLines::highlight_line` で `LayoutJob` に積む方式へ。テーマ（`base16-ocean.dark` / `InspiredGitHub`）と dark_mode 連動は保持 | 4.2 | ✓ |
| 4 | 4.4 | ~~リンク挙動~~ | **superseded** (2026-06-05): ソース表示ではリンクが描画されないので分類器不要。`src/preview/link.rs` と `app.rs::dispatch_link_clicks` / `handle_link_click` を撤去。`open` クレート依存も削除（Phase 9.20） | 4.2 | ✗ |
| 4 | 4.5 | ~~画像・相対パス解決~~ | **superseded** (2026-06-05): ソース表示では画像が描画されないので URI 書き換え不要。`src/preview/image.rs::rewrite_image_uris` / `to_file_uri` と `pulldown-cmark` 直接依存を撤去（Phase 9.20） | 4.2 | ✗ |
| 4 | 4.6 | プレビューのスクロール位置保持 | **削除**：2026-06-04 ユーザー判断でスクロール位置保持・編集追従はいずれも仕様から除外（F-22 も削除）。再読込時は常に最上端に戻る挙動が確定 | 4.2 | ✗ |
| 5 | 5.1 | `notify` Watcher セットアップ | `src/preview/watcher.rs` に `FileWatcher` + `FileWatchEvent` + `classify_event`。`notify::recommended_watcher` は自前で dispatcher thread を持つので追加の `std::thread` は不要。コールバックで `EventKind::Modify/Create` → `Changed`、`Remove` → `Removed`、`Access/Any/Other` は drop。エラーは `FileWatchEvent::Error(String)` に正規化。`watch/unwatch/unwatch_all/watched_paths` で App から path 管理可能。`wake_ui` は `egui::Context::request_repaint` を想定。11 unit test (pure classifier 7 + watch lifecycle 4)。Phase 5.2 で App 配線・100ms デバウンス・F-08 完成 | 0.3 | ✓ |
| 5 | 5.2 | 単一ファイル監視と再レンダリング（F-08） | `App` に `FileWatcher` + `Receiver<FileWatchEvent>` + `pending_reload: Option<Instant>` を配線。`sync_watch_target` が `set_document` / link 切替直後に `unwatch_all` → `watch(new)` を実行。`drain_watch_events` を `logic()` で毎フレーム呼び、`Changed` は `pending_reload = now + 100ms` で再 arm、`Removed` は即時 `set_error(NotFound)`、`Error` は tracing::warn。`poll_pending_reload` が `watcher::reload_decision(deadline, now) -> ReloadStep` の結果に従って `Idle / Wait{remaining}` / `Fire`（= `loader::load_markdown` + `set_document`）を分岐。`watcher::paths_match` は exact equality → fs::canonicalize fallback で macOS FSEvents の `/private/var/...` 正規化に対応。`watcher::reload_decision` + `paths_match` は pure unit test 8 件追加。E2E 検証はインタラクティブ操作が必要なため screenshot helper では未確認、ユーザー実機検証へ。Known platform variance: Linux inotify では `Remove` 後に watch が無効化される可能性、Phase 5.3 の Cmd+R 手動再読込で復旧予定 | 4.2, 5.1 | ✓ |
| 5 | 5.3 | 監視エラーのステータス表示 | `App` に `watcher_error: Option<String>` を追加し、(a) `FileWatcher::start` 失敗 (b) `sync_watch_target` の `watch()` 失敗 (c) `FileWatchEvent::Error` の 3 経路でセット、`reload_current` 成功時にクリア。`src/ui/preview_pane.rs::show_watcher_error_banner` で amber 色の 1 行バナーを preview pane 上部に描画（Phase 7.7 のステータスバーが入るまでの stop-gap）。`Cmd+R` / `Ctrl+R` は `App::consume_reload_shortcut` が `egui::Modifiers::COMMAND + Key::R` を `consume_shortcut` で取得し `reload_current` を呼ぶ。`reload_current` は `Loaded` だけでなく `Failed::path_label` からも reload を試みるため、削除→「見つかりません」→再作成→Cmd+R で復元のフローが動く（spec preview.md §7 の手動復旧経路）。`sync_watch_target` を reload 成功後も呼ぶことで watch も再アタッチ | 5.2 | ✓ |
| 6 | 6.1 | プロジェクトルート解決 | `src/cli.rs` を拡張して `--enable-dev-tools` フラグ + 先頭 positional を取得。`src/project.rs` に `resolve(positional) -> Result<ProjectInit, ProjectInitError>` を追加：`<dir>` → root = arg、`<file>` → root = parent + initial_file = file、引数なし → root = cwd（Phase 7.1 の選択ダイアログまでの暫定 fallback、tracing::info で告知）。存在しないパスは `NotFound`、ファイル/ディレクトリ以外は `Unsupported`、その他 I/O 失敗は `Io { path, source }` に正規化。すべて canonicalize 後に判定（macOS の `/var` → `/private/var` symlink 等で `paths_match` と整合）。`main.rs` がエラー時に `process::exit(2)` + stderr に message を表示。`App::new(cc, cli, project)` に project を渡し、`spawn_session(ctx, root)` で claude の cwd に渡す（既存の `current_dir()` を置換）。pure unit test 5 件（NotFound / directory arg / file arg / no arg fallback / nested file → parent root）+ CLI parser 6 件。実機 smoke run（非存在 / dir / file）で exit code 2 と screenshot 完了を確認 | 0.4 | ✓ |
| 6 | 6.2 | プロジェクト配下 `.md` の再帰監視 | `src/preview/watcher.rs::ProjectWatcher` 追加（`RecursiveMode::Recursive` でプロジェクトルートを `notify::recommended_watcher` に登録、コールバックが `classify_project_event` でフィルタしてから mpsc に流す）。Pure helpers: `EXCLUDED_DIRS` 定数（`.git` / `node_modules` / `target` / `dist` / `build` / `.next` / `.svelte-kit` / `.venv` / `__pycache__`）、`is_excluded_dir`、`is_markdown_path`（`.md` / `.markdown` の case-insensitive 判定）、`is_in_excluded_subtree(path, root)`（intermediate components のみチェック → root 直下の `.git` という *ファイル* は除外しない）、`classify_project_event`。プロジェクトルートは spec 上 mdpilot 起動中に変わらないので `ProjectWatcher::start` で `root` を所有・watch を即時アタッチ、後で path を入れ替えるシナリオは無い。App には `_project_watcher: Option<ProjectWatcher>` と `project_events_rx` を持たせ、`drain_project_events` で tracing::info に流すだけ（auto-follow 配線は Phase 6.3）。13 unit test 追加（excluded dir マッチ / 非マッチ、markdown 拡張子 case-insensitive、subtree フィルタ、`classify_project_event` の 4 ケース、`ProjectWatcher::start` の smoke）| 5.1, 6.1 | ✓ |
| 6 | 6.3 | 自動追従ロジック（F-09 案 A） | `src/preview/watcher.rs::FOLLOW_DEBOUNCE = 200ms` 追加。`App::drain_project_events` を tracing から本実装に置換：`Changed { path }` を受け取ったら `watcher::paths_match(path, current)` で「現在表示中」かを判定 → 一致なら 5.2 の single-file watcher に任せて drop、不一致なら `pending_follow = Some((path, now + 200ms))` で arm。`Removed` は何もしない（deleted file に follow しない、ただし pending target が削除されたらキャンセル）。`Empty` preview からも追従可能（spec §6.4）、`Failed` 状態の path_label は「現在」扱い。`poll_pending_follow` が `watcher::reload_decision` で `Idle/Wait/Fire` を判定し、`Fire` 時に `loader::load_markdown` + `set_document` + `pending_reload = None` + `sync_watch_target`。link クリックや link 経由の `SwitchMarkdown` も `pending_follow = None` でキャンセル（user-driven > claude-driven）。`collect_project_events` は borrow checker 回避用の 2-pass drain（既存の `collect_watch_events` と対称）。実機 smoke run でルート + サブディレクトリ両方の `.md` 検出を確認 | 5.2, 6.2 | ✓ |
| 6 | 6.4 | 起動直後の対象選択 | `src/project.rs` に `find_readme(root)` + `is_readme_name(name)` + `initial_preview(init)` を追加。`is_readme_name` は `readme.md` / `readme.markdown` の ASCII case-insensitive 完全一致（pure unit test）、`find_readme` は root 直下を `read_dir` で走査し最初の match を返す（ファイルのみ、サブディレクトリと「README.md という名のディレクトリ」は除外）。`App::new` で旧 `preview_state_from_env`（`MDPILOT_PREVIEW_FILE` env var）を `initial_preview_state(&ProjectInit)` に置換：(a) `project.initial_file` を優先、(b) なければ `find_readme(project.root)`、(c) どちらも無ければ `PreviewState::default()`（空ペイン）。10 unit test 追加（is_readme_name の許容/拒否、find_readme の root match / mixed case / 不在 / サブディレクトリ無視 / ディレクトリ名無視、initial_preview の 3 ケース）。実機 smoke: `mdpilot <dir-with-ReadMe.md>` → README が表示、`mdpilot <empty-dir>` → 「プレビュー未指定」 | 6.1, 6.3 | ✓ |
| 6 | 6.5 | `MDPILOT_PROJECT_ROOT` 環境変数の付与 | コード自体は Phase 2.1（`src/chat/session.rs::ChatSession::start`）の時点で `.env("MDPILOT_PROJECT_ROOT", &opts.project_root)` を仕込んでいたので、Phase 6.1 で project.root が `std::fs::canonicalize` 結果（絶対パス）になった時点で要件達成。`docs/claude-integration.md` §3 の「環境変数（追加） `MDPILOT_PROJECT_ROOT=<絶対パス>`」を満たしているか確認のみ。実装変更なし、docs/plan.md と CLAUDE.md の進捗更新だけ | 2.1, 6.1 | ✓ |
| 7 | 7.1 | `Cmd+O`/`Ctrl+O` ファイル選択ダイアログ | `rfd = "0.17"`（default features off、Linux 系 xdg-portal/wayland は skip）。`App::consume_open_shortcut(ctx)` を `consume_reload_shortcut` の隣に追加し、`egui::KeyboardShortcut::new(COMMAND, Key::O)` を `input_mut().consume_shortcut` で取り出す。`rfd::FileDialog::new().add_filter("Markdown", &["md","markdown"]).set_directory(start).pick_file()` 同期 API を呼ぶ（dialog 中は egui フレーム停止、macOS の OS 慣習通り）。`file_picker_start_dir()` は preview Loaded なら parent dir、それ以外は新規追加した `App::project_root: PathBuf`（Phase 6.1 で canonicalize した root の clone）。選択時は `loader::load_markdown` → `set_document` → `pending_reload`/`pending_follow` クリア → `sync_watch_target`、エラーは `set_error`、cancel 時は no-op。auto-follow ON/OFF の永続化は Phase 7.2 で扱う | 4.2, 6.3 | ✓ |
| 7 | 7.2 | 自動追従モード ON/OFF | `App::auto_follow_enabled: bool`（default `true`）を追加。`drain_project_events` の Changed 分岐で is_current 判定後に `!auto_follow_enabled` ならスキップ（`pending_follow` を arm しない）。`consume_open_shortcut` 成功時に `auto_follow_enabled = false`（`docs/preview.md` §9.1.1）。`src/ui/preview_pane.rs::show_follow_disabled_banner` を追加し、OFF 時のみ「自動追従: OFF」表示 + 「再開する」`small_button`、`layout::show` に `auto_follow_enabled` + `on_reenable_follow` コールバックを通す（`#[allow(clippy::too_many_arguments)]` で 8 引数を許容、Phase 7.7 で path bar 統合時に struct 化予定）。クリックで `auto_follow_enabled = true` に戻し tracing::info。preview pane chrome（watcher_error + follow OFF）を `chrome_drawn` フラグで束ねて単一 `ui.separator()` を出す | 6.3, 7.1 | ✓ |
| 7 | 7.3 | キーバインド統合 | `App::consume_pane_reset_shortcut` を追加して `Cmd+\` / `Ctrl+\`（`COMMAND + Key::Backslash`）で `ui::layout::reset(ctx)` を呼ぶ。`logic()` の shortcut 群（reload / open / pane reset）を順に消費。他のキーバインド（`Cmd+Q`、`Cmd+C`、`Enter` 系、`Shift+Enter`）は eframe / egui / TextEdit のデフォルトが期待通りに動くので追加配線不要。`Cmd+F` は MVP 後（plan.md §6 着手前リスト）。`Esc` は claude CLI 2.1 で interrupt 不可能（Phase 3.6 で確定）のため何もバインドしない — disabled な「中断」ボタンの tooltip と整合する。フォーカスペイン依存の解釈は現状の `Cmd+V`/`Enter`/`Shift+Enter` が TextEdit 内でしか効かないことで自然に実現済み | 3.6, 4.2 | ✓ |
| 7 | 7.4 | ウィンドウタイトル動的更新 | 自由関数 `compute_window_title(&PreviewStatus) -> String` を `src/app.rs` に追加（pure）: `Loaded` → `mdpilot - <document.path.file_name>`、`Failed` → label の basename（`std::path::Path::new(...).file_name()` で抽出）、`Empty` または `file_name()` が `None` の時は素の `"mdpilot"`。`App::last_window_title: String`（init `""`）と `App::update_window_title(ctx)` を追加して `logic()` 末で呼び、cached value と異なる時のみ `ctx.send_viewport_cmd(ViewportCommand::Title(...))` を送信（安定状態では per-frame 通信ゼロ）。unit test 4 件（Empty / Loaded / Failed / 末尾スラッシュで file_name=None の fallback）| 6.3 | ✓ |
| 7 | 7.5 | macOS メニューバー | **MVP 外**：Cmd+O / Cmd+R / Cmd+\ は Phase 7.1 / 5.3 / 7.3 で動作中であり、メニューバーは discoverability の上乗せのみ。`muda` 統合 + NSApp.mainMenu FFI は中規模の作業で、screenshot helper でも検証不可（ウィンドウ chrome 範囲外）。2026-06-03 ユーザー判断で Phase 9.15 に移送 | 7.1, 7.3 | ✗ |
| 7 | 7.6 | Windows ツールバー | **MVP 外**：7.5 と対の discoverability タスク。同じ理由で Phase 9.16 に移送 | 7.1, 7.3 | ✗ |
| 7 | 7.7 | パスバーとステータス表示 | `src/ui/path_bar.rs` 新規。`egui::Panel::top("path_bar")` を `app::App::ui` の先頭で `show_inside` し、その下に既存の `layout::show` を残す。1 行で左に preview path（`Loaded` は絶対パス、`Failed` は `⚠ <path>`、`Empty` は「（プレビュー未指定）」）、右側は `Layout::right_to_left` で「自動追従: ON/OFF」ボタン → `● Claude 接続中/切断` 色付きラベル → watcher_error がある時のみ amber の警告ラベル。`preview_path_label` を pure helper として抽出し 3 件の unit test（Empty / Loaded / Failed）。Phase 5.3 と 7.2 の `preview_pane.rs` 内バナー（`show_watcher_error_banner` / `show_follow_disabled_banner`）と layout::show の関連引数を撤去（path bar が完全に置換）。auto-follow ボタンは push でフリップ（ON↔OFF）するトグルに変更。エラートーストは出さず（toast 実装は MVP 後で十分）、設計上は同じ slot から出せる構造に統一されている | 5.3, 6.3, 2.7 | ✓ |
| 7 | 7.8 | テーマ追従 | egui 0.34 のデフォルト `ThemePreference::System` で OS 追従が組み込み済み。eframe (`wgpu_integration.rs:276`) が `event_loop.system_theme()` を `egui_winit::State::new` に渡し、winit が `WindowEvent::ThemeChanged` をハンドリングするのでランタイム切替も追従。Phase 9.20 以降は `src/preview/render.rs::show` 内で `ui.style().visuals.dark_mode` を直接読み、`SYNTAX_THEME_DARK` (`base16-ocean.dark`) / `SYNTAX_THEME_LIGHT` (`InspiredGitHub`) を切替（旧 egui_commonmark 経由ではなく直接 syntect 呼び出し）。`main.rs` でも `App` でも `ctx.set_theme(...)` を呼んでいないので explicit override 無し | 4.3 | ✓ |
| 7 | 7.9 | 非機能要件の測定 | `docs/perf.md` を新規作成し、N-01〜N-04 の計測手順 + 目標 + 結果欄を整備。`src/app.rs` に N-01 計測 instrumentation を追加：`App::new` 冒頭で `Instant::now()` を `startup_started: Option<Instant>` に保存、最初の `ui()` 呼び出しで経過を `tracing::info!(target: "mdpilot::perf")` でログして以後 `None`。debug ビルドで動作確認（empty project で約 65ms、3 秒予算に対し十分余裕）。N-02/N-03/N-04 の実値は Phase 8.1/8.2 のリリースビルド完成後にユーザー実機で測定して `docs/perf.md` 表に追記する流れ | 全 Phase | ✓ |
| 8 | 8.1 | macOS バンドル | `cargo-bundle` で `.app` 生成、`aarch64`/`x86_64` 両対応 | 7.* | — |
| 8 | 8.2 | Windows バイナリ | `x86_64-pc-windows-msvc` ターゲットでビルドスクリプト整備 | 7.* | — |
| 8 | 8.3 | CI（GitHub Actions） | macOS + Windows の build/test/clippy/fmt を回す | 0.1 | — |
| 8 | 8.4 | リリース手順ドキュメント | `docs/release.md` に手順記述 | 8.1, 8.2 | — |
| 9 | 9.1 | ~~F-21 リンク・画像（相対パス含む）の解決の精緻化~~ | **superseded** (2026-06-05): markdown プレビュー omit に伴い、HTTPS 画像描画 (A) と画像ファイル auto-reload (B) はいずれも対象外に。`IMAGE_EXTENSIONS` / `is_image_path` / `image::to_file_uri` / `ctx.forget_image` 経路をすべて撤去（Phase 9.20） | 4.4, 4.5 | ✗ |
| 9 | 9.2 | **削除**（旧 F-22 スクロール位置の編集追従）| 2026-06-04 ユーザー判断で仕様から除外。スクロール追従は MVP / MVP 後ともに実装しない | — | ✗ |
| 9 | 9.3 | F-23 設定ファイル | フォント・配色・キーバインド・ペイン比率・行数・モデル選択等 | 0.4 | — |
| 9 | 9.4 | F-24 アプリメニュー拡充 | macOS の環境設定、Windows の正式メニューバー | 7.5, 7.6 | — |
| 9 | 9.5 | F-25 複数チャット・複数プレビューのタブ | 4 サブフェーズで段階実装。**9.5.1**: `src/tab.rs` に `Tab` struct を作成し、`ChatSession`/`ChatHistory`/`PreviewState`/`FileWatcher`/`pending_*`/`auto_follow_enabled`/`watcher_error` を App から移送。`TabId` + `TabIdGen` で安定 ID 管理。App は `tabs: Vec<Tab>` + `active_tab: usize` + project_root + ProjectWatcher のみ保持。Per-tab メソッド (sync_watch_target / drain_chat_events / drain_watch_events / poll_pending_reload / poll_pending_follow / reload_current / handle_removed / handle_send) を Tab impl に。**9.5.2**: `src/ui/tab_bar.rs` 新規。`egui::Panel::top("tab_bar")` の中に chip 状の tab + 右側に `+` ボタン。`TabBarAction { None / Select(usize) / Close(usize) / NewTab }` を返し App が match で `new_tab()` / `close_tab(idx)` / `select_tab(idx)` に dispatch。最後の 1 タブは閉じない (`close_tab` guard)。**9.5.3**: `Cmd+T` (new), `Cmd+W` (close active), `Cmd+1..9` (switch to N-th) の shortcuts。`consume_*_shortcut` パターン。`Cmd+1..9` は `[Key::Num1..Num9]` で loop。**9.5.4**: 9.5.1 の時点で実装済み — project_watcher events は `self.active_mut()` 経由で active タブにのみ届く。非 active タブの auto-follow は事実上停止（spec の MVP 妥協、claude がどのタブから書いたかを判別できないため） | 7.* | ✓ |
| 9 | 9.6 | F-26 拡張: 数式・Mermaid・脚注 | KaTeX 相当・Mermaid・脚注を順次対応 | 4.2 | — |
| 9 | 9.7 | F-27 テーマ切替 | OS 追従に加え強制ライト/ダーク選択 | 7.8 | — |
| 9 | 9.8 | F-09 案 B（stream-json `tool_use` 解釈） | claude の `tool_use` から `file_path` 抽出 → 編集前にプレビュー対象を切替 | 6.3 | — |
| 9 | 9.9 | F-09 案 C（MCP サーバ） | mdpilot を MCP サーバとして公開、`mdpilot__open` などのツールを claude から呼べる | 6.3 | — |
| 9 | 9.10 | F-28 安全モード（パーミッション GUI モーダル） | `--dangerously-skip-permissions` を外し、ツール許可要求をモーダルで都度確認 | 3.4 | — |
| 9 | 9.11 | ~~プレビュー内検索（`Cmd+F`）~~ | **削除** (2026-06-05): markdown プレビュー omit と一体の判断。ソース表示への検索追加は将来課題として保留（実装するなら別タスクとして起票） | 4.2 | ✗ |
| 9 | 9.12 | チャット内検索 | チャット履歴内の検索 | 3.2 | — |
| 9 | 9.13 | ペイン比率の永続化・前回ウィンドウ位置復元 | 設定ファイル経由 | 9.3 | — |
| 9 | 9.14 | **削除**（旧 Phase 4.6 スクロール位置保持）| 2026-06-04 ユーザー判断で仕様から除外。9.2 と一緒にスクロール関連は全て不要と判断 | — | ✗ |
| 9 | 9.15 | macOS メニューバー | 旧 Phase 7.5。`muda` クレートで mdpilot / ファイル / 表示 / ウインドウ / ヘルプを構築し `init_for_nsapp` で NSApp.mainMenu に attach、`MenuEvent::receiver()` を App の drain ループに統合してアクション分岐（Open/Reload/Reset Pane）。MVP では Cmd+O / Cmd+R / Cmd+\ shortcut でアクセス可能だったが、macOS の慣習的 UX として menu bar が無いと違和感がある。Cmd+O 等の action は既に App メソッドとして抽出されているので menu 側からも呼ぶだけで済む | 7.1, 7.3 | — |
| 9 | 9.16 | Windows ツールバー | 旧 Phase 7.6。9.15 の Windows 対応版。`muda` クレートで「開く / 再読込 / 情報」を最小ツールバーに配置。実装パターンは 9.15 と共有可能 | 7.1, 7.3 | — |
| 9 | 9.19 | claude `--resume` 履歴ピッカー | `src/chat/history_picker.rs`：claude 内部ストレージ (`~/.claude/projects/<encoded>/<sid>.jsonl`) を直接読んでセッション一覧を取得。`encode_project_path` は path → ASCII-alphanumeric + `-` ホワイトリスト方式（`/` `.` `_` 等の非英数字を `-` に変換、実機検証で claude のエンコードと一致確認）。`list_sessions` で `.jsonl` を mtime 降順で並べる、`parse_first_user_message` で各 jsonl の最初の `type:"user"` 行から content (string / array 両対応) を抽出して 120 char + 多バイト安全な truncate でプレビュー文字列を作る。15 unit tests。`src/ui/session_picker.rs`：`egui::Window` モーダル中央配置、各セッションを Frame でカード化（short session-id / mtime ローカル時刻 / preview）、クリックで `SessionPickerAction::Resume(uuid)`、X で `Close`。`src/ui/tab_bar.rs` の右側に「履歴」ボタン追加 → `TabBarAction::OpenHistory` → App が `open_session_picker` でディレクトリ scan、結果を `SessionPickerData` に保持してモーダル表示。`SessionPickerAction::Resume` で `open_tab_resuming(uuid)` を呼び新規タブを `ResumeSession` 付きで spawn（既存タブは触らない）。home dir 取得は既存の `directories` crate の `BaseDirs::new()`、追加 dep なし | 9.18 | ✓ |
| 9 | 9.18 | F-11 session-id 永続化 + `--resume` での再開 | App に `SessionStore` を配線（旧 Phase 2.6 で実装済の `chat/session_store.rs` が App から未参照だった）。`<data_dir>/sessions.json` を `App::new` で読込、プロジェクトルートに対応する `SessionEntry` があれば session-id を取り出して `ResumeSession` で初期タブに渡す。`Tab::new(... resume: Option<ResumeSession>)` を拡張、内部の `spawn_session(...)` は `continue_session=true` のとき `--resume <uuid>`、`false` のとき `--session-id <uuid>` を渡す（`--session-id + --continue` の併用は claude が拒否する仕様。`session.rs::build_args` 内 doc コメント + 4 unit test 改訂）。保存タイミングは「`system/init` イベント到着でセッション確定」したフレーム — `Tab::session_confirmed: bool` が `drain_chat_events` で `ChatEvent::Init` を観測した時点で `true` に。`App::maybe_persist_active_session` が `logic()` 末で 1 度だけ `upsert + save_atomic`（`session_persisted_this_run` latch で抑制）。Cmd+T で生成する新規タブは常に `resume=None` で fresh session、saved id は触らない。実機 smoke で 1 回目 (new) → メッセージ送信 → 終了 → 2 回目 (resume) フローはユーザーインタラクティブ確認必要。Known limitation: claude 側で stored session が削除されると `--resume` が "No conversation found" stderr で失敗、Tab に `Disconnected` 表示が出る。自動 fallback は 9.X.2 で対応 | 2.6, 9.5 | ✓ |
| 9 | 9.17 | 選択範囲を出典付きで chat 入力欄に追記 | プレビューでテキスト選択中、`ctx.pointer_latest_pos()` 付近に `egui::Area` でフローティング「→ チャットへ」ボタンを表示 (egui の `LabelSelectionState::has_selection()` で判定)。クリックで `ChatQuoteState { Idle / PendingInject / AwaitingDrain }` 状態機械が起動: 次フレームの `logic()` で `Event::Copy` を input events に push → さらに次フレームで `OutputCommand::CopyText` を output から拾って active タブの `chat.input` に追記。`src/chat/quote.rs` の pure helper `format_quote_block(selection, source, filename)` と `unique_line_range(selection, source)` で `<file:foo.md L12-L15>\n> selected\n` 形式を組み立てる。行範囲は substring が source 内で一意 match の場合のみ付与、複数 match や未 match のときは file 名のみにフォールバック。CopyText は OS clipboard にも残す副作用あり (Cmd+C と同じ感覚)。14 unit test (range / format / 各 fallback) | 9.5 | ✓ |
| 9 | 9.20 | プレビュー → 読み取り専用ソースビューへの置換 | (2026-06-05 ユーザー判断) markdown プレビュー機能と markdown validation を両方 omit、左ペインを syntect ハイライト + 行番号 gutter の read-only ソースビューに置換。`src/preview/render.rs` を `egui_commonmark` ベース → 直接 `syntect` 5 を使う方式に全面書き換え（`HighlightLines::highlight_line` で 1 行ずつ色付け、`LayoutJob` に gutter+本文を append、`Label::selectable(true)` で表示）。`src/preview/image.rs` / `src/preview/link.rs` を全削除、`app.rs::dispatch_link_clicks` / `handle_link_click` も削除。`watcher.rs` から `IMAGE_EXTENSIONS` / `is_image_path` / 画像対応の classify 分岐を除去。`Cargo.toml`: `egui_commonmark` / `pulldown-cmark` / `open` を削除、`syntect = "5"` (default-fancy) を追加、`image` は `--enable-dev-tools` 用 `png` feature のみへ。テスト 171 件 green、screenshot helper は既知の intermittent hang で実機目視は取れず | 4.2, 4.3 | ✓ |
| 10 | 10.0 | Phase 10 設計合意 (2026-06-08 ユーザー対話で確定) | キーボードフル操作 + エディタの vim 化に踏み切る判断。設計詳細は §3 Phase 10 を参照。本セルは合意の根拠ログ | — | ✓ |
| 10 | 10.1 | vim engine 共通モジュール | `src/vim/mod.rs` に modal state machine (Normal/Insert/Visual) + cursor + 編集 op + undo stack。`apply_key(state, key) -> Action` の pure テスト可能な層を中心に組む。最初は preview への組み込みを前提に設計、後で chat にも流用可能な形に保つ | — | — |
| 10 | 10.2 | preview を編集可能化（基盤） | preview ペインを `egui::TextEdit::multiline` に置換、左に行番号 painter。常時編集可能、デフォルト Normal モード。`PreviewState::Loaded` の `LoadedDocument.text` を buffer として使う。**ユーザーの主要価値はここなので 10.1 直後に着手** | 9.20, 10.1 | — |
| 10 | 10.3 | preview に keystroke save | Insert 中 / Normal 中いずれもキーを押すたびに `fs::write` で disk 反映。`Tab::last_written_hash` を更新、自分の write の ProjectWatcher エコーは hash 一致で drop | 10.2 | — |
| 10 | 10.4 | Claude 競合検出 + 解決バナー | watcher イベントで disk hash != last_written_hash → 競合。preview ペイン上部にバナー: 「ディスクを読む (buffer 破棄して reload) / buffer を保つ (次回保存で上書き) / diff (MVP 後)」 | 10.3 | — |
| 10 | 10.5 | vim 検索 (`/` `n` `N`) | buffer 内の string 検索。Normal モードで `/` → 入力欄 → Enter で検索、n / N で次/前のマッチへジャンプ。一致箇所をハイライト。preview に対する vim の "編集体験" を一通り揃える | 10.1 | — |
| 10 | 10.6 | 編集中 auto-follow の確認モーダル | Claude が別 .md を書いた時、編集中なら「開きますか？ (Yes: 切替 / No: 留まる)」モーダル。No なら background tab で開くなどの選択肢は MVP 後 | 6.3, 10.2 | — |
| 10 | 10.7 | chat 入力欄に vim binding | preview で固まった vim engine を chat::view の TextEdit にも適用。Plain Enter で送信は維持（Insert モード中の Enter は改行、Normal モードでは送信 trigger を兼ねるか後で判断）。**preview を完成させて運用してみてから判断するのも可** | 10.1 | — |
| 10 | 10.8 | ペイン間 keynav | Cmd+1 (preview) / Cmd+2 (chat) / Cmd+3 (tree) でフォーカス移動。Cmd+1..9 のタブ切替は別レイヤー（既存）と整合させる。設計上の悩み所はあるが、Cmd+P / Cmd+E のような別バインドも候補 | — | — |
| 10 | 10.9 | file tree keynav | tree 内で j/k 移動、Enter で開く、Space で展開/折りたたみ、Esc でフォーカスを preview に戻す。tree が closed のときは Cmd+B で開いて自動フォーカス | 9.X.4 | — |
| 10 | 10.10 | 履歴ピッカー keynav | modal 内で j/k 移動、Enter で resume、Esc で閉じる。tab bar の「履歴」ボタンや Cmd+Y のような shortcut で開く | 9.19 | — |

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

### Phase 10: vim-style editor + キーボードフル操作

**着手前確認 (2026-06-08 ユーザー対話で確定):**

#### 保存方針

- **keystroke save**: キーを押すたびに `fs::write` で disk に反映
- 自分の write の echo は `last_written_hash` を覚えて ProjectWatcher イベントと比較、一致なら無視

#### Claude 競合

- 競合 = 「disk hash が `last_written_hash` と一致しない write イベント」
- 検出時は preview ペイン上部に競合バナー:
  - **ディスクを読む**: buffer 破棄して reload
  - **buffer を保つ**: 次回の save で disk を上書き（Claude の編集を失う）
  - **diff を見る**: MVP 後（バナー上はボタンを disabled で並べておく）

#### Editor UI

- preview ペインを `egui::TextEdit::multiline` に置換
- 行番号は左に Painter で描画（現状の Phase 9.20 連続線方式を流用）
- 常に編集可能。デフォルト Normal モード、`i` で Insert、`Esc` で Normal に戻る
- `PreviewStatus::Empty` のときは編集不可（保存先がないため）。`Loaded` のみ編集対象

#### vim binding スコープ

| モード | サポート |
|---|---|
| Normal | h/j/k/l, w/b/e, 0/$, gg/G, dd/yy/p, x, u/Ctrl+R, /, n/N, i/a/o |
| Insert | キー入力、Esc で Normal に戻る |
| Visual | h/j/k/l, y (yank), d (delete), Esc |
| Command (`:` プロンプト) | MVP 外 |
| Macro/レジスタ/マーク | MVP 外 |
| 検索 | `/` 入力欄 + n/N でジャンプ。一致箇所をハイライト |

#### Auto-follow during edit

- Claude が別 .md を書いた時、preview が編集中なら **モーダル** で「開きますか？」を確認
- Yes: 切替（keystroke save 済みなので buffer = disk、捨てて問題なし）
- No: 留まる。Claude が書いた .md は ignore

#### Cmd+R during edit

- buffer は keystroke save で常に disk と同期しているので、buffer 破棄 + disk から reload で可

#### 新規ファイル作成

- mdpilot では作れない。Claude に「README 作って」と頼む運用

#### キーボードナビ範囲

- ペイン間: Cmd+1 (preview) / Cmd+2 (chat) / Cmd+3 (tree)
  - 注: 既存の Cmd+1..9 タブ切替と整合させる必要あり。Cmd+1..9 はタブ、Cmd+Shift+1..3 はペインなど別系列にする案を 10.8 で詰める
- file tree: j/k 移動、Enter 開く、Space 展開、Esc preview に戻す
- 履歴ピッカー: j/k 移動、Enter resume、Esc 閉じる

#### 実装サブフェーズ（順序: 2026-06-08 ユーザー指示で preview 優先に並べ直し）

10.1 vim engine → 10.2 **preview 編集化** → 10.3 keystroke save →
10.4 Claude 競合 → 10.5 検索 → 10.6 auto-follow モーダル →
10.7 chat 適用 → 10.8 ペイン keynav → 10.9 tree keynav → 10.10 履歴 keynav

各サブフェーズで実機検証 + commit。**10.1-10.5 がコア**（preview の編集体験の
最低限）。10.6 以降は段階的に積む。10.7 (chat への vim 適用) は preview を
完成させて運用してみてから「本当にやる価値があるか」を判断する余地を残す。

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
| ~~`egui_commonmark` の GFM カバレッジと採用継続可否~~ | ~~4.2~~ | **obsolete** (2026-06-05): Phase 9.20 で `egui_commonmark` 自体を撤去、ソース表示に置換 | — | — |
| `tokio` 採用可否 | 全 Phase | `std::thread` で開始 | `architecture.md` 4, 9 章 | 必要が出たら再評価 |
| 設定ファイル形式 (TOML / JSON) | 9.3 | （Phase 9 で検討） | `architecture.md` 9 章 | Phase 9.3 着手前 |
| syntect ダーク/ライトテーマ名 | 4.3 | `preview.md` 4 章の例示値 | `preview.md` 4 章 | Phase 4 着手前 |
| アプリアイコン | 8.1 | 未準備 | `requirements.md` 8 章 | Phase 8 着手前 |
| ライセンス | 8.4 | ユーザー判断 | `requirements.md` 8 章 | Phase 8 完了前 |
| プロジェクト選択ダイアログ UI 仕様 | 6.1 | `rfd` のディレクトリ選択 1 回 | `claude-integration.md` 2 章 | Phase 6 着手前 |
| F-28 安全モードの UI 設計 | 9.10 | 後日設計 | `requirements.md` F-28 | Phase 9 着手前 |

各項目は対応フェーズの最初のサブタスクとして **ユーザー確認のステップ** を置く。
