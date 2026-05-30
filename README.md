# sekien — Mermaid Drawer

sekien is a drawer of Mermaids — Mermaid コードを SVG に変換する CLI ツール。

Mermaid 公式の [`mmdc`](https://github.com/mermaid-js/mermaid-cli) に比べて、sekien は **最大 83 倍小さく**、**最大 8 倍軽く**、**最大 3 倍速い**（詳細は下表）。

## mmdc との比較

OS ネイティブの WebView を活用することで、標準的な `mmdc` (Puppeteer/Chromium ベース) に比べ圧倒的に軽量・高速に動作します。

いずれの環境でも実行速度・メモリ使用量ともに優位なのは、重量級の Chromium をバンドルせず、OS 標準の描画エンジンをダイレクトに叩くためです。

- `util/bench/` の 3 図の中央値。 mmdc は 11.14.0
- 計測環境: macOS (arm64)、sekien 0.1.0 (mermaid.js 11.14.0)
- 計測環境: Linux (x86_64)、sekien 0.1.0 (mermaid.js 11.14.0)、内部 Xvfb 使用
- Max RSS は Xvfb/WebKit/Chromium を含む全子プロセスの合計最大値 (`util/bench/bench.sh` 参照)。

### バイナリサイズ

| platform | sekien | mmdc | Advantage |
|---|---|---|---|
| Mac | **~10 MB** | 330 MB | 97% 小さい |
| Linux | **4.8 MB** | 401 MB | 99% 小さい |

### 実行速度

| platform | sekien | mmdc | Advantage |
|---|---|---|---|
| Mac | **~360 ms** | ~1.1 s | **67% 速い** |
| Linux | **~1.1 s** | ~1.6 s | **31% 速い** |

### メモリ使用量

| platform | sekien | mmdc | Advantage |
|---|---|---|---|
| Mac | **~90 MB** | ~690 MB | **87% 軽い** |
| Linux | **~430 MB** | ~630 MB | **32% 軽い** |

## インストール

```bash
cargo install sekien
```

Linux の場合はビルドに WebKitGTK 関連のパッケージが必要です（Ubuntu 例）:
`sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev`

## 使い方

```bash
# .mmd ファイル → SVG (stdout)
sekien diagram.mmd > diagram.svg

# stdin → SVG (stdout)
cat diagram.mmd | sekien > diagram.svg

# 複数 Mermaid を 1 回の sekien 起動で処理 (\0 区切り)
printf 'graph LR\n  A --> B\0graph TD\n  X --> Y' | sekien > out.bin
```

sekien は cat のような streaming プロセス。stdin を EOF まで読み続け、各 block
の SVG を即座に stdout に流す。block 単位の Mermaid 解析エラーは stderr に
`<!-- {"id": N} -->` 形式のメタデータ（オプション）とエラーメッセージを出力して継続する。

### 対話モード

terminal から直接起動して 1 block ずつ入力できる:

```text
$ sekien
graph LR
  A --> B
^@
<svg がその場で出る>
^D
$
```

`Ctrl + @` が NUL byte (`\0`) を入力する手段、`Ctrl + D` が EOF を投げて
sekien を終了させる手段。

## オプション

| フラグ | 説明 |
|---|---|
| `--font <name>` | フォント (CSS font-family 形式) |
| `--theme <name>` | mermaid.js テーマ |
| `--look <name>` | 描画スタイル |
| `--config <file>` | mermaid.initialize() 設定 JSON ファイル |
| `--block-id` | 各出力の前にメタデータ (`<!-- {"id": N} -->`) を付与 |
| `--version`, `-v` | バージョン表示 |
| `--help`, `-h` | ヘルプ表示 |

### 設定の永続化

よく使うオプションはシェルの alias に書いておくと毎回の入力を省ける:

```bash
# ~/.bashrc や ~/.zshrc に追記
alias sekien='sekien --theme dark --font "Noto Sans"'

# --config でまとめて管理する場合
alias sekien='sekien --config ~/.config/sekien.json'
```

## 動作環境

| OS | 要件 |
|---|---|
| macOS | ディスプレイ接続が必要 (WKWebView) |
| Windows | ディスプレイ接続が必要 (WebView2) |
| Linux | Xvfb (内部で自動起動。画面/セッション不問) |

### Linux: Xvfb が必須

sekien は Linux では実行のたびに内部で Xvfb を起動し、その仮想 display 上で
描画する。デスクトップ環境 (X11 / Wayland / Xwayland) や `$DISPLAY` の値は
一切参照しない。

## 構成

詳細なプロトコル仕様は [protocol.md](util/docs/protocol.md) 参照。

### render.rs

**wry** が提供する OS ネイティブ WebView を起動し、mermaid.js を使って
Mermaid コードを SVG に変換する。イベントループとウィンドウ管理は **tao**。

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or https://www.apache.org/licenses/LICENSE-2.0)
- MIT License ([LICENSE-MIT](LICENSE-MIT) or https://opensource.org/licenses/MIT)

at your option.

### Bundled Assets

- `mermaid.js`: Licensed under the [MIT License](assets/mermaid.LICENSE). Copyright (c) 2014 - 2024 Knut Sveidqvist and contributors.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall
be dual licensed as above, without any additional terms or conditions.
