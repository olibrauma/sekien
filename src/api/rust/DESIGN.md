# sekien-api — 設計方針

## 概要

sekien-api は [sekien](../../) バイナリを Rust から呼ぶための
ergonomic wrapper library。`Command::spawn` で sekien を起動し、stdin/stdout/stderr
越しの protocol (`\0` 区切りの streaming) を library 利用者から隠す。

## 関連リポジトリ

- [sekien](../../): Mermaid → SVG レンダリングエンジン (binary)
- [sekien-pandoc](../../../2026-05-20-sekien-pandoc): Pandoc filter (binary、本 lib を内部で利用)

## 位置づけ

Rust 以外の言語からは sekien の stdin/stdout protocol を直接叩けば良いため、
sekien-api は **Rust 利用者のための補助** という位置付け。sekien-pandoc も
client コードの重複を避けるため sekien-api 経由で sekien を呼ぶ。

## 依存する protocol

sekien バイナリの stdin/stdout/stderr protocol は
[sekien/DESIGN.md - sekien バイナリの仕様](../../DESIGN.md#sekien-バイナリの仕様) を
source of truth とする。本ドキュメントは消費側 (sekien-api 側) としての挙動と、
protocol が要求する追加 invariant を述べる。

## 界面定義: sekien-pandoc / 3rd-party app → sekien-api

これは Rust API の境界。sekien-api crate の公開 API がそのまま interface。

呼び出し規約:

| 項目 | 仕様 |
|---|---|
| `sekien` | sekien バイナリの場所。`"sekien"` を渡せば PATH lookup、絶対パスを渡せば直接実行 |
| `blocks` | raw Mermaid 文字列のリスト (各要素に `\0` を含めない) |
| `config` | caller が明示的に構築。env を読みたい caller は自分で読んで詰める |
| 戻り値 (Ok) | `blocks` と同順・同数の `Vec<BlockOutcome>`。失敗 block は `Failed(msg)` として位置を保持 |
| 戻り値 (Err) | sekien プロセス自身の失敗 (spawn 失敗、exit 1、I/O エラー等)。per-block 失敗では Err にしない |
| 空入力 | sekien を spawn せず `Ok(vec![])` を即返却 |
| 副作用 | sekien プロセスを spawn する以外なし。sekien-api 自身は env や stdio に触れない |

`sekien` 引数を必須にしている理由は、3rd-party app に **使い分けの選択肢を残す**
ため:

- 多くのケース: `"sekien"` を渡せば PATH lookup (利用者に `cargo install sekien`
  を要求する一般的な配布方法)
- self-contained に配布したい場合: caller が自前で sekien binary を bundle
  (`include_bytes!` + tempfile 展開) してそのパスを渡す

API を 1 つにすることで、似た API が複数並ぶ複雑さを避けている。"PATH lookup
だけ" "明示パスだけ" のどちらかを default 化すると、もう片方を後付けする際に
API surface が増えてしまうため、最初から 1 つに統一。

## 公開 API

```rust
#[derive(Clone, Default)]
pub struct RenderConfig {
    pub font_family: Option<String>,
    pub theme: Option<String>,
    pub look: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum BlockOutcome {
    Rendered(String),  // 成功: SVG 文字列
    Failed(String),    // 失敗: sekien stderr から抽出したエラーメッセージ
}

pub fn render_blocks(
    sekien: impl AsRef<OsStr>,
    blocks: Vec<String>,
    config: &RenderConfig,
) -> Result<Vec<BlockOutcome>, SekienApiError>;

pub fn mermaid_version(sekien: impl AsRef<OsStr>) -> Result<String, SekienApiError>;
```

`mermaid_version` は sekien バイナリに `--version` を渡して mermaid.js の
バージョンを取り出す関数。const として持たないのは、source of truth が
sekien バイナリに同梱されている mermaid.min.js だからで、build 時と runtime の
乖離を避けるため実行時に問い合わせる。

## 実装方針

`render_blocks` は内部で `sekien` バイナリを 1 回 spawn し、stdin/stdout/stderr
を介して通信する:

1. `Command::new(sekien)` で起動 (caller が指定した path / name)
2. `config` を CLI フラグ (`--font`, `--theme`, `--look`) に変換して渡す
3. 各 block の末尾に `\0` を付けて stdin に書き、close
   (末尾の `\0` 1 個は sekien 側で drop されるので、`blocks` に空文字列が
   含まれていても N 個として正しく解釈される — sekien/DESIGN.md の表 `m1\0m2\0\0`
   → 3 blocks 参照)
4. `wait_with_output` で stdout / stderr / 終了コードを取得
5. stdout を `\0` で split → **成功した SVG** のリスト (input より少ない場合がある)
6. stderr を行単位で parse → `Error: mermaid block N: <msg>` の N と msg を抽出
7. 1..=blocks.len() を iterate して per-block の `BlockOutcome` を再構成
   - 失敗 map に block N が居れば `Failed(msg)`、それ以外は stdout iter から
     次の SVG を取って `Rendered(svg)`

```rust
pub fn render_blocks(
    sekien: impl AsRef<OsStr>,
    blocks: Vec<String>,
    config: &RenderConfig,
) -> Result<Vec<BlockOutcome>, SekienApiError> {
    if blocks.is_empty() { return Ok(vec![]); }

    let mut child = build_command(sekien.as_ref(), config).spawn()?;
    {
        let mut stdin = child.stdin.take().unwrap();
        // 各 block 末尾に \0 を書く (最後の \0 は sekien 側で drop される規約に乗る)
        for b in &blocks {
            stdin.write_all(b.as_bytes())?;
            stdin.write_all(&[0])?;
        }
    }
    let output = child.wait_with_output()?;
    if !output.status.success() { return Err(/* sekien プロセス失敗 */); }

    let success_svgs: Vec<String> = output.stdout
        .split(|&b| b == 0).filter(|s| !s.is_empty())
        .map(|bs| std::str::from_utf8(bs).map(str::to_string))
        .collect::<Result<_, _>>()?;
    let failures = parse_stderr_failures(std::str::from_utf8(&output.stderr)?);

    let mut iter = success_svgs.into_iter();
    Ok((1..=blocks.len()).map(|n|
        if let Some(msg) = failures.get(&n) { BlockOutcome::Failed(msg.clone()) }
        else { BlockOutcome::Rendered(iter.next().unwrap()) }
    ).collect())
}
```

### stderr 失敗の parse

sekien バイナリの stderr 出力規約
([sekien/DESIGN.md - stderr error 出力規約](../../DESIGN.md#stderr-error-出力規約))
に従い、`Error: mermaid block <N>: <msg>` の行を抽出して per-block 失敗 map
を構築する:

- `<N>` を 1-origin で `1..=blocks.len()` の範囲に対して突き合わせる
- `Error: mermaid block ` で始まらない stderr 行 (sekien 自身の診断メッセージ等)
  は無視
- per-block の SVG が stdout から消えているのに、対応する失敗行が stderr に
  無い場合は `ProtocolViolation` として全体エラーに昇格する。例: stdout に
  期待数より少ない SVG しか無く、stderr の `Error: mermaid block N:` 行と
  突き合わせても説明がつかないケース

### 非戦略事項

- `process::exit` を呼ばない。普通の関数として戻る
- イベントループや WebView 等は触らない。それらは sekien バイナリ側の責務
- 設定は CLI フラグ経由で渡す。環境変数 (`SEKIEN_FONT` 等) は sekien-api 自身は
  読まない (3rd-party caller の責務)
- per-block 失敗は `Err` ではなく `Vec<BlockOutcome>` 内の `Failed` で表現する。
  プロセス全体の Err は sekien バイナリ自身の失敗だけに限定する

## 設定の受け渡し (sekien-api 側の責務)

### 経路 B: sekien-api 経由 (3rd-party app)

```
3rd-party app が組み立てた RenderConfig
        ↓ sekien-api が CLI フラグに変換し、SEKIEN_FONT/THEME/LOOK を除去して spawn
sekien バイナリの CLI フラグ (--font, --theme, --look)
        ↓ sekien main が parse (SEKIEN_FONT 等は消えているので CLI フラグだけが効く)
mermaid.initialize() の引数
```

### caller の RenderConfig → CLI フラグの変換

```rust
if let Some(f) = &config.font_family { cmd.args(["--font", f]); }
if let Some(t) = &config.theme       { cmd.args(["--theme", t]); }
if let Some(l) = &config.look        { cmd.args(["--look", l]); }
```

`None` のフィールドはフラグ無し。後述の env 除去と合わせて、sekien バイナリは
**caller が明示した値だけ** を受け取る。

### env hygiene: SEKIEN_OWNED_ENV_VARS の除去

sekien-api は `RenderConfig` を受け取って CLI フラグとして sekien に渡す際、
**caller プロセスの `SEKIEN_FONT` / `SEKIEN_THEME` / `SEKIEN_LOOK` を sekien に
継承させない**:

```rust
const SEKIEN_OWNED_ENV_VARS: &[&str] = &["SEKIEN_FONT", "SEKIEN_THEME", "SEKIEN_LOOK"];

let mut cmd = Command::new(sekien.as_ref());

// sekien バイナリが解釈する env のみ明示的に除去する (HOME や PATH は残す)
for key in SEKIEN_OWNED_ENV_VARS {
    cmd.env_remove(key);
}
```

### env を読まない設計の理由

caller プロセスの `SEKIEN_FONT` 等は、**caller アプリが意図して set したもの**
とは限らず、**利用者の shell が export したもの** である可能性がある。例えば:

- 利用者の `.zshrc` に `export SEKIEN_FONT="Comic Sans"`
- 利用者が 3rd-party app を起動
- 3rd-party app が `render_blocks("sekien", blocks, &RenderConfig::default())` を呼ぶ
- 3rd-party app の意図: mermaid デフォルトフォントで描画したい
- env をそのまま継承すると: sekien が `SEKIEN_FONT="Comic Sans"` を読んで使ってしまう
- → 3rd-party app の意図から外れる

sekien-api が該当 env を除去することで、caller の RenderConfig だけが
sekien の振る舞いを決定する。**caller が意図的に env を sekien に渡したい場合
は、自分で env::var を読んで RenderConfig に詰めて render_blocks を呼ぶ** こと
で実現できる ([sekien-pandoc](../../../2026-05-20-sekien-pandoc) がこのパターンを使う)。

### バイネーム指定の理由

`SEKIEN_` prefix で一律除去する (`starts_with` ベース) のではなく、sekien
バイナリが解釈する **3 変数だけをバイネームで除去** する。これは caller
アプリが将来自分独自の `SEKIEN_*` env (例: caller が自前で読む `SEKIEN_BIN` や
`SEKIEN_DEBUG`) を定義する余地を残すため。prefix 一括除去だと、それらが
sekien spawn 直前に消されて caller の意図に反する。"sekien バイナリ自身が
読むもの" だけを許可リスト管理することで、namespace を共有しつつ干渉を防ぐ。

`env_clear()` を使わない理由: HOME / PATH / LANG / 等の generic な env も
消してしまい、Xvfb 起動や dynamic linker が動かなくなる可能性がある。
sekien が読む env だけを selective に除去するのが安全。

## 性能特性

`render_blocks` が sekien を 1 回 spawn するので、起動コスト
(Xvfb / GTK / WebView / mermaid.js 初期化) は 1 回分のみ。N blocks 一括処理では:

    total = 起動コスト (1 回) + render コスト × N

これがそれぞれ sekien を N 回 spawn する設計だと
`起動コスト × N + render コスト × N` となり、N が大きいほど差が開く。
`\0` 区切り protocol によるこの償却が pandoc filter (多数の Mermaid を含む
文書) や bulk 変換のユースケースで効く。

直近の sekien 1 起動の実測値は
[sekien の README](../../README.md#mmdc-との比較) を参照。

## 公開方針

```bash
cargo add sekien-api
```

利用者は `cargo install sekien` も別途必要 (実行時に sekien バイナリを spawn
するため)。

## なぜこの設計か

### なぜ sekien-api を経由するのか (sekien を直接 spawn しない)

Rust caller が `Command::new("sekien")` を直接書くこともできるが、
`\0` の write/split や stderr のエラーハンドリング等の boilerplate を毎回書く
ことになる。sekien-api はこの client コードを **1 箇所に集約** する。

sekien-pandoc も sekien-api を使う。client code の重複を避け、protocol が
変わった場合に追従しやすくするため。

### なぜ sekien-api は lib のみで bin を含まないか

sekien-api 自体は変換ロジックを持たない。`sekien` バイナリを spawn するだけ
の薄い wrapper。bin としての存在理由が無い。

### なぜ caller に sekien path を必須にしているか

caller アプリの配布形態には PATH lookup と bundled binary の 2 通りがあり、
どちらも単一 API でサポートするため。詳細は前述の "界面定義" 節を参照。
