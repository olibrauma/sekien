# mmsvg

Mermaid コードブロックを SVG に変換する CLI ツール。

- OS ネイティブの WebView (macOS: WKWebView) を使用するため、Chromium のバンドルが不要
- Markdown in / Markdown out のフィルタとして動作
- Pandoc の `--filter` オプションにも対応

## 使い方

```bash
# Markdown ファイルを変換 (Mermaid ブロックが SVG に置換される)
mmsvg input.md > output.md

# stdin から読む
cat input.md | mmsvg > output.md

# Pandoc filter として使う
pandoc input.md -o output.pdf --filter mmsvg --pandoc-filter --pdf-engine=typst

# Pandoc とパイプで繋ぐ
cat input.md | mmsvg | pandoc -f markdown -o output.pdf --pdf-engine=typst
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
    ├── markdown.rs          # Markdown transformer
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
イベントループを `ControlFlow::Poll` で回してポーリングする。

### markdown.rs

入力 Markdown から ` ```mermaid ... ``` ` ブロックを検出し、
`renderer::render_all` で SVG に変換したあと元の位置に差し替える。
後ろから置換することで文字位置のずれを防ぐ。

### pandoc.rs

stdin の Pandoc AST JSON を受け取り、
`CodeBlock` ノードのうち class に `mermaid` を持つものを
`RawBlock("html", svg)` に差し替えて stdout に返す。

```json
// 変換前
{ "t": "CodeBlock", "c": [["", ["mermaid"], []], "erDiagram ..."] }

// 変換後
{ "t": "RawBlock", "c": ["html", "<svg ...>...</svg>"] }
```

## 今後の課題

- `ControlFlow::Poll` によるポーリングを廃止し、
  winit 0.30 の `run_app` + `ApplicationHandler` を使った
  イベント駆動型に移行する
- `cargo install` でインストールできるように `assets/` の扱いを整理する
