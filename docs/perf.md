# mdpilot 非機能要件（NFR）測定

`docs/requirements.md` N-01〜N-04 の計測手順と結果。

数値はすべて **リリースビルド + SSD ストレージ + macOS aarch64** を前提とする（`requirements.md` N-01）。Windows と Linux はビルド整備後（Phase 8.x）に追記する。

## サマリ

| ID | 要件 | 目標 | 計測値 | 判定 |
|---|---|---|---|---|
| N-01 | 起動から操作可能まで | 3 秒以内 | TBD | TBD |
| N-02 | chat 入力 → claude 応答開始の UI 知覚遅延 | UI 上で知覚範囲（ネットワーク + Claude API 依存） | TBD | TBD |
| N-03 | claude ストリーミング描画 | フレーム落ちなく追従 | TBD | TBD |
| N-04 | Markdown 1 万字の再レンダリング | 100 ms 以内 | TBD | TBD |

リリースビルド完成（Phase 8.1/8.2）後、ユーザーが実機で測定して TBD を埋める。

## 計測環境

- ハードウェア: TBD（測定者が記入。例: MacBook Pro M3, 16GB RAM, NVMe SSD）
- OS: TBD（例: macOS 26.3）
- ビルド: `cargo build --release`
- 起動コマンド: `target/release/mdpilot <project-dir>`

## N-01: 起動時間

### 手順

`App::new` の冒頭で `Instant::now()` を捕捉し、最初の `ui()` 呼び出しで経過を tracing する instrumentation が `src/app.rs` に入っている (`target: "mdpilot::perf"`)。

```sh
cargo build --release
target/release/mdpilot <project-dir> 2>&1 | grep "mdpilot::perf"
```

ログ例:

```
INFO mdpilot::perf: first frame rendered (N-01) elapsed_ms=420
```

3 回計測して中央値を採用する。Cold start（OS 再起動直後）と warm start で値が大きく違う場合は両方記録する。

### 結果

| 測定日 | ハードウェア | 状態 | 経過 (ms) | 備考 |
|---|---|---|---|---|
| TBD | TBD | cold | TBD | |
| TBD | TBD | warm | TBD | |

## N-02: chat → claude 応答開始の遅延

ネットワークと Claude API に支配されるため mdpilot 内部の指標は限定的。UI 上で「Enter を押してから assistant のカーソルが現れるまで」を目視で測る。

### 手順

1. mdpilot 起動、適当な短いプロンプトを準備（例: 「say hi」）
2. Enter を押した瞬間と、右ペインの assistant 行に最初の文字が出る瞬間を計測
3. ストップウォッチで 3 回計測、中央値を記録

claude API の応答遅延が支配的なので mdpilot 側の最適化余地は少ない。UI が固まる症状があれば mdpilot の問題、何も表示されないまま数秒待つ場合は API 側。

### 結果

| 測定日 | プロンプト長 | 中央値 (ms) | 備考 |
|---|---|---|---|
| TBD | TBD | TBD | |

## N-03: ストリーミング描画

claude のストリーミング text_delta が連続到着している間、UI が 30fps 以上で更新できているかを目視確認する。

### 手順

1. 長めの応答を要求するプロンプトを送信（例: 「mdpilot のアーキテクチャを 500 字で説明して」）
2. ストリーミング中の egui の repaint 頻度を観察
3. egui 内蔵の `frame` インスペクタ（`Cmd+Shift+F12` 等）でフレームレートを見る、もしくは debug ビルドで `RUST_LOG=egui=debug` を有効にしてフレームログを観察

カクつき・フリーズが知覚できなければ pass。

### 結果

| 測定日 | 応答長 | 観察 (fps 体感) | 備考 |
|---|---|---|---|
| TBD | TBD | TBD | |

## N-04: 大きな Markdown の再レンダリング

`SizeClass::Small` (< 1 MiB) のうち 1 万字程度のファイルを reload して描画完了まで 100 ms 以内か確認する。

### 手順

1. 1 万字程度の `.md` ファイルを準備（手元の docs/* を `cat docs/architecture.md docs/chat.md > /tmp/big.md` で連結など）
2. `target/release/mdpilot /tmp/big.md` で起動 → N-01 ログで初回描画時間を確認
3. ファイルを `touch /tmp/big.md` で更新して watcher 経由の再ロードをトリガ、もしくは `Cmd+R` で強制再読み込み
4. 体感のラグを観察。debug 計測したい場合は `src/preview/render::show` の前後で `Instant::now()` を挟む instrumentation を一時的に追加

### 結果

| 測定日 | 文字数 | 初回描画 (ms) | 再描画 (ms) | 備考 |
|---|---|---|---|---|
| TBD | TBD | TBD | TBD | |

## 履歴

- 2026-06-03: Phase 7.9 で本書とコード instrumentation を追加。実値は Phase 8.1/8.2 のリリースビルド完成後に追記
