# mdpilot

Claude Code と協調して Markdown を書くための、ネイティブ GUI アプリケーション。

## コンセプト

- **文章を書くのは Claude Code**。ユーザーは右ペインの chat UI に指示を出し、生成・編集された Markdown を左ペインのプレビューで確認する
- **左ペインに Markdown プレビュー、右ペインに内蔵 chat UI**。mdpilot が背後で `claude` CLI を spawn し、JSON ストリームで通信する
- **手動編集の機能は持たない**（プレビュー専用）。テキストの増減はすべて Claude Code 経由

## 主要機能

| | 機能 | 概要 |
|---|---|---|
| 1 | 内蔵 chat UI | `claude --print --input-format=stream-json --output-format=stream-json` を子プロセスとして spawn し、JSON ストリームで双方向にメッセージをやり取り |
| 2 | Markdown プレビュー | CommonMark + GFM 拡張（テーブル / チェックボックス / 取り消し線）+ コードブロックのシンタックスハイライト |
| 3 | セッション継続 | プロジェクトルートごとに session-id を保存、次回起動時に `--continue` で再開 |
| 4 | 自動リロード | Claude Code がファイルを編集したら、プレビューが自動的に追従。プロジェクト配下の他の `.md` 編集も自動で対象切替 |

## 技術スタック

| 層 | 採用 |
|---|---|
| 言語 | Rust |
| GUI フレームワーク | [egui](https://github.com/emilk/egui) (eframe) |
| Markdown レンダラ | [egui_commonmark](https://crates.io/crates/egui_commonmark)(暫定) |
| Markdown パーサ | [pulldown-cmark](https://crates.io/crates/pulldown-cmark)(egui_commonmark 内部依存) |
| シンタックスハイライト | [syntect](https://crates.io/crates/syntect)（`egui_commonmark` の `better_syntax_highlighting` feature 経由） |
| ファイル監視 | [notify](https://crates.io/crates/notify) |
| stream-json パース | [serde_json](https://crates.io/crates/serde_json) |
| Claude 連携 | `claude` CLI（非対話 `--print` モード） |

## 対象プラットフォーム

- macOS（Apple Silicon / Intel）
- Windows 10 / 11

Linux は当面サポート対象外。

## ステータス

設計フェーズ。Phase 0（スケルトン）/ Phase 0.5（依存クレートのスパイク）まで完了し、chat UI 路線への方針転換を経て仕様書を改訂中。

## ドキュメント

| ファイル | 内容 |
|---|---|
| [docs/requirements.md](docs/requirements.md) | 機能要件・非機能要件・スコープ |
| [docs/architecture.md](docs/architecture.md) | プロセス構成・モジュール分割・データフロー |
| [docs/ui.md](docs/ui.md) | ウィンドウ・ペイン・キーバインド |
| [docs/chat.md](docs/chat.md) | 内蔵 chat UI の仕様（claude 子プロセスとの stream-json プロトコル） |
| [docs/preview.md](docs/preview.md) | Markdown プレビューの仕様 |
| [docs/claude-integration.md](docs/claude-integration.md) | Claude Code との連携方式 |
| [docs/plan.md](docs/plan.md) | 段階的実装計画 |
| [docs/spike-report.md](docs/spike-report.md) | Phase 0.5 のスパイク結果 |

## ライセンス

未定。
