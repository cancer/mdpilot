# 実装計画: mdpilot 全機能段階的実装

## 1. Context（背景・目的）

mdpilot は Claude Code と協調して Markdown を書くためのネイティブ GUI アプリ。設計フェーズ完了・実装未着手の状態（Cargo.toml も `src/` も存在せず、`docs/` のみ）から、**スケルトン → MVP → MVP 後の拡張**までを段階的に積む全体実装計画を本書にまとめる。

参照する仕様書（すべて `docs/` 配下）：

- [requirements.md](requirements.md): 機能要件 F-01〜F-10 (MVP), F-21〜F-27 (MVP 後), 非機能要件 N-01〜N-07, スコープ外
- [architecture.md](architecture.md): 単一プロセス・複数スレッド構成、モジュール分割、データフロー、依存クレート
- [ui.md](ui.md): 2 ペイン構成、ウィンドウ既定値、メニュー、キーバインド、フォーカス、テーマ
- [terminal.md](terminal.md): PTY 起動、シェル選択、ANSI 対応、コピー/貼り付け、スクロールバック、リサイズ、IME
- [preview.md](preview.md): CommonMark + GFM、syntect、リンク・画像、ファイル監視（F-08）、対象切替
- [claude-integration.md](claude-integration.md): 自動追従（F-09 案 A）、追従 ON/OFF、起動条件

設計方針の前提：

- MVP は F-01〜F-10 と各仕様書で「MVP 必須」と明記された項目に限定する
- F-09 は **案 A（自動追従）のみ** で MVP を成立させ、MCP/CLI シム/フックは MVP 後の拡張余地とする（`claude-integration.md` 5.3）
- 非機能要件 N-01〜N-07 は Phase 7（仕上げ）で測定・確認する
- Linux サポート・WYSIWYG・手動編集・プラグイン・自動アップデートは恒久的にスコープ外（`requirements.md` 6 章）

## 2. 対応一覧

各フェーズは前フェーズの完了を前提とする。同一フェーズ内のタスクは可能な範囲で並行可。

| Phase | タスク# | タスク名 | 概要 | 依存 |
|-------|---------|---------|------|------|
| 0 | 0.1 | リポジトリ初期化と Cargo パッケージ作成 | `Cargo.toml`, `src/main.rs`, `.gitignore` 等を作る。CI スクリプトと `rust-toolchain.toml` も含む | — |
| 0 | 0.2 | 最小限の eframe アプリ起動 | `eframe::run_native` で空ウィンドウを開く。タイトル「mdpilot」、既定サイズ 1400×900 | 0.1 |
| 0 | 0.3 | エラー型とロギング基盤 | `src/error.rs` (`thiserror`), `tracing-subscriber` 初期化 | 0.1 |
| 0 | 0.4 | 設定ディレクトリ解決 | `directories` で OS 別の config/data/cache パス取得（中身は空でも構造を用意） | 0.1 |
| 0.5 | 0.5.1 | egui_term 統合スパイク | 別ディレクトリの最小サンプルで PTY 起動 → `claude` 入力 → IME 日本語入力までを通す。不足機能を洗い出し | 0.2 |
| 0.5 | 0.5.2 | egui_commonmark 統合スパイク | 最小サンプルで GFM テーブル / タスクリスト / 取り消し線 / コードブロックを表示し、`preview.md` 2 章の表を実機で埋める | 0.2 |
| 0.5 | 0.5.3 | スパイク結果のレポートと採用判断 | 不足機能があれば `comrak` 切替・自前描画追加などの方針をユーザーと確認、本計画と仕様書に反映 | 0.5.1, 0.5.2 |
| 1 | 1.1 | レイアウト状態とペイン分割（F-01 前半） | `LayoutState`, `egui::SidePanel`/分割比率の保持 | 0.5.3 |
| 1 | 1.2 | 境界リサイズハンドル（F-01 後半） | マウスドラッグで比率変更、ダブルクリックで 1:1 リセット、最小幅 240px | 1.1 |
| 1 | 1.3 | プレビュー/ターミナルのプレースホルダ描画 | 「空ペイン」表示、ウィンドウタイトルは固定文字列 | 1.1 |
| 2 | 2.1 | PTY セッション抽象 | `portable-pty` で PTY 起動、`CommandBuilder` で OS 別シェル選択（macOS=`$SHELL`、Win=`pwsh`→`powershell`→`cmd`） | 0.3 |
| 2 | 2.2 | PTY 読込スレッドと mpsc 通知 | バックグラウンドスレッドで PTY マスタを読む、メインに `request_repaint` 通知 | 2.1 |
| 2 | 2.3 | egui_term による描画と入力 | 右ペインで `egui_term` を表示、キー入力を PTY に書き込み（F-02, F-03） | 1.3, 2.2 |
| 2 | 2.4 | ターミナル初期サイズ計算と SIGWINCH 連動 | ペインピクセル÷フォントメトリクスで rows/cols、`MasterPty::resize` 呼び出し、デバウンス 50ms | 2.3 |
| 3 | 3.1 | 選択・コピー・貼り付け（F-04） | ドラッグ選択、`Cmd+C` / `Cmd+V` (mac) / `Ctrl+Shift+C` / `Ctrl+Shift+V` (Win)。`Ctrl+C` は SIGINT 送信 | 2.3 |
| 3 | 3.2 | スクロールバック（F-05） | 既定 10,000 行、`Shift+PgUp/PgDn/Home/End`、末尾追従モード | 2.3 |
| 3 | 3.3 | IME 対応 | egui の `Event::Ime`、プリエディット表示、確定文字列の PTY 書き込み（N-07） | 2.3 |
| 3 | 3.4 | `claude` 自動起動 | シェル起動後に `claude` を起動する方式（a/b/c）を確定し実装（`terminal.md` 4 章） | 2.3 |
| 3 | 3.5 | 子プロセス終了表示と PTY クリーンアップ | 「[Process exited with code N]」表示、アプリ終了時の `SIGHUP` | 2.3 |
| 4 | 4.1 | ファイル読込ローダー | 指定パスから UTF-8 読込、サイズしきい値（1MB/10MB）でモード切替 | 0.3 |
| 4 | 4.2 | egui_commonmark 描画 | CommonMark + GFM (テーブル / タスクリスト / 取り消し線) 確認、不足機能の補強方針決定（F-06） | 1.3, 4.1 |
| 4 | 4.3 | syntect シンタックスハイライト統合 | `syntect` 同梱定義使用、ダーク/ライト 2 テーマ、1MB 超ブロックはフォールバック（F-07） | 4.2 |
| 4 | 4.4 | リンク挙動 | 外部 URL は OS 既定ブラウザ、相対 `.md` は対象切替、その他は OS 既定アプリ（`preview.md` 5 章） | 4.2 |
| 4 | 4.5 | 画像・相対パス解決 | ローカル相対/絶対パスを `egui` 画像 API で表示、HTTP/HTTPS は MVP 非対応（`preview.md` 6 章） | 4.2 |
| 4 | 4.6 | プレビューのスクロール位置保持 | 再読込前の最上端行を記憶、ベストエフォートで復元（`preview.md` 8 章） | 4.2 |
| 5 | 5.1 | `notify` Watcher セットアップ | バックグラウンドスレッドで `RecommendedWatcher`、mpsc でメインに通知 | 0.3 |
| 5 | 5.2 | 単一ファイル監視と再レンダリング（F-08） | プレビュー対象 1 個を監視、100ms デバウンス、ファイル削除時の「見つかりません」表示 | 4.2, 5.1 |
| 5 | 5.3 | 監視エラーのステータス表示 | 監視開始失敗をステータスバー/トーストに出す、`Cmd+R`/`Ctrl+R` で手動再読込 | 5.2 |
| 6 | 6.1 | プロジェクトルート解決 | 起動引数（`mdpilot <dir>` / `<file>` / 引数なし）から root を決定（`claude-integration.md` 2 章） | 0.4 |
| 6 | 6.2 | プロジェクト配下 `.md` の再帰監視 | 除外ディレクトリ（`.git`, `node_modules`, `target` 等）を除き再帰監視 | 5.1, 6.1 |
| 6 | 6.3 | 自動追従ロジック（F-09 案 A） | 「現在表示中以外の `.md` 書き換え」で対象切替、200ms デバウンス、複数更新は最新 mtime 採用（`claude-integration.md` 6 章） | 5.2, 6.2 |
| 6 | 6.4 | 起動直後の対象選択 | `<file>` 指定時はそのファイル、`<dir>` 指定時は `README.md` 検索、なければ空ペイン | 6.1, 6.3 |
| 6 | 6.5 | `MDPILOT_PROJECT_ROOT` 環境変数の付与 | `claude` 子プロセスに絶対パスを渡す（将来 IPC の足場） | 3.4, 6.1 |
| 7 | 7.1 | `Cmd+O`/`Ctrl+O` ファイル選択ダイアログ | `rfd` 等のクロスプラットフォーム dialog で `.md` 選択 | 4.2, 6.3 |
| 7 | 7.2 | 自動追従モード ON/OFF | `Cmd+O` で OFF、パスバーのボタンで再 ON（`claude-integration.md` 6.3） | 6.3, 7.1 |
| 7 | 7.3 | キーバインド統合 | `ui.md` 6 章のキーバインドをフォーカスペインに応じて解釈 | 3.1, 4.2 |
| 7 | 7.4 | ウィンドウタイトル動的更新 | 「mdpilot - <ファイル名>」を対象切替に応じて変更（`ui.md` 4 章） | 6.3 |
| 7 | 7.5 | macOS メニューバー | mdpilot / ファイル / 表示 / ウインドウ / ヘルプ（`ui.md` 5.1） | 7.1, 7.3 |
| 7 | 7.6 | Windows ツールバー | 開く / 再読込 / 情報 を最小ツールバーで提供（`ui.md` 5.2） | 7.1, 7.3 |
| 7 | 7.7 | パスバーとステータス表示 | プレビューファイルのフルパス、監視状態、エラートースト | 5.3, 6.3 |
| 7 | 7.8 | テーマ追従 | OS のダーク/ライトに追従、コードブロックテーマも連動 | 4.3 |
| 7 | 7.9 | 非機能要件の測定 | N-01〜N-04 を測定、超過があれば最適化 | 全 Phase |
| 8 | 8.1 | macOS バンドル | `cargo-bundle` で `.app` 生成、`aarch64`/`x86_64` 両対応 | 7.* |
| 8 | 8.2 | Windows バイナリ | `x86_64-pc-windows-msvc` ターゲットでビルドスクリプト整備 | 7.* |
| 8 | 8.3 | CI（GitHub Actions） | macOS + Windows の build/test/clippy/fmt を回す | 0.1 |
| 8 | 8.4 | リリース手順ドキュメント | `docs/release.md` に手順記述（タグ運用、配布物の作り方） | 8.1, 8.2 |
| 9 | 9.1 | F-21 リンク・画像（相対パス含む）の解決の精緻化 | HTTP 画像対応、画像の自動リロード | 4.4, 4.5 |
| 9 | 9.2 | F-22 スクロール位置の編集追従 | 編集差分から該当位置にスクロール（カーソル追従不要） | 4.6 |
| 9 | 9.3 | F-23 設定ファイル | フォント・配色・キーバインド・ペイン比率・行数等を上書き可能に | 0.4 |
| 9 | 9.4 | F-24 アプリメニュー拡充 | macOS の環境設定、Windows の正式メニューバー | 7.5, 7.6 |
| 9 | 9.5 | F-25 タブ管理 | 複数ターミナル・複数プレビューのタブ | 7.* |
| 9 | 9.6 | F-26 拡張: 数式・Mermaid・脚注 | KaTeX 相当・Mermaid・脚注を順次対応 | 4.2 |
| 9 | 9.7 | F-27 テーマ切替 | OS 追従に加え強制ライト/ダーク選択 | 7.8 |
| 9 | 9.8 | F-09 案 C（MCP サーバ）の後付け | 編集前のファイル指定や明示切替を MCP ツール経由で実現 | 6.3 |
| 9 | 9.9 | プレビュー内検索（`Cmd+F`） | プレビュー側の文字列検索とハイライト | 4.2 |
| 9 | 9.10 | ターミナル内検索 | スクロールバック内の検索 | 3.2 |
| 9 | 9.11 | ペイン比率の永続化・前回ウィンドウ位置復元 | 設定ファイル経由 | 9.3 |

## 3. 各タスクの詳細

### Phase 0: プロジェクトスケルトン

#### タスク 0.1: リポジトリ初期化と Cargo パッケージ作成
- **対象ファイル**:
  - `Cargo.toml`（新規）
  - `rust-toolchain.toml`（新規、`channel = "stable"`）
  - `.gitignore`（新規、`/target`, `*.swp` 等）
  - `src/main.rs`（新規、ひとまず `fn main() {}`）
- **作業内容**: `cargo new --bin mdpilot` 相当の構造を手で整える。依存はまだ空。`edition = "2021"` または `2024`。バイナリ名 `mdpilot`。
- **参考パターン**: `architecture.md` 2 章のモジュール構成を雛形に取り込む（空モジュールでも `mod.rs` を置く）。

#### タスク 0.2: 最小限の eframe アプリ起動
- **対象ファイル**: `src/main.rs`, `src/app.rs`（新規）
- **作業内容**: `eframe::run_native("mdpilot", NativeOptions { viewport: ViewportBuilder::default().with_inner_size([1400.0, 900.0]).with_min_inner_size([800.0, 500.0]), ..)` で空ウィンドウを開く。`App::update` は中央に「mdpilot」ラベルを描画するだけ。
- **参考パターン**: `architecture.md` 5 章の `App` 構造体の意図に沿うが、この段階はフィールド空でよい。

#### タスク 0.3: エラー型とロギング基盤
- **対象ファイル**: `src/error.rs`（新規）, `src/main.rs`（更新）
- **作業内容**:
  - `thiserror::Error` で `mdpilot::error::Error` を定義（`Io(std::io::Error)`, `Pty(String)`, `Watch(notify::Error)` などのバリアントを段階的に増やす）
  - `tracing-subscriber` の `fmt::init()` を `main` 冒頭で呼ぶ
  - 出力先（標準エラー / ファイル）は MVP では標準エラー固定（未確定事項に残す）
- **参考パターン**: `architecture.md` 6 章「panic は `std::panic::set_hook` で捕捉」を組み込む。

#### タスク 0.4: 設定ディレクトリ解決
- **対象ファイル**: `src/config/mod.rs`（新規）, `src/config/paths.rs`（新規）
- **作業内容**: `directories::ProjectDirs::from("dev", "mdpilot", "mdpilot")` で config/data/cache パスを返すヘルパ。中身は空で構わない。
- **参考パターン**: `architecture.md` 2 章のモジュール構成、依存ライブラリ一覧。

### Phase 0.5: 主要依存クレートの統合スパイク

仕様書が「暫定」「検討中」「実装着手時に検証」とした 3 クレート（`egui_term` / `egui_commonmark` / `syntect`）が要件を満たすかを Phase 1 着手前に検証する。失敗時は採用クレートの差し替え判断を Phase 2 着手前に済ませる。

#### タスク 0.5.1: egui_term 統合スパイク
- **対象ファイル**: `spike/egui_term/`（一時ディレクトリ、本リポジトリの `src/` には入れない）
- **作業内容**:
  - 最小サンプルで `egui_term` の `TerminalView` を起動 → PTY 経由でログインシェル → `claude` を起動
  - 日本語 IME 入力（プリエディット表示・確定）を実機検証
  - マウス選択・コピー・貼り付けの挙動を確認
  - 不足機能をリストアップ
- **判定基準**: IME・基本入出力が動くなら採用継続。動かないなら自前 IME 補強の作業量見積もり、または別ウィジェット採用を検討。

#### タスク 0.5.2: egui_commonmark 統合スパイク
- **対象ファイル**: `spike/egui_commonmark/`（一時ディレクトリ）
- **作業内容**:
  - 最小サンプルで CommonMark + GFM (テーブル / タスクリスト / 取り消し線 / 自動リンク / 脚注) を描画
  - `syntect` をコードブロックに適用
  - 画像・相対パスリンクの挙動を確認
  - `preview.md` 2 章「対応 Markdown 仕様」の表を実機の挙動で埋める
- **判定基準**: MVP 必須 (テーブル / タスクリスト / 取り消し線) が出れば採用継続。出ない要素があれば `comrak` + 自前描画への切替を検討。

#### タスク 0.5.3: スパイク結果のレポートと採用判断
- **対象ファイル**: `docs/spike-report.md`（新規）
- **作業内容**:
  - 0.5.1 / 0.5.2 の結果をレポート化
  - 採用クレートの最終決定、差し替えが必要なら本計画書および該当仕様書（`terminal.md` / `preview.md` / `architecture.md`）を更新
  - ユーザーに採用判断を確認
- **判定基準**: ユーザーが採用方針に同意したら Phase 1 着手。

### Phase 1: 2 ペインレイアウト (F-01)

#### タスク 1.1: レイアウト状態とペイン分割
- **対象ファイル**: `src/ui/mod.rs`（新規）, `src/ui/layout.rs`（新規）, `src/app.rs`（更新）
- **作業内容**: `LayoutState { left_ratio: f32 }` を `App` に持たせ、`egui::SidePanel::left("preview").resizable(true).default_width(viewport.x * 0.5)` 相当で左右分割。
- **参考パターン**: egui の `SidePanel` か手動分割（`Splitter`）。シンプルさ優先で `SidePanel` を使い、最小幅は `min_width(240.0)`。

#### タスク 1.2: 境界リサイズハンドル
- **対象ファイル**: `src/ui/layout.rs`（更新）
- **作業内容**:
  - マウスドラッグでの比率変更は `SidePanel` 標準機能で OK
  - 境界ダブルクリックの検出 → 1:1 リセット
  - `Cmd+\` / `Ctrl+\` でも 1:1 リセット（後で `7.3` で配線）
- **参考パターン**: `ui.md` 3 章。

#### タスク 1.3: プレビュー/ターミナルのプレースホルダ描画
- **対象ファイル**: `src/ui/preview_pane.rs`（新規）, `src/ui/terminal_pane.rs`（新規）
- **作業内容**: 各ペインに「プレビューがまだ開かれていません」「ターミナルを準備中…」のラベルを置く。後の Phase で実装に置き換わる。

### Phase 2: 内蔵ターミナル基礎 (F-02, F-03)

#### タスク 2.1: PTY セッション抽象
- **対象ファイル**: `src/terminal/mod.rs`（新規）, `src/terminal/session.rs`（新規）
- **作業内容**:
  - `portable_pty::native_pty_system().openpty(PtySize { rows, cols, .. })` でマスタ/スレイブ取得
  - シェル選択ロジック（順序は 6 章「着手前に確定すべき項目」で確定後に実装）：
    - macOS: `terminal.md` 3 章記載の通り `$SHELL` 優先・フォールバック `/bin/zsh` → `/bin/bash`
    - Windows: `terminal.md` 3 章の暫定順序をユーザー確認後に実装
  - `CommandBuilder::new(shell)` を `slave.spawn_command(builder)` で起動
  - cwd は Phase 6 でプロジェクトルートに差し替えるが、ここでは `std::env::current_dir()` を渡しておく
- **参考パターン**: `terminal.md` 2-3 章。

#### タスク 2.2: PTY 読込スレッドと mpsc 通知
- **対象ファイル**: `src/terminal/session.rs`（更新）
- **作業内容**:
  - `std::thread::spawn` で PTY マスタの `Read` を回す
  - `alacritty_terminal::Term` にバイト列を流し、グリッド更新
  - `mpsc::Sender<AppEvent>` で「ターミナル更新あり」を送る
  - メイン側は `ctx.request_repaint()` を呼ぶ
- **参考パターン**: `architecture.md` 3.2, 4 章。

#### タスク 2.3: egui_term による描画と入力
- **対象ファイル**: `src/terminal/view.rs`（新規）, `src/ui/terminal_pane.rs`（更新）
- **作業内容**:
  - `egui_term` の `TerminalView` ウィジェットを右ペインに配置
  - キーイベントを `alacritty_terminal::input` 経由で ANSI 化し PTY に書き込み
  - 修飾キー（Shift / Alt / Ctrl / Cmd）を正しく扱う
- **参考パターン**: `terminal.md` 5, 10 章。`egui_term` のサンプルを起点にする。

#### タスク 2.4: ターミナル初期サイズ計算と SIGWINCH 連動
- **対象ファイル**: `src/terminal/session.rs`（更新）, `src/terminal/view.rs`（更新）
- **作業内容**:
  - フォントメトリクスから 1 文字の幅・高さを取得し、ペインピクセルサイズから rows/cols 算出
  - ペイン幅・ウィンドウ高さ変更時に `MasterPty::resize(PtySize { ... })`
  - 50ms デバウンス（連続リサイズで頻発しないよう）
- **参考パターン**: `terminal.md` 8 章。

### Phase 3: ターミナル機能拡充 (F-04, F-05, N-07)

#### タスク 3.1: 選択・コピー・貼り付け
- **対象ファイル**: `src/terminal/view.rs`（更新）
- **作業内容**:
  - マウスドラッグで連続選択、ダブルクリックで単語選択、トリプルクリックで行選択
  - `Cmd+C` (mac) / `Ctrl+Shift+C` (Win) で選択範囲を `egui::Output::copied_text` に
  - `Cmd+V` / `Ctrl+Shift+V` で `egui::Event::Paste` を PTY に書き込み（bracketed paste 対応）
  - `Ctrl+C` は ETX (0x03) を PTY に送る
- **参考パターン**: `terminal.md` 6 章, `ui.md` 6.1 章。

#### タスク 3.2: スクロールバック
- **対象ファイル**: `src/terminal/session.rs`（更新）, `src/terminal/view.rs`（更新）
- **作業内容**: `alacritty_terminal::Term` のスクロールバック行数を 10,000 に設定、`Shift+PgUp/PgDn/Home/End` でスクロール、末尾追従モードはスクロール位置で判定。
- **参考パターン**: `terminal.md` 7 章。

#### タスク 3.3: IME 対応
- **対象ファイル**: `src/terminal/view.rs`（更新）
- **作業内容**:
  - `egui::Event::Ime(ImeEvent::Preedit(s))` をプリエディット表示
  - `ImeEvent::Commit(s)` で確定文字列を PTY 書き込み
  - `egui_term` の IME 実装が不足する場合は自前で補強
  - macOS/Windows 両方で日本語入力を実機検証
- **参考パターン**: `terminal.md` 10 章, `requirements.md` N-07。

#### タスク 3.4: `claude` 自動起動
- **対象ファイル**: `src/terminal/session.rs`（更新）
- **作業内容**: `terminal.md` 4 章の方式 a / b / c のいずれを採用するかを 6 章「着手前に確定すべき項目」でユーザーと決定したうえで実装。
- **参考パターン**: `terminal.md` 4 章, `claude-integration.md` 7 章。

#### タスク 3.5: 子プロセス終了表示と PTY クリーンアップ
- **対象ファイル**: `src/terminal/session.rs`（更新）
- **作業内容**: 子プロセス終了を `Child::try_wait` で監視し、終了したらターミナル末尾に「[Process exited with code N]」を追加描画。`App` の `Drop` で PTY を閉じ、`SIGHUP` 相当を送る。
- **参考パターン**: `terminal.md` 11 章。

### Phase 4: Markdown プレビュー (F-06, F-07)

#### タスク 4.1: ファイル読込ローダー
- **対象ファイル**: `src/preview/mod.rs`（新規）, `src/preview/loader.rs`（新規）
- **作業内容**:
  - UTF-8 で読み込み、BOM があれば剥がす
  - 1MB 未満は通常レンダリング、1MB 以上 10MB 未満は警告ステータス付き、10MB 以上はエラー表示
- **参考パターン**: `preview.md` 10 章。

#### タスク 4.2: egui_commonmark 描画
- **対象ファイル**: `src/preview/render.rs`（新規）, `src/ui/preview_pane.rs`（更新）
- **作業内容**:
  - `egui_commonmark::CommonMarkViewer::new("preview").show(ui, &mut cache, &markdown)` で描画
  - 着手直後に GFM テーブル / タスクリスト / 取り消し線が出るか検証
  - 不足機能があれば `pulldown-cmark` の AST から自前描画を追加、または `comrak` に置換
- **参考パターン**: `preview.md` 2-3 章。

#### タスク 4.3: syntect シンタックスハイライト統合
- **対象ファイル**: `src/preview/render.rs`（更新）
- **作業内容**:
  - `syntect::parsing::SyntaxSet::load_defaults_newlines()` でシンタックス取得
  - ダーク/ライトテーマの具体的なテーマ名は 6 章「着手前に確定すべき項目」でユーザーと決定（`preview.md` 4 章は「既定: ダーク=`base16-ocean.dark`、ライト=`InspiredGitHub` 程度」と例示のみ）
  - コードブロックの info string から `find_syntax_by_token` で言語判定
  - 1MB 超ブロックはハイライトせずプレーンテキスト
- **参考パターン**: `preview.md` 4 章。

#### タスク 4.4: リンク挙動
- **対象ファイル**: `src/preview/render.rs`（更新）
- **作業内容**:
  - 外部 URL → `open::that(url)` で OS 既定ブラウザ起動
  - 相対 `.md` → プレビュー対象切替（Phase 6 の自動追従と独立に動かす）
  - 相対の `.md` 以外 → `open::that(path)` で OS 既定アプリ
  - アンカー `#section` → 同一プレビュー内スクロール
  - `mailto:` → `open::that` で OS 既定メーラ
- **参考パターン**: `preview.md` 5 章。

#### タスク 4.5: 画像・相対パス解決
- **対象ファイル**: `src/preview/render.rs`（更新）
- **作業内容**:
  - 相対パスはプレビューファイルのディレクトリ基準で `std::fs::read` → `egui::ColorImage` 経由で描画
  - HTTP/HTTPS は MVP 非対応、`alt` を代替表示
  - `data:` URL は `egui` の `Image` API に渡す
  - SVG/PNG/JPEG/GIF/WebP に対応（`egui` の対応に従う）
- **参考パターン**: `preview.md` 6 章。

#### タスク 4.6: プレビューのスクロール位置保持
- **対象ファイル**: `src/preview/render.rs`（更新）
- **作業内容**:
  - 再読込前の最上端の論理行番号を記憶
  - 再描画後に該当行へ `scroll_to_cursor`/`scroll_to_rect` でスクロール
  - 行が消えていれば最も近い行にフォールバック
- **参考パターン**: `preview.md` 8 章。

### Phase 5: ファイル監視と自動リロード (F-08)

#### タスク 5.1: `notify` Watcher セットアップ
- **対象ファイル**: `src/preview/watcher.rs`（新規）
- **作業内容**:
  - `notify::recommended_watcher` で監視ハンドルを作る
  - バックグラウンドスレッドで `Event` を受信し、mpsc でメインに転送
  - 共通 `AppEvent::FileChanged(PathBuf)` バリアントを `error.rs` 隣の `events.rs`（新規）に定義
- **参考パターン**: `architecture.md` 3.3, 4 章。

#### タスク 5.2: 単一ファイル監視と再レンダリング
- **対象ファイル**: `src/preview/watcher.rs`（更新）, `src/preview/mod.rs`（更新）
- **作業内容**:
  - 表示対象 1 個のみ監視（非再帰）
  - 100ms デバウンス（連続書き込みを集約）
  - ファイル削除を `EventKind::Remove` で検知し、「ファイルが見つかりません」表示
  - 再作成（`Create`）を検知して自動再開
- **参考パターン**: `preview.md` 7 章。

#### タスク 5.3: 監視エラーのステータス表示
- **対象ファイル**: `src/ui/preview_pane.rs`（更新）
- **作業内容**: 監視開始失敗時はステータスバー（左ペイン上部）にエラー文言を出し、`Cmd+R`/`Ctrl+R` で手動再読込できるよう導線を明示。

### Phase 6: Claude 自動追従 (F-09 案 A)

#### タスク 6.1: プロジェクトルート解決
- **対象ファイル**: `src/main.rs`（更新）, `src/app.rs`（更新）, `src/config/paths.rs`（更新）
- **作業内容**:
  - `mdpilot <dir>` → そのディレクトリ
  - `mdpilot <file.md>` → 親ディレクトリをプロジェクトルート、初期表示はそのファイル
  - 引数なし → ファイル選択ダイアログを起動時に出し、選んだ親をルートに（MVP では「プロジェクト選択ダイアログ」は単純な `rfd` の選択 1 回で済ませる）
- **参考パターン**: `claude-integration.md` 2 章。

#### タスク 6.2: プロジェクト配下 `.md` の再帰監視
- **対象ファイル**: `src/preview/watcher.rs`（更新）
- **作業内容**:
  - 再帰監視 Watcher を別系統で持つ（F-08 の単一監視とは別インスタンス、または同一 Watcher 内の別パス登録）
  - 除外ディレクトリ: `.git/`, `node_modules/`, `target/`, `dist/`, `build/`, `.next/`, `.svelte-kit/`, `.venv/`, `__pycache__/`
  - 大文字小文字を区別しない拡張子フィルタ（`.md`, `.markdown`）
- **参考パターン**: `claude-integration.md` 6.1 章。

#### タスク 6.3: 自動追従ロジック
- **対象ファイル**: `src/app.rs`（更新）, `src/preview/mod.rs`（更新）
- **作業内容**:
  - 現在表示中ファイルが書き換わった場合: 切替なし（F-08 で再レンダリング）
  - 現在表示中以外の `.md` が書き換わった場合: 200ms デバウンス後、最新 mtime のファイルに切替、F-08 用 Watcher を張り替え
  - 自動追従モードのフラグ（後の 7.2 で UI に出す）
- **参考パターン**: `claude-integration.md` 6.2, 6.4 章。

#### タスク 6.4: 起動直後の対象選択
- **対象ファイル**: `src/app.rs`（更新）
- **作業内容**:
  - `<file>` 指定時はそのファイル
  - `<dir>` 指定時はルート直下 `README.md`（大文字小文字を区別しない検索）→ 無ければ空ペイン
  - 空ペイン状態でも自動追従は有効
- **参考パターン**: `claude-integration.md` 6.5 章。

#### タスク 6.5: `MDPILOT_PROJECT_ROOT` 環境変数の付与
- **対象ファイル**: `src/terminal/session.rs`（更新）
- **作業内容**: `CommandBuilder::env("MDPILOT_PROJECT_ROOT", project_root.absolute())` を `spawn` 前に呼ぶ。
- **参考パターン**: `claude-integration.md` 3 章。

### Phase 7: UX 仕上げ

#### タスク 7.1: `Cmd+O`/`Ctrl+O` ファイル選択ダイアログ
- **対象ファイル**: `src/ui/mod.rs`（更新）
- **作業内容**: `rfd::FileDialog::new().add_filter("Markdown", &["md", "markdown"]).pick_file()` でファイル選択。
- **参考パターン**: `ui.md` 6.2 章, `preview.md` 9 章。

#### タスク 7.2: 自動追従モード ON/OFF
- **対象ファイル**: `src/ui/preview_pane.rs`（更新）, `src/app.rs`（更新）
- **作業内容**:
  - `Cmd+O` で明示選択した瞬間に追従 OFF
  - 左ペイン上部のパスバーに「Claude の編集を追従」トグルボタン
  - OFF 中は 6.3 のロジックを無効化
- **参考パターン**: `claude-integration.md` 6.3 章。

#### タスク 7.3: キーバインド統合
- **対象ファイル**: `src/app.rs`（更新）
- **作業内容**:
  - フォーカスペインを `App` で持ち、キー解釈を切り替える
  - アプリ全体: `Cmd+Q` / `Alt+F4`, `Cmd+O` / `Ctrl+O`, `Cmd+R` / `Ctrl+R`, `Cmd+\` / `Ctrl+\`
  - ペイン依存: `Cmd+C` / `Ctrl+Shift+C`, `Ctrl+C`, `Cmd+V` / `Ctrl+Shift+V`
- **参考パターン**: `ui.md` 6 章。

#### タスク 7.4: ウィンドウタイトル動的更新
- **対象ファイル**: `src/app.rs`（更新）
- **作業内容**: `ctx.send_viewport_cmd(ViewportCommand::Title(format!("mdpilot - {}", filename)))` を対象切替で呼ぶ。未指定時は「mdpilot」。
- **参考パターン**: `ui.md` 4 章。

#### タスク 7.5: macOS メニューバー
- **対象ファイル**: `src/ui/menu_macos.rs`（新規）
- **作業内容**:
  - eframe の標準メニューサポートを使うか、`muda` クレートで OS ネイティブメニュー
  - mdpilot / ファイル / 表示 / ウインドウ / ヘルプ の階層を組む
  - 日本語表記を遵守
- **参考パターン**: `ui.md` 5.1 章。

#### タスク 7.6: Windows ツールバー
- **対象ファイル**: `src/ui/toolbar_windows.rs`（新規）
- **作業内容**: ウィンドウ上部に最小ツールバー（開く / 再読込 / 情報）を `TopBottomPanel::top` で描画。
- **参考パターン**: `ui.md` 5.2 章。

#### タスク 7.7: パスバーとステータス表示
- **対象ファイル**: `src/ui/preview_pane.rs`（更新）
- **作業内容**:
  - 左ペイン上部にプレビューファイルのフルパス
  - 監視状態（監視中 / エラー）バッジ
  - エラー時はトースト表示（`egui-toast` または自前簡易実装）
- **参考パターン**: `ui.md` 9 章。

#### タスク 7.8: テーマ追従
- **対象ファイル**: `src/app.rs`（更新）, `src/preview/render.rs`（更新）
- **作業内容**:
  - eframe の `Theme::FollowSystem` を採用
  - syntect のテーマもダーク/ライトに連動して切替
- **参考パターン**: `ui.md` 8 章。

#### タスク 7.9: 非機能要件の測定
- **対象ファイル**: `docs/perf.md`（新規）
- **作業内容**: 以下を実測しレポート化：
  - N-01: 起動から操作可能まで（リリースビルドで 3 秒以内）
  - N-02: ターミナル入力の表示遅延（50ms 以下）
  - N-03: claude のストリーミング追従
  - N-04: 1 万字 Markdown の再レンダリング時間（100ms 以内）
  - 超過があれば最適化（描画キャッシュ、差分レンダリング等）

### Phase 8: ビルド・配布

#### タスク 8.1: macOS バンドル
- **対象ファイル**: `Cargo.toml`（更新）, `assets/Info.plist`（新規）, `scripts/build-macos.sh`（新規）
- **作業内容**:
  - `cargo-bundle` 設定を `[package.metadata.bundle]` に記述
  - `aarch64-apple-darwin` と `x86_64-apple-darwin` の両方をビルドし `lipo` で結合（または個別配布）
- **参考パターン**: `architecture.md` 8 章。

#### タスク 8.2: Windows バイナリ
- **対象ファイル**: `scripts/build-windows.ps1`（新規）
- **作業内容**: `x86_64-pc-windows-msvc` ターゲット、`cargo build --release` を回し、必要に応じて `.exe` を配布。署名は当面行わない。

#### タスク 8.3: CI（GitHub Actions）
- **対象ファイル**: `.github/workflows/ci.yml`（新規）
- **作業内容**:
  - matrix: `macos-latest`, `windows-latest`
  - jobs: `cargo fmt --check`, `cargo clippy -- -D warnings`, `cargo test`, `cargo build --release`
  - キャッシュ: `Swatinem/rust-cache`

#### タスク 8.4: リリース手順ドキュメント
- **対象ファイル**: `docs/release.md`（新規）
- **作業内容**: タグ運用（`v0.1.0` 形式）、`cargo-bundle` 実行手順、Windows 向けビルド手順、配布物のチェックサム作成。

### Phase 9: MVP 後（拡張）

各サブタスクは独立に進められる。優先順位はユーザー需要次第。

#### タスク 9.1 〜 9.11: `requirements.md` 4.2 章 (F-21〜F-27) および各仕様書の「MVP 後」項目
- 詳細は対応する仕様書セクションを参照（表 2 章末尾）

## 4. 修正対象ファイル一覧

### 新規作成（Phase 0-7 で順次）

- `Cargo.toml` — クレート定義と依存
- `rust-toolchain.toml` — stable 固定
- `.gitignore` — `/target`, IDE ファイル等
- `src/main.rs` — エントリポイント
- `src/app.rs` — `App` 構造体、`eframe::App` 実装、状態統合
- `src/error.rs` — クレート共通エラー型
- `src/events.rs` — `AppEvent` 列挙（ターミナル更新・ファイル変更通知）
- `src/ui/mod.rs` — UI モジュール束ね
- `src/ui/layout.rs` — 2 ペイン分割、リサイズ
- `src/ui/preview_pane.rs` — 左ペイン
- `src/ui/terminal_pane.rs` — 右ペイン
- `src/ui/menu_macos.rs` — macOS メニュー（Phase 7）
- `src/ui/toolbar_windows.rs` — Windows ツールバー（Phase 7）
- `src/terminal/mod.rs` — ターミナルモジュール
- `src/terminal/session.rs` — PTY 起動・読み書き
- `src/terminal/view.rs` — egui_term 描画
- `src/preview/mod.rs` — プレビューモジュール
- `src/preview/loader.rs` — ファイル読込
- `src/preview/watcher.rs` — notify Watcher（単一 + 再帰）
- `src/preview/render.rs` — egui_commonmark + syntect 描画
- `src/claude/mod.rs` — Claude 連携（Phase 6 で `MDPILOT_PROJECT_ROOT` 付与・自動追従連携）
- `src/config/mod.rs` — 設定モジュール
- `src/config/paths.rs` — OS 別パス解決
- `assets/Info.plist` — macOS バンドル用
- `scripts/build-macos.sh`, `scripts/build-windows.ps1` — ビルドスクリプト
- `.github/workflows/ci.yml` — CI
- `docs/perf.md` — 非機能要件測定レポート（Phase 7.9）
- `docs/release.md` — リリース手順（Phase 8.4）
- `docs/spike-report.md` — Phase 0.5 のスパイク結果レポート
- `spike/egui_term/`, `spike/egui_commonmark/` — Phase 0.5 の一時スパイクコード（Phase 1 着手時にレポートだけ残して削除可）

### 既存更新

- `docs/plan.md` — 本書（実装進捗に応じて更新）
- `README.md` — 「設計フェーズ」を「実装中」「MVP リリース済み」等に更新（Phase 8 完了時）

## 5. 検証方法

### コマンドベース検証（各 Phase 共通）

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build --release
```

### Phase 別の手動検証

- **Phase 0**: `cargo run` で空ウィンドウが 1400×900 で開く。タイトルが「mdpilot」
- **Phase 0.5**:
  - `spike/egui_term` のサンプルが起動し、PTY 経由で `claude` が動く。日本語 IME 入力が破綻しない
  - `spike/egui_commonmark` のサンプルで GFM テーブル / タスクリスト / 取り消し線 / コードブロックが描画される
  - `docs/spike-report.md` にカバレッジ表と採用判断が記録され、ユーザーが採用方針に同意
- **Phase 1**: 境界をドラッグして比率が変わる。ダブルクリックで 1:1 に戻る。最小幅 240px を割らない
- **Phase 2**: 右ペインに既定シェルのプロンプトが出る。`ls` 等が動作。色付き出力が正しく見える
- **Phase 3**:
  - 選択 → `Cmd+C` でコピーできる
  - `Cmd+V` でクリップボードからペースト
  - `Ctrl+C` でフォアグラウンドプロセスに SIGINT
  - `Shift+PgUp` でスクロールバック
  - 日本語入力で変換中表示が崩れない、確定が PTY に届く
  - シェル起動後に `claude` が自動で起動（Claude Code がインストール済みのマシンで確認）
- **Phase 4**:
  - `mdpilot README.md` でプレビューが描画される
  - テーブル / タスクリスト / 取り消し線が表示される
  - コードブロックがハイライトされる
  - 外部 URL クリックでブラウザが開く
  - 相対 `.md` リンクで対象切替
  - 相対画像が表示される
- **Phase 5**:
  - 外部エディタで `.md` を保存するとプレビューが自動更新
  - ファイル削除で「見つかりません」表示
  - 再作成で自動復帰
- **Phase 6**:
  - `mdpilot some-dir` で起動 → ルート直下 `README.md` が表示
  - claude が別ファイル（`docs/install.md`）を新規作成 → プレビューが切替
  - 連続編集中も 200ms デバウンスで最新ファイルだけに切替
- **Phase 7**:
  - `Cmd+O` でファイル選択 → 追従 OFF、ボタン押下で ON
  - メニュー / ツールバーが日本語表記
  - OS のダーク/ライト切替に追従
  - 非機能要件 N-01〜N-04 を `docs/perf.md` に記録
- **Phase 8**:
  - macOS: `.app` が起動する
  - Windows: `.exe` が起動する
  - CI が macOS / Windows 両方で green

### 機能要件カバレッジ確認

実装完了時点で `requirements.md` の F-01〜F-10 をチェックリスト形式で全項目満たすことを確認。非機能要件は以下の方法で確認する：

- **N-05（異常終了で編集データを失わない）**: mdpilot が手動編集機能を持たない設計（`requirements.md` 6 章、`ui.md` 4 章）により成立する性質。Phase 7 のアーキテクチャレビューで「mdpilot から `.md` への書込み経路が存在しない」ことを `grep` 等で確認する
- **N-06（単一実行可能ファイル配布）**: Phase 8 のビルド・配布時点で確認
- **N-07（IME 日本語入力）**: Phase 3.3 完了時に実機で確認
- **N-01〜N-04（性能要件）**: Phase 7.9 で測定し `docs/perf.md` に記録

## 6. 推奨する実行方法

本計画は **10 フェーズ・約 60 タスク** にわたる大規模実装である。以下を推奨する：

### 全体方針

- **`/team-manager` の使用を推奨**：フェーズ単位でマネージャーが進捗管理し、フェーズ内の独立タスクをサブエージェントに割り振る運用に適している
- 各タスクの完了ごとに `git commit`、機能ごとに作業粒度とコミット粒度を合わせる（CLAUDE.md の指示に従う）
- フェーズ完了ごとに `code-review` / `pair-review` でセルフレビュー → PR

### フェーズごとの推奨運用

| Phase | 推奨実行手段 | 理由 |
|-------|------------|------|
| 0 | `/implement-issue` または `/write-code` を直列で | スケルトン構築は逐次依存が強い |
| 0.5 | `/team-manager` | 0.5.1（ターミナル）と 0.5.2（プレビュー）は独立に並行検証可能、0.5.3 で集約 |
| 1 | `/implement-issue` または `/write-code` を直列で | レイアウトは逐次依存が強い |
| 2, 3 | `/team-manager` | PTY セッション基盤の上に複数機能（コピペ/スクロール/IME/claude 起動）が並行可 |
| 4 | `/team-manager` | 4.1〜4.6 は描画基盤の上に独立タスクが乗る |
| 5, 6 | `/implement-issue` を直列で | 監視ロジックは順序依存が強い |
| 7 | `/team-manager` | UX 機能は独立性が高く並列化に向く |
| 8 | `/implement-issue` 直列 | ビルド整備は構成変更が衝突しやすい |
| 9 | フェーズ完了後にユーザー需要に応じて issue 起票（`/create-general-issue`） | 拡張は需要ドリブンで進める |

### 着手前に確定すべき項目（実装着手時に決める）

仕様書の「未確定事項」セクションから本計画に影響するもの。**いずれも仕様書では未確定**であり、エージェント側で勝手に確定させず、対応フェーズの着手前にユーザー判断を仰ぐ。エージェントは比較材料と暫定案を提示することはあるが、確定はユーザーが行う。

| 項目 | 関連タスク | エージェント暫定案 | 仕様書の根拠 | 確定タイミング |
|------|----------|------------------|------------|--------------|
| `claude` 自動起動の実装方式 (a/b/c) | 3.4 | b（`claude\n` 書き込み）案。シェル初期化を経由しつつ OS 差異が小さい点を理由付け | `terminal.md` 4 章 | Phase 3 着手前 |
| マウスレポートのデフォルト ON/OFF | 3.* | 実機検証次第。スパイク（0.5.1）の所見を踏まえて判断 | `terminal.md` 5 章「MVP では対応」と記載のみ | Phase 3 着手前 |
| `egui_commonmark` の GFM カバレッジと採用継続可否 | 4.2 | Phase 0.5.2 のスパイク結果に基づき判断 | `preview.md` 3 章 | Phase 0.5 完了時 |
| `tokio` 採用可否 | 全 Phase | `std::thread` + `std::sync::mpsc` で開始、必要になれば導入 | `architecture.md` 4, 9 章 | Phase 5/6 着手前 |
| 設定ファイル形式 (TOML / JSON) | 9.3 | （Phase 9 で改めて検討） | `architecture.md` 9 章, `requirements.md` 8 章 | Phase 9.3 着手前 |
| ロギング出力先 | 0.3 | MVP は標準エラー固定 | `architecture.md` 9 章 | Phase 0 着手前 |
| syntect ダーク/ライトテーマ名 | 4.3 | 仕様書の例示（`base16-ocean.dark` / `InspiredGitHub`） | `preview.md` 4 章「程度」と例示のみ | Phase 4 着手前 |
| Windows での既定シェル順序 | 2.1 | 仕様書暫定の `pwsh`→`powershell`→`cmd` をそのまま | `terminal.md` 3 章「暫定」と記載 | Phase 2 着手前 |
| アプリアイコン | 8.1 | （素材未準備のため Phase 8 着手前にユーザー判断） | `requirements.md` 8 章 | Phase 8 着手前 |
| ライセンス | 8.4 | （ユーザー判断） | `README.md` / `requirements.md` 8 章 | Phase 8 完了前 |
| プロジェクト選択ダイアログ UI 仕様 | 6.1 | MVP は `rfd` のディレクトリ選択 1 回で代用 | `claude-integration.md` 2 章「プロジェクト選択ダイアログを表示」のみ | Phase 6 着手前 |

各項目は対応フェーズの最初のサブタスクとして **ユーザー確認のステップ** を置き、その結果を本書および該当仕様書に反映する。エージェント暫定案は判断材料であり、確定値ではない。
