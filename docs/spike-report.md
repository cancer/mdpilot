# Phase 0.5 スパイク結果レポート

## 検証範囲と前提

- 対象クレート: `egui_term` / `egui_commonmark` / `syntect`（後者は egui_commonmark の feature 経由）
- 検証 OS: macOS（aarch64-apple-darwin）のみ
- Windows 側は別途実施（ユーザー判断、`docs/plan.md` Phase 0.5 着手前の決定）
- 検証コード: `spike/egui_term/`、`spike/egui_commonmark/`
- 検証日: 2026-05-29

各スパイクで実施した自動検証は「`cargo build` 成功」「`cargo run` で起動して 6 秒間プロセスが生存する（即時クラッシュなし）」の 2 点のみ。**描画品質・対話的動作・IME などは実機目視が必要で、本レポートでは未検証**として明記する。

## 0.5.1 egui_term スパイク

### バージョン状況（重要）

| 経路 | バージョン | egui 対応 | mdpilot 本体 (egui 0.34) との互換 |
|------|-----------|----------|----------------------------------|
| crates.io | 0.1.0 | `^0.31.0` | ✗ 非互換 |
| GitHub main 最新（コミット `df910f2`、2026-05-06） | 未リリース | `0.34.2` に bump 済み | ✓ 整合 |

**結論**: crates.io 0.1.0 をそのまま使うと egui 0.31 縛りになり、mdpilot 本体（eframe 0.34）と非互換。`df910f2` を git rev 固定で取り込めば egui 0.34 ベースで動く。スパイクはこの方針で取り込み、ビルドが通ることを確認した。

### 自動検証結果

| 項目 | 結果 |
|------|------|
| `cargo build` | ✓ 成功 |
| 起動 6 秒生存（即時クラッシュなし） | ✓ |
| egui_term の機能リスト（README 由来） | PTY 描画、複数インスタンス、キー入力、カスタムバインド、リサイズ、スクロール、フォーカス、選択、フォント / カラースキーム、ハイパーリンク（hover/open）。README は macOS / Linux / Windows でテスト済みと明記 |

### 実機目視が必要な未検証項目

`docs/terminal.md` で MVP 必須とされた以下は本スパイクでは未検証：

- 日本語 IME 入力（プリエディット表示・確定。`requirements.md` N-07）
- `claude` 子プロセスの動作（PTY 経由でシェル → `claude` を起動）
- 選択 → `Cmd+C` → OS クリップボード、`Cmd+V` → PTY 書込み
- `Ctrl+C` → SIGINT 送信
- マウスレポート（`terminal.md` 5 章「MVP では対応」）
- ANSI エスケープ各種（カーソル制御 / SGR / 24-bit color / 代替スクリーンバッファ / bracketed paste）
- スクロールバック 10,000 行（`terminal.md` 7 章）
- ウィンドウタイトル変更 OSC を **反映しない** 挙動（`terminal.md` 5 章末尾）

これらは Phase 2 / 3 の実装と並行して実機で検証する。

### Windows 側で別途必要な検証

- Windows 上でのビルド（`x86_64-pc-windows-msvc`）
- `pwsh.exe` / `powershell.exe` / `cmd.exe` での起動可否
- Windows IME での日本語入力

## 0.5.2 egui_commonmark スパイク

### バージョン状況

| 項目 | 値 |
|------|-----|
| crates.io 最新 | 0.23.0 |
| egui 要求 | `^0.34.0` |
| mdpilot 本体との互換 | ✓ 整合 |
| 採用 features | `better_syntax_highlighting`（syntect 5.x 統合）, `svg`, `embedded_image`, デフォルトの `load-images` / `pulldown_cmark` |

### 自動検証結果

| 項目 | 結果 |
|------|------|
| `cargo build` | ✓ 成功 |
| 起動 6 秒生存 | ✓ |
| サンプル `sample.md` に含めた要素 | 見出し、強調、太字、インラインコード、コードブロック（Rust / Python / プレーン）、引用、リスト（順序付き / 順序なし / ネスト）、リンク（インライン）、自動リンク、水平線、GFM テーブル、GFM タスクリスト、GFM 取り消し線、脚注、生 HTML（`<br>`）、画像（相対パス） |

### 実機目視で `docs/preview.md` 2 章対応表を埋める

`preview.md` 2.1 / 2.2 の対応表は「○ / 限定対応 / 非対応」の予測値で書かれており、**実機での挙動確認は未実施**。Phase 1 着手前に `spike_egui_commonmark` を実機で立ち上げ、以下の観点を確認した上で `preview.md` を更新する：

| 要素 | 確認内容 |
|------|---------|
| GFM テーブル | カラム揃え（左・右・中央）、セル内のインライン要素 |
| タスクリスト | チェックボックスの描画、クリック反応の有無 |
| 取り消し線 | `~~` の正しい解釈 |
| 自動リンク | 裸 URL のクリック可否 |
| 脚注 | レンダリング、参照ジャンプ |
| 生 HTML | `<br>` の改行扱い、`<script>` 等の無視 |
| 画像（相対パス） | 存在しない場合の代替テキスト挙動 |
| コードブロック | syntect ハイライトのテーマ、フェンス info string の言語判定 |

### syntect について

- `better_syntax_highlighting` feature を有効化すると egui_commonmark が内部で `syntect` を引き込む（今回のビルドで `syntect v5.3.0` がコンパイルされたことを確認）
- ダーク / ライトテーマの具体的なテーマ名は `docs/plan.md` 6 章「着手前に確定すべき項目」のとおり Phase 4 着手前にユーザー判断
- `preview.md` 4 章は `base16-ocean.dark` / `InspiredGitHub` を例示値として置いている

### 未検証項目（実機目視必要）

- 各 GFM 要素の描画品質
- スクロール位置の保持（`preview.md` 8 章）
- リンクのクリック挙動（OS 既定ブラウザ起動、相対 `.md` の対象切替）
- 画像の自動リロード（`preview.md` 6 章末尾）
- 大きなファイル（1MB / 10MB しきい値）の挙動

## 採用判断（ユーザー確認待ち）

以下 3 項目について、ユーザー判断を仰ぐ。

### 判断 A: egui_term の取り込み方法

| 案 | 内容 | メリット | デメリット |
|----|------|---------|-----------|
| A-1 | `git = "https://github.com/Harzu/egui_term", rev = "df910f2"` 固定 | egui 0.34 で即着手可、reproducible | crates.io 公式リリース外を本体依存に含む、定期的に rev を bump する運用が必要 |
| A-2 | 次の crates.io 正式リリース（0.2.0?）を待つ | 公式リリースのみで構成 | リリース時期不明、Phase 2 着手が遅延 |
| A-3 | 別のターミナルウィジェット候補を再調査 | 選択肢を広げる | 調査コスト増、候補があるかも不明 |

エージェント暫定案: **A-1**（Phase 2 にすぐ着手するため）。最終判断はユーザー。

### 判断 B: egui_commonmark の継続採用

| 案 | 内容 |
|----|------|
| B-1 | 継続採用（GFM の実機挙動を Phase 1 着手前に目視確認、不足あれば自前描画で補強） |
| B-2 | `comrak` + 自前 egui 描画に切替（pulldown-cmark の GFM 対応度合いに不安が残る場合） |

エージェント暫定案: **B-1**（spike でビルド・起動が通り、egui 0.34 と整合しているため）。最終判断は実機目視結果を踏まえてユーザー。

### 判断 C: preview.md 2 章対応表の埋め込みタイミング

- C-1: 本スパイクのバイナリ（`spike/egui_commonmark/target/debug/spike_egui_commonmark`）をユーザーが手元で起動し、結果を本書または `preview.md` に書き込む
- C-2: Phase 4.2（egui_commonmark 描画タスク）の着手時に確認

エージェント暫定案: **C-1**（着手後の手戻りを避けるため）。

## まとめ

- 両クレートとも macOS でビルド・起動 OK
- `egui_term` のみ git rev 依存にする必要がある（egui 0.34 対応リリースが未公開のため）
- Windows 側および実機目視は未実施。判断 A / B / C についてユーザーに確認を取った後 Phase 1 へ進む
