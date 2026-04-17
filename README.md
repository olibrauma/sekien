# mmsvg

Mermaid コードを SVG に変換する CLI ツール。

- OS ネイティブの WebView (macOS: WKWebView) を使用するため、Chromium のバンドルが不要
- Pandoc の `--filter` オプションに対応

## 使い方

```bash
# .mmd ファイル → SVG (stdout)
mmsvg diagram.mmd > diagram.svg

# stdin → SVG (stdout)
cat diagram.mmd | mmsvg > diagram.svg

# Pandoc filter として使う (HTML 出力に SVG がインラインで埋め込まれる)
pandoc input.md -o output.html --filter mmsvg
```

### 注意: ファイル名に拡張子をつけること

mmsvg は引数がファイルパス (`.` を含む) かどうかで動作モードを切り替える。
`.mmd` などの拡張子なしでファイルを渡すと、誤って Pandoc filter モードと判定される。

```bash
mmsvg diagram      # NG: filter モードと判定される
mmsvg diagram.mmd  # OK
```

### 対応 PDF engine

| PDF engine | 動作 |
|---|---|
| `weasyprint` | ✓ (HTML 経由) |
| `wkhtmltopdf` | ✓ (HTML 経由) |
| `pdflatex` / `xelatex` | ✗ (raw HTML を drop) |
| `typst` | ✗ (raw HTML を drop) |

```bash
pandoc input.md -o output.pdf --filter mmsvg --pdf-engine=weasyprint
```

## ビルド

```bash
# assets/ に mermaid.js を用意する (初回のみ)
npm install mermaid
cp node_modules/mermaid/dist/mermaid.min.js assets/

cargo build --release
```

## 構成

```
mmsvg/
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

pandoc は filter を `mmsvg <output-format>` として呼び出す。
引数が `.` を含まない単語の場合は Pandoc filter モードと判定する。
