# sekien-api — sekien の言語別 wrapper 集

`sekien` バイナリを各言語から呼ぶための **参照実装** をまとめる場所。各実装は
sekien バイナリの stdin/stdout protocol (`\0` 区切り streaming、stderr の
`Error: mermaid block N: <msg>` 行) をその言語に翻訳した形で書かれている。

配布チャネル (cargo / npm / pypi 等) には基本的に載せていない。利用者は該当
フォルダのコードを読んで自分のプロジェクトに取り込む想定。

## 言語別フォルダ

| 言語 | フォルダ | 配布形態 |
|---|---|---|
| Rust | [rust/](rust/) | cargo crate (`sekien-api`)。今のところ publish はしておらず、`sekien-pandoc` から path dep で利用 |
| TypeScript / JavaScript | (未実装) | — |
| Python | (未実装) | — |

## protocol の source of truth

各言語実装が翻訳している protocol 自体は **sekien バイナリ側** で決まる。仕様の
原典はこちら:

- [sekien バイナリの仕様 (../DESIGN.md)](../DESIGN.md#sekien-バイナリの仕様)
- [stderr error 出力規約 (../DESIGN.md)](../DESIGN.md#stderr-error-出力規約)
- [`\0` 区切り protocol (../DESIGN.md)](../DESIGN.md#区切り文字-0-nul-byte)

各 wrapper 実装はこの仕様を上書きしない。protocol を変更する時は `sekien`
バイナリ側 (`../DESIGN.md` + `../src/`) と同時に各 wrapper を更新する。

## sekien 本体と同一リポジトリに置いている理由

sekien-api を sekien バイナリと **同一 repo** に置いているのは、protocol
contract (`\0` 区切り stdin/stdout、`Error: mermaid block N:` 形式の stderr) に
直接 coupling しているため。具体的には:

- **atomic 更新**: protocol を変更する時、sekien バイナリ側と各 wrapper 側を 1
  commit で同期できる。別 repo だと PR / リリースのタイミングがずれて contract
  drift のリスクが生まれる
- **bisect 容易**: protocol regression を `git bisect` で原典 commit 1 つに辿れる
- **CI 統一**: `SEKIEN_TEST_BIN` 経由の e2e contract test
  ([rust/tests/e2e.rs](rust/tests/e2e.rs)) が同一 CI で完結する。cross-repo
  workflow が要らない

別 repo にすると "crates.io から sekien-api を直接たどりやすい" という
discoverability の利点はあるが、これは各 wrapper の README / Cargo.toml の
metadata (description / repository / documentation の link) で十分カバーできる。
contract drift のリスクを取ってまで切り出す動機は薄い。

なお [sekien-pandoc](../../2026-05-20-sekien-pandoc/) は **別 repo** にしている。
こちらは sekien-api 経由で protocol を間接参照しているため contract から 2 段
離れており、独立の repo に置いて pandoc filter 利用者が素直に辿れる方を優先。

## 新しい言語の wrapper を追加するには

`api/<lang>/` ディレクトリを作って、次の責務をその言語の慣習で実装する:

1. `\0` 区切りで blocks を stdin に書き込む (各 block 末尾に `\0`、末尾 1 個は sekien 側で drop される規約に乗る)
2. CLI フラグ (`--font` / `--theme` / `--look`) を caller 設定から構築
3. caller プロセスの `SEKIEN_FONT` / `SEKIEN_THEME` / `SEKIEN_LOOK` env を除去 (env hygiene)
4. stdout を `\0` で split して成功 block の SVG を取り出す
5. stderr を行単位で parse して `Error: mermaid block N: <msg>` から per-block 失敗 map を作る
6. input blocks と 1:1 で `Rendered(svg) | Failed(msg)` を組み立てる

Rust 実装 ([rust/](rust/)) を参考にすればだいたい同じ構造で書ける。
