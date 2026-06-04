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

左ペインは現在開いている markdown ファイルのソースを表示する。

- ユーザーは閲覧専用（編集不可）
- syntect で markdown syntax highlight
- 左カラムに行番号（gutter）
- 縦スクロール、テキスト選択・コピーは可能
- Claude Code がファイルを書き換えると自動でリロードして再表示

## 2. レンダリング

| 項目 | 内容 |
|---|---|
| ハイライトエンジン | `syntect` 5 (default-fancy features) |
| シンタックス定義 | `syntect` 同梱の `markdown.sublime-syntax` |
| テーマ | OS のダーク/ライト連動。ダーク=`base16-ocean.dark`、ライト=`InspiredGitHub`（いずれも `syntect::ThemeSet::load_defaults` に含まれる）。判定は `ui.style().visuals.dark_mode` |
| フォント | egui の `FontId::monospace(13.0)`（CJK 部分は `ui::fonts::install_japanese` のフォールバックチェーン） |
| 行番号 gutter | 行番号を右寄せ + ` │ ` セパレータで本文の左にプレフィックスとして付与（同一 `LayoutJob` の中で構築） |
| 1 ファイル全体 | 1 つの `LayoutJob` として組み立てて `egui::Label::new(job).selectable(true)` で描画 |

実装は `src/preview/render.rs::build_layout_job`。

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
- ファイル再読込時 / 別ファイル切替時はスクロール位置を常にリセット（egui の
  `ScrollArea` がデフォルトで状態を持たない構成）
- スクロール位置保持・編集追従は仕様から除外（2026-06-04 ユーザー判断）

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

## 9. 既知の制限・未確定事項

| 項目 | 状態 |
|---|---|
| プレビュー内検索（`Cmd+F`） | 仕様削除（2026-06-05 ユーザー判断、Phase 9.11 omit） |
| syntect ハイライトの per-frame コスト | 巨大ファイル（数 MiB）で 1 フレーム描画コストが嵩む可能性。キャッシュは未実装 |
| テーマ切替 UI | MVP 後 |
| 大きなファイルのしきい値 | 上記は暫定 |
