# sekien — Mermaid Drawer

sekien is a drawer of Mermaids — Mermaid コードを SVG に変換する CLI ツール。

- OS ネイティブの WebView (macOS: WKWebView) を使用するため、Chromium のバンドルが不要
- Pandoc の `--filter` オプションに対応

## mmdc との比較

|  | sekien | mmdc |
|---|---|---|
| バイナリサイズ | 4.5 MB | 330 MB (node_modules) |
| 依存 | OS ネイティブ WebView | Puppeteer (Chromium 同梱) |
| インストール | `cargo install` | `npm install -g` |
| 実行速度 | ~0.6s | ~1.4s |
| メモリ使用量 (RSS) | ~106 MB | ~252 MB |
| Pandoc filter | ✓ (built-in) | 別途 mermaid-filter が必要 |
| stdout 出力 | ✓ | ✗ (ファイル指定必須) |

実行速度・メモリ使用量ともに優位なのは Chromium をバンドルせず OS の WebView を使うため。
計測環境: macOS (Apple Silicon)、`bench/` の図を各 20 回平均。

## インストール

```bash
cargo install sekien-cli sekien-pandoc
```

スタンドアロン CLI のみ使う場合:

```bash
cargo install sekien-cli
```

## 使い方

### スタンドアロン CLI (`sekien`)

```bash
# .mmd ファイル → SVG (stdout)
sekien diagram.mmd > diagram.svg

# stdin → SVG (stdout)
cat diagram.mmd | sekien > diagram.svg
```

### Pandoc filter (`sekien-pandoc`)

```bash
pandoc input.md -o output.html --filter sekien-pandoc
```

#### 対応 PDF engine

sekien-pandoc は `RawBlock("html", svg)` を出力するため、HTML を経由しないエンジンでは SVG が落とされる。

| PDF engine | 動作 |
|---|---|
| `weasyprint` | ✓ (HTML 経由) |
| `pdflatex` / `xelatex` / `lualatex` | ✗ (raw HTML を drop) |
| `typst` | ✗ (raw HTML を drop) — Lua filter で回避可能 |

#### HTML を経由しない PDF engine (Lua filter を使う)

typst や pdflatex など raw HTML を drop する PDF engine では、sekien-pandoc に同梱の Lua filter で
SVG をファイルに書き出して Image ノードに変換することで回避できる。

```bash
# Lua filter をカレントディレクトリに書き出す
sekien-pandoc --print-lua-filter > sekien.lua

pandoc input.md -o output.pdf \
  --pdf-engine=typst \
  --filter sekien-pandoc \
  --lua-filter sekien.lua \
  -V mainfont="Hiragino Sans"
```

インストール不要で使うには process substitution が使える (bash/zsh):

```bash
pandoc input.md -o output.pdf \
  --pdf-engine=typst \
  --filter sekien-pandoc \
  --lua-filter <(sekien-pandoc --print-lua-filter) \
  -V mainfont="Hiragino Sans"
```

常用するなら pandoc の user data directory に置くとパスなしで参照できる:

```bash
sekien-pandoc --print-lua-filter > ~/.local/share/pandoc/filters/sekien.lua

pandoc input.md -o output.pdf \
  --pdf-engine=typst \
  --filter sekien-pandoc \
  --lua-filter sekien.lua \
  -V mainfont="Hiragino Sans"
```

## オプション

`--font` / `--theme` / `--look` は `sekien` (スタンドアロン) のフラグ。
`sekien-pandoc` はフラグを受け付けないため、環境変数で指定する。

### `--font` / `SEKIEN_FONT`

デフォルトは mermaid.js のデフォルト (`"trebuchet ms", verdana, arial, sans-serif`)。

```bash
# フラグで指定 (sekien)
sekien --font "Hiragino Sans" diagram.mmd > diagram.svg

# 環境変数で指定 (sekien / sekien-pandoc 共通)
export SEKIEN_FONT="Hiragino Sans, Noto Sans JP, sans-serif"
```

### `--theme` / `SEKIEN_THEME`

mermaid.js のテーマを指定できる。未指定時は mermaid.js のデフォルト (`default`) が使われる。

```bash
sekien --theme dark diagram.mmd > diagram.svg
SEKIEN_THEME=forest pandoc input.md -o output.html --filter sekien-pandoc
```

指定できる値: `default` / `base` / `dark` / `forest` / `neutral` / `neo` / `neo-dark` / `redux` / `redux-dark` / `null`

### `--look` / `SEKIEN_LOOK`

図の描画スタイルを指定できる。

```bash
sekien --look handDrawn diagram.mmd > diagram.svg
```

指定できる値: `classic` / `handDrawn` / `neo`

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

これは Wayland セッションで Xwayland を介すると、コンポジタが
ウィンドウを可視位置に出してしまい一瞬画面に flash するのを防ぐため。

事前に Xvfb をインストールしておく:

```bash
apt install xvfb       # Debian/Ubuntu
dnf install Xvfb       # Fedora
```

```bash
sekien diagram.mmd > diagram.svg   # デスクトップでも CI でも flash 無し
```

起動した Xvfb は sekien 終了時に自動的に停止する (`-terminate` 起動)。

## ビルド

```bash
cargo build --release
```

`sekien/assets/mermaid.min.js` (v11.14.0) はリポジトリに同梱済み。更新する場合は npm から取得して差し替える。

```bash
npm install mermaid
cp node_modules/mermaid/dist/mermaid.min.js sekien/assets/
```

## 構成

```
sekien/                          # workspace root
├── sekien/                      # lib クレート (コアロジック)
│   ├── assets/
│   │   └── mermaid.min.js       # コンパイル時にバイナリへ埋め込まれる
│   └── src/
│       ├── lib.rs               # WebView レンダラ
│       └── linux_display.rs     # Linux 専用: Xvfb 起動と DISPLAY 設定
├── sekien-cli/                  # sekien コマンド
│   └── src/
│       └── main.rs
└── sekien-pandoc/               # sekien-pandoc コマンド
    ├── assets/
    │   └── sekien.lua           # 同梱 Lua filter
    └── src/
        ├── main.rs
        └── pandoc.rs            # Pandoc AST 処理
```

### lib.rs (sekien)

wry が提供する OS ネイティブ WebView を起動し、mermaid.js を使って
Mermaid コードを SVG に変換する。

```
render_all(blocks) の流れ:
  (Linux のみ冒頭で linux_display::ensure_display() が Xvfb 起動 + DISPLAY 設定)

[Rust]                         [WebView / JS]
  |                                  |
  |-- WebView 起動、HTML ロード ----->|
  |                                  |-- mermaid.initialize()
  |                                  |-- ipc.postMessage({ type: 'ready' })
  |<-- IPC: ready ------------------|
  |-- evaluate_script: render(0) -->|
  |                                  |-- mermaid.render(code)
  |                                  |-- ipc.postMessage({ type: 'svg', svg })
  |<-- IPC: svg ---------------------|
  |-- evaluate_script: render(1) -->|
  |          ...                     |
  |-- process::exit() (全ブロック完了)
```

IPC メッセージは `EventLoopProxy<String>` を介してやり取りし、
`ControlFlow::Wait` でイベントドリブンに処理する。

### pandoc.rs (sekien-pandoc)

stdin の Pandoc AST JSON を受け取り、
`CodeBlock` ノードのうち class に `mermaid` を持つものを
`RawBlock("html", svg)` に差し替えて stdout に返す。

```json
// 変換前
{ "t": "CodeBlock", "c": [["", ["mermaid"], []], "graph LR ..."] }

// 変換後
{ "t": "RawBlock", "c": ["html", "<svg ...>...</svg>"] }
```

pandoc は filter を `sekien-pandoc <output-format>` として呼び出す。
format 引数は受け取るが使用しない。
