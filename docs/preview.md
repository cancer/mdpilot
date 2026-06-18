# プレビューペイン仕様

## 0. 重要な仕様変更（2026-06-05）

ユーザー判断により、**markdown プレビュー（rendered view）は実装しない**。
左ペインは markdown ソースを syntect でハイライトして行番号付きで表示する
**read-only ソースビュー**に置き換わった。

理由・背景:

- 「markdown としての validation」を実装する案も検討したが、pulldown-cmark /
  markdown-rs / comrak のいずれも markdown spec の寛容性に従う設計のため
  「syntax 異常」として自動検出できるものは限定的（未解決参照リンクなど）。
  ROI が低く、validation 機能も同時に omit
- 「重要なのは Claude Code でドキュメントを書けること、書いたドキュメントを
  すぐさま目視できること」（ユーザー発言）→ rendered preview は必須ではなく、
  ソース表示で目視できれば十分という判断

このドキュメントの旧バージョン（egui_commonmark で render markdown を扱う
仕様）は git history で参照可能。

## 1. 概要

左ペインは現在開いている markdown ファイルのソースを **vim 風モーダルエディタ** として表示・編集する。

- syntect で markdown syntax highlight、左カラムに行番号 (gutter)
- vim Normal モードがデフォルト (`i`/`a`/`o` で Insert、`v`/`V` で Visual)
- キー入力ごとに `fs::write` で disk に反映 (Phase 10.4 keystroke save)
- Claude Code がファイルを書き換えると competing edit を検出してバナー表示 (Phase 10.5)、競合なしなら自動リロード
- 縦スクロール、テキスト選択・コピー可能

Phase 10 (2026-06-08〜) で read-only ソースビューから vim 編集に移行した。`.md` / `.markdown` / `.mdx` を対象とする (`MARKDOWN_EXTENSIONS`)。

## 2. レンダリング

| 項目 | 内容 |
|---|---|
| ハイライトエンジン | `syntect` 5 (default-fancy features) |
| シンタックス定義 | `syntect` 同梱の `markdown.sublime-syntax` |
| テーマ | OS のダーク/ライト連動。ダーク=`base16-ocean.dark`、ライト=`InspiredGitHub`（いずれも `syntect::ThemeSet::load_defaults` に含まれる）。判定は `ui.style().visuals.dark_mode` |
| フォント | egui の `FontId::monospace(13.0)`（CJK 部分は `ui::fonts::install_japanese` のフォールバックチェーン） |
| 行番号 gutter | `egui::Grid` で行ごとに gutter Galley + 縦セパレータ + 本文 Galley を 1 行として並べる |
| 行単位レイアウト | 行ごとに `LayoutJob` を構築して `f.layout_job` で `Arc<Galley>` を取得。`label_text_selection` (egui plugin API) で描画 + 選択処理 |
| カーソル / Visual | `Galley::pos_from_cursor(CCursor)` で char-precise に算出。Visual 範囲は背景 rect を `pos_from_cursor` ベースで union して描く |
| 検索ハイライト | `/`-prompt の match を bg 色付き `TextFormat` で本文 LayoutJob に焼き込む。アクティブマッチは強調色 |

実装は `src/preview/render.rs::show_source_grid`。Phase 10.29 で行ごとに `LayoutJob` 構築 + `layout_job` 呼び出しの結果を `EditorState.body_galley_cache` (HashMap<u64, Arc<Galley>>) にキャッシュし、未編集行は per-frame の再構築を完全にスキップする (詳細は §8)。

## 2.x vim エンジン (Phase 10.1〜)

`src/vim/mod.rs` に modal state machine がある。`apply(VimEvent) -> Action` を pure に保ち、副作用 (clipboard / chat 連携 / scroll) は `Action` 構造体に signal として乗せて host (App / preview render) が遂行する。

| Mode | 主な遷移 |
|---|---|
| `Normal` | デフォルト。motion + edit prefix + visual entry |
| `Insert` | 文字 / Backspace / Enter で buffer 編集。`Esc` で Normal |
| `Visual` (charwise) / `VisualLine` | 範囲選択 + `y`/`d`/`x`/`Y` |

代表的な `Action` フィールド:

| フィールド | 用途 |
|---|---|
| `buffer_changed` | 編集発生。host が `Tab::save_current_buffer()` で disk へ反映 |
| `cursor_moved` | preview render が次フレームで `scroll_to_me(Center)` |
| `copy_to_clipboard: Option<String>` | yank 系 op (`y` / `yy` / `dd` / `x` / Visual `y`/`d`/`x`) で OS clipboard に書く |
| `send_to_chat: Option<String>` | Visual `Y` — 選択テキストを host が引用ブロック化して chat input に append |
| `send_file_reference_to_chat: bool` | Normal `Y` (選択なし) — host が現在ファイル相対パスを `@<rel>\n` で chat input に挿入、カーソルを末尾に移動 (Phase 10.17 / 10.23) |

詳細キーバインドは `ui.md` §6.1 を参照。

## 3. リンク・画像

markdown ソースを **テキストとしてそのまま表示する**ため、リンクや画像は
クリックできず、画像のインライン表示もない。`[](...)` `![](...)` の文字列が
そのまま見える。

旧仕様（rendered preview）にあった以下は **すべて削除**:

- 相対 `.md` リンククリックでの preview 切替（旧 Phase 4.4）
- インライン画像表示（旧 Phase 4.5）
- 外部 URL クリックで OS ブラウザ起動
- 相対画像・PDF クリックで OS 既定アプリ起動

## 4. ファイルロード

| 項目 | 内容 |
|---|---|
| 実装 | `src/preview/loader.rs::load_markdown(path)` |
| エラー | `NotFound` / `PermissionDenied` / `NotUtf8` / `TooLarge { size_bytes }` / `Io(String)` |
| Hard limit | 10 MiB（`HARD_LIMIT_BYTES`）。超過時は body を読まずに `TooLarge` を返す |
| Soft limit | 1 MiB（`SOFT_LIMIT_BYTES`）。超過時は `SizeClass::Large` でロード、ペイン上部に警告バナーを表示 |

`SizeClass::Large` でも syntect ハイライトは普通に走る（旧仕様では fence
info-string を剥がしてプレーン化していたが、ソース表示ではその意味がない
ため削除）。

## 5. 自動リロード（ファイル監視）

| 項目 | 内容 |
|---|---|
| 監視ライブラリ | `notify` 8 |
| 単一ファイル監視 | `preview::watcher::FileWatcher`（`NonRecursive`）。現在開いているファイルを監視 |
| プロジェクト監視 | `preview::watcher::ProjectWatcher`（`Recursive`）。`.md` のみ通す（画像は対象外） |
| デバウンス | リロード 100ms、自動追従 200ms |
| エラー処理 | 監視開始失敗時はタブの `watcher_error` バナーに表示。リロードは `Cmd+R`/`Ctrl+R` で手動可能 |
| ファイル削除 | プレビューを「ファイルが見つかりません」状態に切替。再作成されたら次の Change イベントで自動的に表示再開 |

## 6. スクロール挙動

- 縦スクロールのみ。`ScrollArea::vertical()` で `auto_shrink([false, false])`
- 起動直後は最上端
- ファイル再読込時 / 別ファイル切替時はスクロール位置を常にリセット
- vim カーソル移動時は `EditorState.scroll_to_cursor` フラグ (`Action.cursor_moved` で立つ) を見て、render が `scroll_to_me(Center)` で active row を画面内に追従させる (Phase 10.16)。マウスでスクロールしただけのフレームでは flag が立たないので user のスクロール位置を勝手に戻さない
- ファイルツリーを開いているとき、wheel が tree 側に奪われないよう `egui::Panel::left` で tree を独立 scroll region に分離 (Phase 10.16)

## 7. プレビュー対象ファイルの指定方法

### 7.1 初期表示の決定（起動時）

1. `Cmd+O`/`Ctrl+O` でユーザーがファイル選択ダイアログから選んだファイル
   （起動後の操作）。この場合は自動追従が OFF になる
2. コマンドライン引数で渡したパス（`mdpilot path/to/file.md`）
3. コマンドライン引数がプロジェクトディレクトリの場合、ルート直下の
   `README.md`（大文字小文字を区別しない検索）
4. 上記いずれも無ければ空ペイン（プレースホルダ表示）

### 7.2 起動後の自動切替

自動追従モードが ON のとき、プロジェクトルート以下の `.md` ファイル書込みを
`notify` が検出すると、表示中ファイル以外への書込みでプレビュー対象を切替
（`claude-integration.md` §5 案 A、§6 詳細仕様）。stream-json `tool_use` 解釈
（案 B）や MCP（案 C）は MVP 後。

### 7.3 切替時の動作

- 現在の `FileWatcher` を停止し、新ファイルに対する Watcher を開始
- スクロール位置はリセット
- ウィンドウタイトルを更新（`ui.md` §4）

## 8. 大きなファイル

| 状況 | 動作 |
|---|---|
| 1 MiB 未満（`SizeClass::Small`） | 通常通り全行 syntect highlight |
| 1 MiB 以上 10 MiB 未満（`SizeClass::Large`） | ペイン上部に警告バナーを表示してから全行 syntect highlight |
| 10 MiB 以上 | `TooLarge` エラーバナー、本文ロードしない |

`ScrollArea::vertical()` は virtualize しない (画面外行も毎フレーム allocate される) ので、行数が多いと LayoutJob 構築・font shape・egui galley cache lookup の per-frame コストが嵩む (2500 行 md でキーストロークラグが体感できるレベル)。

Phase 10.29 で **行ごとの Galley キャッシュ** を `EditorState.body_galley_cache` に追加した。キーは `(行テキスト, theme, dark, この行のサーチマッチ範囲, wrap_width)` の 64-bit hash。ヒット時は `LayoutJob` 構築と `f.layout_job` を完全にスキップ。`syntect::HighlightLines` の per-line 状態 (fenced code block 等) を壊さないため `highlight_line` だけは毎行常時呼ぶ (保守的判断)。フレームごとに entries を rotate して buffer から消えた行は自動 eviction。

## 9. `/` 検索 (Phase 10.6)

| 操作 | 動作 |
|---|---|
| Normal で `/` | 検索プロンプトを開く |
| 文字入力 → `Enter` | クエリ確定。全マッチ位置を保持し、最初のマッチへジャンプ |
| `n` / `N` | 次 / 前のマッチへジャンプ |
| `Esc` | プロンプト or 検索結果をクリア |

マッチは行ごとに背景色付き `TextFormat` で `LayoutJob` に焼き込む。アクティブマッチは別色 (`active_match_bg`) で強調。

## 10. 既知の制限・未確定事項

| 項目 | 状態 |
|---|---|
| プレビュー内検索（`Cmd+F`） | vim の `/` で代替 (Phase 10.6 で実装) |
| 行 Galley キャッシュの保守的方針 | Phase 10.29 は `highlight_line` を常時呼ぶことで syntect 状態を保つ。syntect 自体の per-frame コストはまだ残っているので、必要なら viewport 限定描画 / syntect 状態 cache 化に進む |
| テーマ切替 UI | MVP 後 |
| 大きなファイルのしきい値 | 上記は暫定 |
