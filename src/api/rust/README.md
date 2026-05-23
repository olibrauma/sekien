# sekien-api — Rust wrapper for sekien

Rust から [sekien](../../) バイナリを呼んで Mermaid を SVG に
変換するための ergonomic wrapper crate。`Command::spawn` で sekien を起動し、
stdin/stdout/stderr 越しの protocol (`\0` 区切りの streaming) を library
利用者から隠す。

Rust 以外の言語からは sekien バイナリの stdin/stdout protocol を直接叩けば
良いため、sekien-api は **Rust 利用者のための補助** という位置付け。

## 前提

実行時に [sekien](../../) バイナリが必要。

```bash
cargo install sekien
```

## インストール

```bash
cargo add sekien-api
```

## 使い方

```rust
use sekien_api::{render_blocks, BlockOutcome, RenderConfig};

let blocks = vec![
    "graph LR\n  A --> B".to_string(),
    "graph TD\n  X --> Y".to_string(),
];
let config = RenderConfig::default();

let outcomes = render_blocks("sekien", blocks, &config)?;
for outcome in outcomes {
    match outcome {
        BlockOutcome::Rendered(svg) => println!("{svg}"),
        BlockOutcome::Failed(msg) => eprintln!("failed: {msg}"),
    }
}
```

第 1 引数の `"sekien"` は PATH lookup。絶対パスを渡せばそれを直接実行する
(self-contained に配布したい場合は、caller アプリが sekien バイナリを bundle
してそのパスを渡せる)。

## API 概要

```rust
pub struct RenderConfig {
    pub font_family: Option<String>,
    pub theme: Option<String>,
    pub look: Option<String>,
}

pub enum BlockOutcome {
    Rendered(String),  // 成功: SVG
    Failed(String),    // 失敗: sekien stderr から抽出したエラーメッセージ
}

pub fn render_blocks(
    sekien: impl AsRef<OsStr>,
    blocks: Vec<String>,
    config: &RenderConfig,
) -> Result<Vec<BlockOutcome>, SekienApiError>;

pub fn mermaid_version(sekien: impl AsRef<OsStr>) -> Result<String, SekienApiError>;
```

## 性能特性

`render_blocks` は内部で sekien を 1 回だけ spawn する。N blocks 一括処理の
場合、起動コスト (Xvfb / WebView / mermaid.js の初期化) は 1 回分のみで、
render コストだけが N 倍になる:

    total = 起動コスト (1 回) + render コスト × N

直近の sekien 1 起動の実測値は
[sekien の README](../../README.md#mmdc-との比較) を参照。

## 設計の特徴

- **env hygiene**: caller プロセスの `SEKIEN_FONT` / `SEKIEN_THEME` /
  `SEKIEN_LOOK` を sekien に継承させない (`env_remove`)。caller の
  `RenderConfig` だけが sekien の振る舞いを決定する
- **per-block 失敗は Err にしない**: ある block の Mermaid 解析失敗は
  `BlockOutcome::Failed` で返す。プロセス全体の Err は sekien プロセス自身の
  失敗 (spawn 失敗、exit 1、I/O エラー等) に限定
- **`sekien` パスは caller が必須指定**: PATH lookup と絶対パス指定の両方を
  単一 API で扱えるようにし、bundled binary の道を残す

## 関連リポジトリ

- [sekien](../../): Mermaid → SVG レンダリングエンジン (binary)
- [sekien-pandoc](../../../2026-05-20-sekien-pandoc): Pandoc filter (binary)

詳細な実装方針と protocol 消費規約は [DESIGN.md](DESIGN.md) 参照。

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](../../LICENSE-APACHE) or https://www.apache.org/licenses/LICENSE-2.0)
- MIT License ([LICENSE-MIT](../../LICENSE-MIT) or https://opensource.org/licenses/MIT)

at your option.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall
be dual licensed as above, without any additional terms or conditions.
