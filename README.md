# mdpilot

Claude Code と協調して Markdown を書くための、ネイティブ GUI アプリケーション。

## コンセプト

- **文章を書くのは Claude Code**。ユーザーは Claude Code に指示し、生成・編集された Markdown を即座にプレビューで確認する
- **左ペインに Markdown プレビュー、右ペインに内蔵ターミナル**。ターミナル内で `claude` を対話的に動かす
- **手動編集の機能は持たない**（プレビュー専用）。テキストの増減はすべて Claude Code 経由

## 主要機能

| | 機能 | 概要 |
|---|---|---|
| 1 | 内蔵ターミナル | アプリ内で PTY を起動し、`claude` を対話的に動かせるフル機能のターミナルエミュレータ |
| 2 | Markdown プレビュー | CommonMark + GFM 拡張（テーブル / チェックボックス / 取り消し線）+ コードブロックのシンタックスハイライト |
| 3 | Claude Code 連携 | Claude Code 側からプレビュー対象ファイルを開ける仕組み（実現方式は検討中） |
| 4 | 自動リロード | Claude Code がファイルを編集したら、プレビューが自動的に追従 |

## 技術スタック

| 層 | 採用 |
|---|---|
| 言語 | Rust |
| GUI フレームワーク | [egui](https://github.com/emilk/egui) (eframe) |
| ターミナル | [egui_term](https://github.com/Harzu/egui_term) + [alacritty_terminal](https://crates.io/crates/alacritty_terminal) + [portable-pty](https://crates.io/crates/portable-pty) |
| Markdown レンダラ | [egui_commonmark](https://crates.io/crates/egui_commonmark)(暫定) |
| Markdown パーサ | [pulldown-cmark](https://crates.io/crates/pulldown-cmark)(egui_commonmark 内部依存) |
| シンタックスハイライト | [syntect](https://crates.io/crates/syntect)(検討中) |
| ファイル監視 | [notify](https://crates.io/crates/notify)(検討中) |

## 対象プラットフォーム

- macOS（Apple Silicon / Intel）
- Windows 10 / 11

Linux は当面サポート対象外。

## ステータス

設計フェーズ。仕様書を `docs/` 以下にまとめている段階で、実装はまだ着手していない。

## ドキュメント

| ファイル | 内容 |
|---|---|
| [docs/requirements.md](docs/requirements.md) | 機能要件・非機能要件・スコープ |
| [docs/architecture.md](docs/architecture.md) | プロセス構成・モジュール分割・データフロー |
| [docs/ui.md](docs/ui.md) | ウィンドウ・ペイン・キーバインド |
| [docs/terminal.md](docs/terminal.md) | 内蔵ターミナルの仕様 |
| [docs/preview.md](docs/preview.md) | Markdown プレビューの仕様 |
| [docs/claude-integration.md](docs/claude-integration.md) | Claude Code との連携方式 |

## ライセンス

未定。
