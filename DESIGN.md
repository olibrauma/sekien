# sekien — 設計方針

## 概要

sekien は Cargo workspace 構成。3 クレートに分けている理由は責務の分離と crates.io への独立公開を両立するため:
- `sekien` (lib): Mermaid → SVG 変換のコアロジック。他のアプリから利用可能
- `sekien-cli` (bin): スタンドアロン CLI
- `sekien-pandoc` (bin): Pandoc フィルタ

## クレート構成

```
sekien/                  # workspace root
├── Cargo.toml           # [workspace]
├── sekien/              # lib クレート
│   ├── assets/
│   │   └── mermaid.min.js
│   └── src/
│       ├── lib.rs
│       └── linux_display.rs    # Linux 専用 (cfg-gated): Xvfb 起動と DISPLAY 設定
├── sekien-cli/
│   └── src/main.rs
└── sekien-pandoc/
    ├── assets/
    │   └── sekien.lua
    └── src/
        ├── main.rs
        └── pandoc.rs    # Pandoc AST 処理 (sekien-pandoc 固有)
```

### 各クレートの責務

| クレート | 種別 | 実行ファイル名 | 責務 |
|---|---|---|---|
| `sekien` | lib | — | Mermaid → SVG 変換のみ |
| `sekien-cli` | bin | `sekien` | ファイル / stdin から SVG を出力 |
| `sekien-pandoc` | bin | `sekien-pandoc` | Pandoc JSON フィルタとして動作 |

- `sekien-cli` は `[[bin]] name = "sekien"` により、`cargo install sekien-cli` で `sekien` コマンドとして入る。
- `pandoc.rs` (Pandoc AST のパース・シリアライズ) は `sekien-pandoc` 固有のロジックのため lib には置かない。

## 公開方針

3 クレートすべてを crates.io に公開する。

```sh
cargo add sekien                        # lib として使う
cargo install sekien-cli sekien-pandoc  # CLI ツールとして使う
```

バージョンとメタデータ (authors / license / repository 等) は各クレートが独立して管理する。`[workspace.package]` は使わない。

## lib の公開 API

`sekien` lib が公開する項目:

| 項目 | 種別 |
|---|---|
| `RenderConfig` | struct |
| `render_all` | fn |
| `MERMAID_VERSION` | const |

`build_html` / `create_window` / `create_webview` / `RenderState` は pub にしない。

### render_all のシグネチャ

tao のイベントループは呼び出し元に戻らないため、コールバック方式を採用する。
完了時は `on_complete` を呼んで `std::process::exit(0)` で終了する。
エラーは `Result` で返さず `eprintln!` + `exit(1)` で処理する。

```rust
pub fn render_all<F>(blocks: Vec<String>, config: &RenderConfig, on_complete: F) -> Result<()>
where
    F: FnOnce(Vec<String>) + Send + 'static
```

戻り値は `Result<()>` だが、内部で `std::process::exit` を呼ぶため実際には戻らない。
`Result` にしているのはイベントループ開始前のエラーを `?` で伝播できるようにするため。

### イベントループの実装 (tao)

イベントループは tao を使う。winit より Linux サポートが充実しているため。

Linux では 1x1 のウィンドウサイズが GDK のアサーションエラーを引き起こすため、
`#[cfg(target_os = "linux")]` で 100x100 に調整し、画面外 (-10000, -10000) に配置する。

### Linux display 解決 (linux_display モジュール)

`render_all` の冒頭、GTK 初期化より前に display backend を解決する。

#### GDK backend は常に X11 を強制

`GDK_BACKEND=x11` を必ずセットする。後段で起動する Xvfb は X server なので、
GDK にも X11 backend を選ばせる必要があるため。これを指定しないと Wayland
セッションでは GDK が `$WAYLAND_DISPLAY` を優先し、`DISPLAY` で指した Xvfb を
無視して Wayland コンポジタに接続してしまう。

#### Display の確保

`$DISPLAY` の有無に関わらず、常に内部で Xvfb を spawn して `$DISPLAY` を
上書きする。

`$DISPLAY` を利用しない理由: Wayland セッションでは Xwayland 経由で X11 backend
を使うことになるが、Wayland コンポジタがウィンドウ位置を制御するため off-screen
配置トリック (-10000, -10000) が無視されて一瞬画面に flash する。Xvfb は
in-memory framebuffer なので、そもそも画面が無く flash しない。
pure X11 セッションでもデスクトップ上で flash しないように、Linux では一律
Xvfb 経路に統一する。

Xvfb は `-displayfd 1 -terminate -screen 0 100x100x24 -nolisten tcp` で起動し、
Xvfb 自身が空き display 番号を選んで stdout に書き出すのを待つ
(`-displayfd` は X server が client 受付可能になったタイミングで発火する)。
socket file の存在だけでは server 完全 ready の前に GTK が接続を試みて失敗する
ため、このシグナルを使う。

`-terminate` により sekien 終了時に Xvfb も自動的に停止するため、明示的な
プロセス管理は不要。

#### GTK4 headless への将来的な移行

GTK 4.10+ で `GDK_BACKEND=headless` が利用可能になり、display server 自体が
不要になる。ただし wry 0.55 は GTK3 / webkit2gtk-4.x にハードコードされており、
GTK4 を選ぶ feature flag が無い。wry の GTK4 対応後、Xvfb 経路を headless に
差し替え可能。

## オプションと環境変数

### sekien-cli

CLI フラグを優先し、未指定の場合は環境変数にフォールバックする。

| フラグ | 環境変数 |
|---|---|
| `--font <font>` | `SEKIEN_FONT` |
| `--theme <theme>` | `SEKIEN_THEME` |
| `--look <look>` | `SEKIEN_LOOK` |

### sekien-pandoc

pandoc は filter を `sekien-pandoc <format>` として呼ぶため、追加フラグを渡す手段がない。
レンダリングオプションは環境変数のみで受け付ける (`SEKIEN_FONT`, `SEKIEN_THEME`, `SEKIEN_LOOK`)。

環境変数の名前空間は両ツールで共通とする。同じ lib で同じコードをレンダリングするため、出力を一致させたいケースがほとんどであるため。

### 環境変数の読み取り

環境変数の読み取りロジックはバイナリ側 (`sekien-cli`, `sekien-pandoc`) の責務とする。
lib が env var を暗黙に参照するとライブラリとしての汎用性が下がるため。

各バイナリが自分で env var を読み取り、`RenderConfig { .. }` を直接構築して渡す。

```rust
// sekien-cli: CLI フラグ優先、未指定なら env var
let config = RenderConfig {
    font_family: options.font_family.or_else(|| env::var("SEKIEN_FONT").ok()),
    theme:       options.theme      .or_else(|| env::var("SEKIEN_THEME").ok()),
    look:        options.look       .or_else(|| env::var("SEKIEN_LOOK").ok()),
};

// sekien-pandoc: env var のみ
let config = RenderConfig {
    font_family: std::env::var("SEKIEN_FONT").ok(),
    theme:       std::env::var("SEKIEN_THEME").ok(),
    look:        std::env::var("SEKIEN_LOOK").ok(),
};
```

## Pandoc との連携

```sh
# スタンドアロン
sekien diagram.mmd

# Pandoc フィルタ (HTML など SVG をそのまま扱える出力)
pandoc input.md -o output.html --filter sekien-pandoc

# Pandoc フィルタ + Lua フィルタ (typst / pdflatex など)
pandoc input.md -o output.pdf --pdf-engine=typst \
  --filter sekien-pandoc \
  --lua-filter <(sekien-pandoc --print-lua-filter) \
  -V mainfont="Hiragino Sans"
```

pandoc は filter を `sekien-pandoc <format>` で呼ぶ。format 引数は使わないため受け取って無視する。

`--filter <(sekien-pandoc ...)` は process substitution が実行不可な fd を生成するため pandoc では動作しない。
`--lua-filter` はテキスト読み取りなので `<(...)` が使える。
