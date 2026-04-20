# sekien — Mermaid Drawer

sekien is a drawer of Mermaids — Mermaid コードを SVG に変換する CLI ツール。

- OS ネイティブの WebView (macOS: WKWebView) を使用するため、Chromium のバンドルが不要
- Pandoc の `--filter` オプションに対応

## mmdc との比較

|  | sekien | mmdc |
|---|---|---|
| 実行速度 | ~0.6s | ~1.4s |
| メモリ使用量 (RSS) | ~106 MB | ~252 MB |
| バイナリサイズ | 4.5 MB | 330 MB (node_modules) |
| 依存 | OS ネイティブ WebView | Puppeteer (Chromium 同梱) |
| インストール | `cargo install` | `npm install -g` |
| Pandoc filter | ✓ | ✗ |

実行速度・メモリ使用量ともに優位なのは Chromium をバンドルせず OS の WebView を使うため。
計測環境: macOS (Apple Silicon)、`bench/` の図を各 20 回平均。

## インストール

```bash
cargo install sekien
```

## 使い方

### スタンドアロンモード

```bash
# .mmd ファイル → SVG (stdout)
sekien diagram.mmd > diagram.svg

# stdin → SVG (stdout)
cat diagram.mmd | sekien > diagram.svg
```

#### 注意: ファイル名に拡張子をつけること

sekien は引数がファイルパス (`.` を含む) かどうかで動作モードを切り替える。
`.mmd` などの拡張子なしでファイルを渡すと、誤って Pandoc filter モードと判定される。

```bash
sekien diagram      # NG: filter モードと判定される
sekien diagram.mmd  # OK
```

### Pandoc filter モード

```bash
pandoc input.md -o output.html --filter sekien
```

#### 対応 PDF engine

sekien は `RawBlock("html", svg)` を出力するため、HTML を経由しないエンジンでは SVG が落とされる。

| PDF engine | 動作 |
|---|---|
| `weasyprint` | ✓ (HTML 経由) |
| `pdflatex` / `xelatex` / `lualatex` | ✗ (raw HTML を drop) |
| `typst` | ✗ (raw HTML を drop) — Lua filter で回避可能 |

#### HTML を経由しない PDF engine (Lua filter を使う)

typst や pdflatex など raw HTML を drop する PDF engine では、sekien に同梱の Lua filter で
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

インストール不要で使うには process substitution が使える (bash/zsh):

```bash
pandoc input.md -o output.pdf \
  --pdf-engine=typst \
  --filter sekien \
  --lua-filter <(sekien --print-lua-filter) \
  -V mainfont="Hiragino Sans"
```

常用するなら pandoc の user data directory に置くとパスなしで参照できる:

```bash
sekien --print-lua-filter > ~/.local/share/pandoc/filters/sekien.lua

pandoc input.md -o output.pdf \
  --pdf-engine=typst \
  --filter sekien \
  --lua-filter sekien.lua \
  -V mainfont="Hiragino Sans"
```

## オプション

### `--font`

デフォルトは mermaid.js のデフォルト (`"trebuchet ms", verdana, arial, sans-serif`) で、
未指定時はシステムのフォントフォールバックが効く。
明示的に指定したい場合は `--font` フラグまたは環境変数を使う。

```bash
# フラグで指定 (スタンドアロンモード)
sekien --font "Hiragino Sans" diagram.mmd > diagram.svg

# 環境変数で指定 (pandoc filter モードでも有効)
export SEKIEN_FONT="Hiragino Sans, Noto Sans JP, sans-serif"
```

pandoc filter モードでは pandoc がフラグを渡せないため、環境変数を使う。

### `--theme`

`SEKIEN_THEME` 環境変数で mermaid.js のテーマを指定できる。
未指定時は mermaid.js のデフォルト (`default`) が使われる。

```bash
SEKIEN_THEME=dark sekien diagram.mmd > diagram.svg
SEKIEN_THEME=forest pandoc input.md -o output.html --filter sekien
```

指定できる値: `default` / `dark` / `forest` / `base` / `neutral`

## 注意: ディスプレイ接続が必要

sekien は OS ネイティブの WebView を使うため、いずれの OS でもディスプレイ接続が必要。
ディスプレイなし環境で実行するには下記「ヘッドレス実行」を参照。

## ヘッドレス実行 (Linux)

### X11

未検証。`xvfb-run` で仮想ディスプレイを立ち上げると動作する可能性がある。

```bash
xvfb-run sekien diagram.mmd > diagram.svg
```

GitHub Actions では:

```yaml
- run: xvfb-run cargo test
```

### Wayland

未検証。動作しない場合は `GDK_BACKEND=x11` で X11 バックエンドを強制すると
XWayland 経由で動作する可能性がある。

```bash
GDK_BACKEND=x11 sekien diagram.mmd > diagram.svg
```

Ubuntu 22.04 以降や Fedora など主要なデスクトップ環境では XWayland がデフォルトで利用可能。

## ビルド

```bash
cargo build --release
```

`assets/mermaid.min.js` はリポジトリに同梱済み。更新する場合は npm から取得して差し替える。

```bash
npm install mermaid
cp node_modules/mermaid/dist/mermaid.min.js assets/
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

IPC メッセージは `EventLoopProxy<String>` を介してやり取りし、
`ControlFlow::Wait` でイベントドリブンに処理する。

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
