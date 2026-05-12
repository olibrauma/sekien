# sekien — Mermaid Drawer

sekien is a drawer of Mermaids — Mermaid コードを SVG に変換する CLI ツール。

- OS ネイティブの WebView (macOS: WKWebView, Linux: WebKit2GTK) を使用するため、Chromium のバンドルが不要
- Pandoc の `--filter` オプションに対応
- ライブラリ、CLI、Pandoc フィルタの 3 つのコンポーネントで構成

## mmdc との比較

|  | sekien | mmdc |
|---|---|---|
| バイナリサイズ | ~4.5 MB | 330 MB (node_modules) |
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

Linux の場合はビルドに WebKit2GTK 関連のパッケージが必要です（Fedora 例）:
`sudo dnf install webkit2gtk4.1-devel gtk3-devel`

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

sekien-pandoc は `RawBlock("html", svg)` を出力するため、HTML を経由しないエンジンでは SVG が落とされる場合があります。

| PDF engine | 動作 |
|---|---|
| `weasyprint` | ✓ (HTML 経由) |
| `pdflatex`, `typst` 等 | ✗ (raw HTML を drop) — Lua filter で回避可能 |

#### HTML を経由しない PDF engine (Lua filter を使う)

typst や pdflatex など raw HTML を drop する PDF engine では、`sekien-pandoc` に同梱の Lua filter で SVG をファイルに書き出して Image ノードに変換することで回避できます。

**注意 (Typst ユーザー):**
Typst はデフォルトでプロジェクト外のファイルアクセスを制限しているため、Lua filter が作成する一時ファイル (`/tmp`) を読み込めるように `--pdf-engine-opt=--root=/` を付ける必要があります。

```bash
# Lua filter をカレントディレクトリに書き出す
sekien-pandoc --print-lua-filter > sekien.lua

pandoc input.md -o output.pdf \
  --pdf-engine=typst \
  --pdf-engine-opt=--root=/ \
  --filter sekien-pandoc \
  --lua-filter sekien.lua \
  -V mainfont="Hiragino Sans"
```

インストール不要で使うには process substitution が使える (bash/zsh):

```bash
pandoc input.md -o output.pdf \
  --pdf-engine=typst \
  --pdf-engine-opt=--root=/ \
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

`--font` / `--theme` / `--look` は `sekien` (スタンドアロン) のフラグです。
`sekien-pandoc` はフラグを受け付けないため、環境変数で指定します。

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

## 注意: ヘッドレス実行 (Linux)

sekien は OS ネイティブの WebView を使うため、Linux 環境のディスプレイがない環境（CI等）で実行する場合、`xvfb-run` が必要です。

```bash
xvfb-run sekien diagram.mmd > diagram.svg
```

レンダリングがうまくいかない場合は以下の環境変数をセットしてください：
- `GDK_BACKEND=x11`
- `WEBKIT_DISABLE_COMPOSITING_MODE=1`

## 構成

```
sekien/                 # Workspace Root
├── sekien/             # lib クレート (コアロジック)
│   ├── assets/
│   │   └── mermaid.min.js  # コンパイル時にバイナリへ埋め込まれる
│   └── src/lib.rs      # WebView レンダラ
├── sekien-cli/         # CLI クレート
│   └── src/main.rs     # sekien コマンド
└── sekien-pandoc/      # Pandoc フィルタクレート
    ├── assets/
    │   └── sekien.lua  # 同梱 Lua filter
    └── src/
        ├── main.rs     # sekien-pandoc コマンド
        └── pandoc.rs   # Pandoc AST 処理
```

## テスト

```bash
# ユニットテスト
cargo test

# 統合テスト (Linux では xvfb-run を推奨)
xvfb-run ./test.sh
```

## ビルド

```bash
cargo build --release
```

`sekien/assets/mermaid.min.js` (v11.14.0) はリポジトリに同梱済み。更新する場合は npm から取得して差し替える。

```bash
npm install mermaid
cp node_modules/mermaid/dist/mermaid.min.js sekien/assets/
```
