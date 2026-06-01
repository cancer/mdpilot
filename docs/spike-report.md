# Phase 0.5 スパイク結果レポート

## 0.5.2 追加検証: 描画品質（2026-06-01）

`spike_egui_commonmark` を改造して `ViewportCommand::Screenshot` で内部スクショを 3 枚撮影し、エージェントが画像を Read で確認した結果：

| 要素 | preview.md 2 章の予測 | 実機（egui_commonmark 0.23 + better_syntax_highlighting） |
|------|--------------------|--------------------------------------------------------|
| 見出し ATX | ◯ | ✓ |
| 段落 | ◯ | ✓ |
| 強調 / 太字 | ◯ | ✓ |
| インラインコード | ◯ | ✓ 背景付き monospace |
| コードブロック（fenced + info string） | ◯ | ✓ syntect ハイライト（rust / python ともに色付き、プレーンはハイライト無し） |
| 引用ブロック | ◯ | ✓ 左サイドバー + 字下げ |
| リスト（順序付き・順序なし・ネスト） | ◯ | ✓ |
| インラインリンク | ◯ | ✓ 青色アンダーライン |
| 自動リンク（裸 URL） | ◯ | ✓ |
| 水平線 | ◯ | ✓ |
| 画像（相対パス・存在しない） | ◯ | ⚠️ **警告アイコンのみ表示、代替テキスト（alt）は出ない**。`preview.md` 6 章の「HTTP/HTTPS は代替テキストのみ」は HTTP の話で、ローカルパスが存在しない場合の挙動は別途仕様化が必要 |
| GFM テーブル | ◯（MVP 必須） | ✓ カラム揃え（左/右/中央）が機能、セル内のインラインも描画 |
| GFM タスクリスト | ◯（MVP 必須） | ✓ チェック済み / 未チェックを区別して描画。クリック反応は本検証では未確認 |
| GFM 取り消し線 | ◯（MVP 必須） | ✓ |
| 脚注 | できれば | ✓ 本文に上付き参照、末尾に本文（[note]: 形式） |
| 生 HTML（`<br>`） | 限定対応（`<br>` 程度を想定） | ⚠️ **`<br>` は解釈されず文字列としてそのまま表示される**。仕様書の「限定対応」記述は実機挙動と食い違うため修正が必要 |
| コードブロック右上のコピーアイコン | 仕様書記載なし | egui_commonmark が自動で付与（現状そのままでよさそう） |

スクショ取得は本リポジトリ内に残る `spike/egui_commonmark/src/app.rs` の自動キャプチャロジック（`/tmp/spike_md_*.png` に保存）で再現できる。

### `preview.md` 反映が必要な差分

1. 2.1 章「生 HTML」の「`<br>` 程度を想定」を「**未対応**（egui_commonmark は `<br>` を解釈せず生表示）」に修正、もしくは自前で前処理して `<br>` を改行に置換する仕様を追加
2. 6 章「画像」の節に「ローカルパスが存在しない場合は警告アイコンのみが描画される（代替テキストは出ない）」を追記

### 未検証 / 別途実機が必要

- macOS スクショは Retina 物理ピクセル（1800×1400）。ライト/ダークの自動切替や、テーマ別のコードブロック色味は未確認
- タスクリストのチェックボックスはクリック反応するか未確認（プレビューが読み取り専用である mdpilot のスコープでは無反応が望ましい）
- 大きなファイル（1MB 以上）の挙動は未確認

## 結論サマリ（2026-05-29 ピボット後）

- **0.5.1 egui_term**: **採用中止**。本書の調査と判断 A の議論を経て、ユーザーが「ターミナル以外で Claude Code とやり取りする手段」を選び、In-app chat UI 路線（A 案）に方針転換。`spike/egui_term/` は git 履歴に残し、参考リソースとする
- **0.5.2 egui_commonmark**: **継続採用**。preview ペインのレンダラとして使う
- **方針転換後の新仕様**: `docs/chat.md`（新規）、`docs/requirements.md` / `docs/architecture.md` / `docs/ui.md` / `docs/claude-integration.md`（改訂）、`docs/terminal.md`（削除）に反映済み
- **`docs/plan.md`**: Phase 2 / 3 を chat UI 実装に書き換え。Phase 0.5.1 は superseded マーク

以降は当時の検証ログとして保持する。

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

#### A-1〜A-3 の比較

| 案 | 内容 | メリット | デメリット |
|----|------|---------|-----------|
| A-1 | `git = "https://github.com/Harzu/egui_term", rev = "df910f2"` 固定 | egui 0.34 で即着手可、reproducible | crates.io 公式リリース外を本体依存に含む、定期的に rev を bump する運用が必要 |
| A-2 | 次の crates.io 正式リリース（0.2.0?）を待つ | 公式リリースのみで構成 | リリース時期不明、Phase 2 着手が遅延 |
| A-3 | 別のターミナルウィジェット候補を再調査 | 選択肢を広げる | 調査コスト増、候補があるかも不明 |

#### A-3 の調査結果（2026-05-29 時点）

crates.io と GitHub から「egui + terminal/PTY 」をキーに調査した結果：

| crate / repo | star | 直近 push | crates.io | コメント |
|---|---|---|---|---|
| **Harzu/egui_term** | 69 | 2026-05-26 | 0.1.0 (egui 0.31) / main は egui 0.34.2 | 本スパイクで採用、最活発、機能リスト最多 |
| Quinntyx/egui-terminal | 33 | 2024-12-30 | 0.1.0 | **1 年以上更新なし**、egui バージョン未確認 |
| Quinntyx/eguitty | 11 | 2025-01-04 | — | 同上、メンテされていない |
| par-term | — | アクティブ | 0.32.0 | **アプリ本体**であり widget としての使い方不明、Sixel/iTerm2/Kitty 画像対応はある |
| afar | — | 実験段階 | 0.0.0 | egui-elegance 依存、リモートシェル特化、汎用 widget ではない |
| msiShariful/rustty, vibeterm, conch, zaxiom 等 | 0-4 | 様々 | — | いずれも **アプリ実装**で widget としては再利用不可 |

**egui エコシステムで「実用可能なターミナル widget」は実質的に Harzu/egui_term のみ**。他は古いか、アプリ本体（widget ではない）か、機能が限定的。

#### A-3 から派生する補助案

| 案 | 内容 | コスト |
|----|------|------|
| A-3a | 自前実装（`alacritty_terminal` + `portable-pty` + egui のキー入力ハンドラを直接組む） | 大。本質的に egui_term がやっていることを再実装 |
| A-3b | Harzu/egui_term を fork して mdpilot 配下で管理 | 中。fork メンテ運用が必要 |

エージェント暫定案: **A-1（git rev 固定）**。Harzu/egui_term は実質的な唯一の生きた選択肢で、main は既に egui 0.34 対応している。fork する利点は今のところ無く、必要になったら後で fork に切替えれば良い。最終判断はユーザー。

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
