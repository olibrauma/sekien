# sekien — Mermaid Drawer

sekien is a drawer of Mermaids — Mermaid コードを SVG に変換する CLI ツール。

Mermaid 公式の [`mmdc`](https://github.com/mermaid-js/mermaid-cli) に比べて、sekien は **約 70 倍小さく**、**約 8 倍軽く**、**約 3 倍速い**。

## mmdc との比較

OS ネイティブの WebView を活用することで、標準的な `mmdc` (Puppeteer/Chromium ベース) に比べ圧倒的に軽量・高速に動作します。

### macOS (ネイティブ描画)
計測環境: macOS (arm64)、sekien 0.1.0 (mermaid.js 11.14.0) vs mmdc 11.12.0、`util/bench/` の 3 図の中央値。

|  | sekien | mmdc |
|---|---|---|
| バイナリサイズ | ~10 MB (合計) | 330 MB (node_modules) |
| 依存 | OS ネイティブ WebView | Puppeteer (Chromium 同梱) |
| 実行速度 (中央値) | **~360 ms** | ~1.1 s |
| メモリ使用量 (RSS) | **~90 MB** | ~690 MB |

### Linux (内部 Xvfb 描画)
計測環境: Linux (x86_64)、内部 Xvfb 使用、`util/bench/` の 3 図の中央値。
Max RSS は Xvfb を含む全子プロセスの合計最大値。

|  | sekien | mmdc |
|---|---|---|
| 実行速度 (中央値) | **~800 ms** | ~1.1 s |
| メモリ使用量 (Max RSS) | **~440 MB** | ~630 MB |

いずれの環境でも実行速度・メモリ使用量ともに優位なのは、重量級の Chromium をバンドルせず、OS 標準の描画エンジンをダイレクトに叩くためです。
また、Pandoc 連携用の専用フィルタ ([`sekien-pandoc`](../2026-05-20-sekien-pandoc)) を同梱しており、別途 `mermaid-filter` 等を導入する必要もありません。

## インストール

```bash
cargo install sekien
```

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
1 行 (`Error: mermaid block N: <msg>`) 流して継続し、最終的に exit 0 で終わる。
sekien 自身の失敗 (display 初期化失敗、I/O エラー等) のみ exit 1。

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

> **macOS の注意**: sekien 起動直後の WebView 初期化で terminal の key window
> を **1 度だけ** 奪う制約あり (tao + wry の API レベルで回避不能)。一度
> `Cmd + Tab` で terminal にフォーカスを戻せば、以降の block 入力では
> 再奪取されない (WebView は同一プロセス内で再利用される)。Linux では
> Xvfb 上で完結するためこの制約はない。

## オプション

| フラグ | 環境変数 | 説明 |
|---|---|---|
| `--font <name>` | `SEKIEN_FONT` | フォント (CSS font-family 形式) |
| `--theme <name>` | `SEKIEN_THEME` | mermaid.js テーマ |
| `--look <name>` | `SEKIEN_LOOK` | 描画スタイル |

CLI フラグが優先、未指定時は環境変数。

### `--theme` の値

`default` / `base` / `dark` / `forest` / `neutral` / `neo` / `neo-dark` / `redux` / `redux-dark` / `null`

### `--look` の値

`classic` / `handDrawn` / `neo`

## 動作環境

| OS | 要件 |
|---|---|
| macOS | ディスプレイ接続が必要 (WKWebView) |
| Windows | ディスプレイ接続が必要 (WebView2) |
| Linux | Xvfb (内部で自動起動。画面/セッション不問) |

### Linux: Xvfb が必須

sekien は Linux では実行のたびに内部で Xvfb を起動し、その仮想 display 上で
描画する。デスクトップ環境 (X11 / Wayland / Xwayland) や `$DISPLAY` の値は
一切参照しない。Wayland セッションで Xwayland を介すると、コンポジタが
ウィンドウを可視位置に出して一瞬画面に flash する問題を防ぐため。

```bash
apt install xvfb       # Debian/Ubuntu
dnf install Xvfb       # Fedora
```

起動した Xvfb は sekien 終了時に自動的に停止する (`-terminate` 起動)。

## ビルド

```bash
cargo build --release
```

`assets/mermaid.min.js` (v11.14.0) はリポジトリに同梱済み。更新する場合は npm から取得して差し替える。

```bash
npm install mermaid
cp node_modules/mermaid/dist/mermaid.min.js assets/
```

## 関連リポジトリ

- [sekien-api](api/rust/): sekien を Rust から呼ぶ wrapper (lib)
- [sekien-pandoc](../2026-05-20-sekien-pandoc): Pandoc filter (binary)

詳細な protocol 仕様は [DESIGN.md](DESIGN.md) 参照。

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or https://www.apache.org/licenses/LICENSE-2.0)
- MIT License ([LICENSE-MIT](LICENSE-MIT) or https://opensource.org/licenses/MIT)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall
be dual licensed as above, without any additional terms or conditions.
