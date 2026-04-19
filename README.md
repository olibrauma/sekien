# sekien — Mermaid Drawer

sekien is a drawer of Mermaids — Mermaid コードを SVG に変換する CLI ツール。

- OS ネイティブの WebView (macOS: WKWebView) を使用するため、Chromium のバンドルが不要
- Pandoc の `--filter` オプションに対応

## 使い方

```bash
# .mmd ファイル → SVG (stdout)
sekien diagram.mmd > diagram.svg

# stdin → SVG (stdout)
cat diagram.mmd | sekien > diagram.svg

# Pandoc filter として使う (HTML 出力に SVG がインラインで埋め込まれる)
pandoc input.md -o output.html --filter sekien
```

### 注意: ファイル名に拡張子をつけること

sekien は引数がファイルパス (`.` を含む) かどうかで動作モードを切り替える。
`.mmd` などの拡張子なしでファイルを渡すと、誤って Pandoc filter モードと判定される。

```bash
sekien diagram      # NG: filter モードと判定される
sekien diagram.mmd  # OK
```

### 対応 PDF engine

sekien は `RawBlock("html", svg)` を出力するため、HTML を経由しないエンジンでは SVG が落とされる。

| PDF engine | 動作 |
|---|---|
| `weasyprint` | ✓ (HTML 経由) |
| `pdflatex` / `xelatex` / `lualatex` | ✗ (raw HTML を drop) |
| `typst` | ✗ (raw HTML を drop) — Lua filter で回避可能 |

```bash
pandoc input.md -o output.pdf --filter sekien --pdf-engine=weasyprint
```

#### typst で PDF 化する (Lua filter を使う)

typst は `RawBlock("html")` を drop するが、sekien に同梱の Lua filter で
SVG をファイルに書き出して Image ノードに変換することで回避できる。

```bash
# Lua filter をカレントディレクトリに書き出す
sekien --print-lua-filter > sekien.lua

pandoc input.md -o output.pdf \
  --pdf-engine=typst \
  --filter sekien \
  --lua-filter sekien.lua \
  -V mainfont="Hiragino Sans"
```

pandoc の user data directory に置くとパスなしで参照できる:

```bash
sekien --print-lua-filter > ~/.local/share/pandoc/filters/sekien.lua

pandoc input.md -o output.pdf \
  --pdf-engine=typst \
  --filter sekien \
  --lua-filter sekien.lua \
  -V mainfont="Hiragino Sans"
```

### フォントの指定

ダイアグラム内のテキストフォントは `--font-family` フラグまたは環境変数で指定できる。
デフォルトは `"Noto Sans JP, sans-serif"`。

```bash
# フラグで指定 (スタンドアロンモード)
sekien --font-family "Hiragino Sans" diagram.mmd > diagram.svg

# 環境変数で指定 (pandoc filter モードでも有効)
export SEKIEN_FONT_FAMILY="Hiragino Sans, Noto Sans JP, sans-serif"
```

pandoc filter モードでは pandoc がフラグを渡せないため、環境変数を使う。

## ビルド

```bash
# assets/ に mermaid.js を用意する (初回のみ)
npm install mermaid
cp node_modules/mermaid/dist/mermaid.min.js assets/

cargo build --release
```

## 構成

```
sekien/
├── Cargo.toml
├── assets/
│   └── mermaid.min.js       # コンパイル時にバイナリへ埋め込まれる
└── src/
    ├── main.rs              # エントリポイント・CLI 引数処理
    ├── renderer.rs          # WebView レンダラ (コア)
    └── pandoc.rs            # Pandoc filter
```

### renderer.rs

wry が提供する OS ネイティブ WebView を起動し、mermaid.js を使って
Mermaid コードを SVG に変換する。

```
render_all(blocks) の流れ:

[Rust]                         [WKWebView / JS]
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
  |-- evl.exit() (全ブロック完了)
```

IPC メッセージは `Arc<Mutex<Option<String>>>` を介してやり取りし、
`ControlFlow::Poll` でポーリングする。

### pandoc.rs

stdin の Pandoc AST JSON を受け取り、
`CodeBlock` ノードのうち class に `mermaid` を持つものを
`RawBlock("html", svg)` に差し替えて stdout に返す。

```json
// 変換前
{ "t": "CodeBlock", "c": [["", ["mermaid"], []], "graph LR ..."] }

// 変換後
{ "t": "RawBlock", "c": ["html", "<svg ...>...</svg>"] }
```

pandoc は filter を `sekien <output-format>` として呼び出す。
引数が `.` を含まない単語の場合は Pandoc filter モードと判定する。
