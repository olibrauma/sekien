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

#### HTML を経由しない PDF engine (Lua filter を使う)

typst や pdflatex など raw HTML を drop する PDF engine では、`sekien-pandoc` に同梱の Lua filter を併用することで SVG を画像として埋め込めます。

```bash
# Lua filter を書き出し、pandoc に渡す
sekien-pandoc --print-lua-filter > sekien.lua
pandoc input.md -o output.pdf --filter sekien-pandoc --lua-filter sekien.lua
```

## 注意: ヘッドレス実行 (Linux)

Linux 環境のディスプレイがない環境（CI等）で実行する場合、`xvfb-run` が必要です。

```bash
xvfb-run sekien diagram.mmd > diagram.svg
```

また、レンダリングがうまくいかない場合は以下の環境変数を試してください：
- `GDK_BACKEND=x11`
- `WEBKIT_DISABLE_COMPOSITING_MODE=1`

## 構成

```
sekien/                 # Workspace Root
├── sekien/             # lib クレート (コアロジック)
│   └── src/lib.rs      # WebView レンダラ
├── sekien-cli/         # CLI クレート
│   └── src/main.rs     # sekien コマンド
└── sekien-pandoc/      # Pandoc フィルタクレート
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
