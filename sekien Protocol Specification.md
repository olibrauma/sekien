# sekien Protocol Specification

- Version: 1
- Date: 2026-05-22

## Abstract

`sekien` は Mermaid 図を SVG にレンダリングする streaming filter で、stdin
(またはファイル引数) から Mermaid を受け取り、stdout に SVG を、per-block
失敗時には stderr に診断行を流す。本書は `sekien` バイナリの stdin / stdout /
stderr の wire protocol および exit code semantics を spec として固める。各言語の
wrapper 実装者は本書のみを参照して interop 可能とし、Rust 参照実装を読む必要は
ない。protocol は major version で管理し、本書は version 1 を定義する。

## 1. Terminology

本書は RFC 2119 / RFC 8174 の以下のキーワードを大文字で使う:

- **MUST**, **MUST NOT**, **SHOULD**, **SHOULD NOT**, **MAY**

その他の用語:

- **block**: stdin (またはファイル引数) に渡される 1 つの Mermaid source 単位
- **SVG**: 成功した block の rendered 出力 (stdout に書かれる)
- **input-position**: stdin における block の 1-origin 順番
- **wrapper**: `sekien` バイナリを子プロセスとして起動し protocol を実装する
  caller プログラム (Rust の sekien-api crate, sekien-pandoc, 他言語実装等)

## 2. Overview

`sekien` は stdin (またはファイル引数) を EOF まで読み続ける streaming filter。
入力 byte stream は NUL byte (0x00) で区切られた Mermaid block の列として
解釈される。各 block について以下のいずれか:

- 成功時: rendered SVG を stdout に書く
- 失敗時: per-block 診断行を stderr に書く

per-block 失敗は binary を abort させず、stream は EOF まで継続する。これは
"streaming filter" としての設計上の選択であり、bulk 入力で一部 block が
失敗しても残りの成功 block を yield することを意図する。

exit code は正常完了で 0 (per-block の成否を問わない)、`sekien` 自身の internal
failure で 1。

stdin とファイル引数 (`sekien file.mmd`) は protocol 上等価で、ファイル引数が
指定された場合はその内容を stdin に流したのと同じ扱いになる。

## 3. Syntax (ABNF)

各 stream の byte-level 構造は ABNF (RFC 5234) で定義する。

### 3.1 stdin

```abnf
stdin        = [ block *(NUL block) [NUL] ]
block        = *non-NUL-byte
non-NUL-byte = %x01-FF
NUL          = %x00
```

注:

- block は NUL byte を含んではならない (MUST NOT)
- block 全体 (NUL 区切りで切り出した各 byte 列) は valid UTF-8 でなければならない
  (MUST)。違反時、`sekien` は exit 1 で終わる (section 3.4 参照)
- 末尾の NUL は OPTIONAL。EOF 直前に存在する場合、追加の空 block を導入しては
  ならない (MUST NOT)。これは Unix の慣習 (`find -print0`, `printf '%s\0' a b c`,
  `xargs -0` 等) に整合させるため
- 空 stdin (0 byte) は 0 block として扱い、出力も 0 byte
- 2 個以上連続する trailing NUL の 2 個目以降は空 block を導入する。空 block は
  Mermaid parse で失敗する

### 3.2 stdout

```abnf
stdout       = [ svg *(NUL svg) ]
svg          = *non-NUL-byte
```

注:

- 成功した block のみが stdout に寄与し、順序は input-position 順
- 失敗 block は stdout に何も寄与しない (空 separator も含まない)
- stdout は NUL byte で始まっても終わってもならない (MUST NOT)
- stdout 中の SVG 数は `(input block 数) - (stderr 中の block-error 行数)` に等しい
- SVG は通常 LF (`0x0A`) を含む multi-line XML (mermaid.js 由来)。wrapper は SVG
  内容に line-oriented parsing (例: `head -1`) を適用してはならない (MUST NOT)。
  block 単位の処理には `\0` を区切りとして使う (例: `awk -v RS='\0' 'NR==1'`,
  `tr '\0' '\n'`)
- `sekien` は block の render 完了ごとに即座に stdout を flush して良い (MAY)

### 3.3 stderr

```abnf
stderr       = *(line LF)
line         = block-error / diagnostic
block-error  = "Error: mermaid block " 1*DIGIT ": " *non-LF-byte
diagnostic   = *non-LF-byte

LF           = %x0A
DIGIT        = %x30-39
non-LF-byte  = %x00-09 / %x0B-FF
```

注:

- 各 line は LF (0x0A) 1 個で終端される
- wrapper が `block-error` として parse してよいのは、リテラル
  `Error: mermaid block ` (末尾の space 含む) で始まる行に限る (MUST)。それ以外の
  行は diagnostic として扱い、per-block 失敗に再構成してはならない (MUST NOT)
- `block-error` 中の `1*DIGIT` 部は 1-origin の input-position で、stdin に
  渡された block 数 `N` に対して `1..=N` の範囲 (MUST)
- `: ` の後は Mermaid error message で、`\n` (`0x0A`) および `\r` (`0x0D`) は
  単一 space (`0x20`) に置換され、1 行に収まる (MUST)

### 3.4 Exit code

```abnf
exit-code    = "0" / "1"
```

- `0`: stdin を EOF まで処理完了 (per-block の成否を問わない)
- `1`: `sekien` 自身の internal failure。具体例: CLI argument error, display
  初期化失敗, WebView からの malformed IPC, stdout write 失敗

per-block の Mermaid parse error は exit code に反映しない (MUST NOT)。これは
section 4.4 を参照。

## 4. Semantics

### 4.1 Block correspondence

stdin に `N` 個の block が input-position 順 `1, 2, ..., N` で並ぶとき、`sekien` が
exit code 0 で終了した場合、出力 stream は次を満たす。

各 `i ∈ 1..=N` について以下のいずれか 1 つが成り立つ:

1. **成功**: i 番目の block の SVG が stdout 内の対応位置 (= i より前の成功 block
   数) に出る
2. **失敗**: `Error: mermaid block i: <msg>` の 1 行が stderr に出る。stdout には
   何も寄与しない

両者は決定論的で、per-block 結果は stdout と stderr の組から復元可能。

復元 algorithm (wrapper 側、pseudocode):

```
# Inputs:
#   N         = number of blocks sent on stdin
#   stdout    = bytes from sekien's stdout
#   stderr    = bytes from sekien's stderr
#   exit_code = process exit status
#
# Output:
#   result[1..=N], each one of Rendered(svg) | Failed(message)

require exit_code == 0  # else: protocol-level failure, do not reconstruct

errors = parse_block_errors(stderr)
#   parse stderr lines matching "Error: mermaid block <i>: <msg>"
#   into a map { i -> msg }
svgs = stdout.split(NUL)
#   length = N - len(errors)

j = 0
for i in 1..=N:
    if i in errors:
        result[i] = Failed(errors[i])
    else:
        result[i] = Rendered(svgs[j])
        j += 1
```

wrapper は復元の前に protocol invariant を verify すべき (SHOULD): (i) stderr
から抽出された失敗 `N` の集合は `1..=N` (N = 入力 block 数) に含まれる、かつ
(ii) stdout の SVG 数 + 失敗数 == 入力 block 数。違反検出時は protocol violation
として扱い、復元結果を返してはならない (MUST NOT)。

exit code が 1 のとき、wrapper は invocation を全体失敗として扱わねばならない
(MUST)。部分的な stdout / stderr は diagnostic としてのみ参照してよい (MAY)。
途中まで処理された block の結果は信頼してはならない (MUST NOT)。

### 4.2 Streaming and flushing

`sekien` は block の render 完了ごとに SVG を即座に stdout に flush して良い
(MAY)。wrapper は最小・最大の buffering を仮定してはならず (MUST NOT)、partial
read を扱える形で実装しなければならない (MUST)。

これにより `sekien | head -1` のような pipeline でも最初の SVG が即座に下流に
届く性質が保たれる。

### 4.3 Configuration

rendering parameter は以下:

| パラメータ | CLI flag | environment variable | 値 |
|---|---|---|---|
| font | `--font <name>` | `SEKIEN_FONT` | CSS font-family 文字列 |
| theme | `--theme <name>` | `SEKIEN_THEME` | mermaid.js が受け付ける theme 名 |
| look | `--look <name>` | `SEKIEN_LOOK` | mermaid.js が受け付ける look 名 |

- CLI flag が environment variable に優先する
- 設定は 1 回の `sekien` 起動の lifetime で固定で、block ごとに変化させること
  はできない (MUST NOT)
- `theme` / `look` の有効値は bundle されている mermaid.js version に依存し、
  protocol contract の一部ではない。wrapper は値を opaque な文字列として
  扱い、`sekien` に転送するのみとする

### 4.4 Continue-on-error

per-block の Mermaid parse error は `sekien` の exit を引き起こしてはならない
(MUST NOT)。bulk 入力で一部 block に syntax error があっても、残りの成功 block を
yield することを意図する。

fail-fast semantics が欲しい wrapper は stderr の `block-error` 行を inspect
して early-return する形で実装する (`sekien` 側で fail-fast を選ぶ手段は無い)。

### 4.5 Environment hygiene

wrapper は `sekien` を spawn する際、caller プロセスから継承される
`SEKIEN_FONT` / `SEKIEN_THEME` / `SEKIEN_LOOK` を child の environment から
明示的に削除すべき (SHOULD)。wrapper が渡す `RenderConfig` 等の構造化された
設定のみが `sekien` の振る舞いを決めるようにし、caller の shell に置かれた env が
silent に influence する状態を避けるため。

## 5. Versioning

protocol は major version で管理する。本書は version `1` を定義する。

将来 protocol が後方非互換に変更される場合 (例: stderr の `block-error` 行形式
変更、separator の変更)、version を 2 に上げ、新版を本書とは別文書として
発行する。

`sekien` バイナリが自身の protocol version を露出する手段 (例: 専用 CLI flag、
banner 出力) は本書では規定しない。将来の version では runtime negotiation
mechanism が追加される可能性がある。それまでの間、wrapper は `sekien` の binary
version (`sekien --version` の出力) と protocol version の対応を out-of-band で
解決する。

## 6. Examples

凡例: `<NUL>` は 1 byte の `0x00`、`<EOF>` は stream の終端、`\n` は LF
(`0x0A`)。

### 6.1 Single block, success

```
stdin:  graph LR\n  A --> B<EOF>
stdout: <svg ...>\n  <g>...</g>\n</svg><EOF>
stderr: (empty)
exit:   0
```

SVG 内に LF が含まれる点に注意 (mermaid.js が multi-line XML を返すため)。
`sekien | head -1` は SVG 途中で truncate してしまうので、block 単位で区切るに
は `\0` を separator として使う必要がある (例:
`sekien | awk -v RS='\0' 'NR==1'`)。

### 6.2 Three blocks, second fails

```
stdin:  m1<NUL>BAD<NUL>m3<EOF>
stdout: <svg1><NUL><svg3><EOF>
stderr: Error: mermaid block 2: Parse error on line 1\n
exit:   0
```

復元: block 1 = Rendered(svg1), block 2 = Failed("Parse error on line 1"),
block 3 = Rendered(svg3)。

### 6.3 Trailing NUL (1 個)

```
stdin:  m1<NUL>m2<NUL><EOF>
stdout: <svg1><NUL><svg2><EOF>
stderr: (empty)
exit:   0
```

`m1<NUL>m2<NUL>` は 2 block と解釈される (3 ではない)。末尾の NUL は追加の空
block を導入せずに consume される。

### 6.4 Trailing NUL (2 個連続)

```
stdin:  m1<NUL>m2<NUL><NUL><EOF>
stdout: <svg1><NUL><svg2><EOF>
stderr: Error: mermaid block 3: <msg>\n
exit:   0
```

2 個目の trailing NUL は 3 番目の (空) block を導入する。空文字列は Mermaid
parse に失敗するため stderr に `block 3` の error が出る。

### 6.5 Empty stdin

```
stdin:  (empty)
stdout: (empty)
stderr: (empty)
exit:   0
```

### 6.6 Internal failure (exit 1)

```
stdin:  m1<NUL>m2<EOF>
stdout: <svg1>(途中で打ち切り)
stderr: Error: malformed IPC from webview: <details>\n
exit:   1
```

`malformed IPC from webview` は diagnostic line (block-error の prefix と
一致しない) であり、wrapper は per-block 失敗として扱ってはならない。exit code
が 1 なので、wrapper は invocation 全体を失敗として処理する。

## 7. Security Considerations

- `sekien` は Mermaid source を OS の WebView で評価する。untrusted Mermaid
  input を流す場合、wrapper は SVG output の取り扱いをレビューすべき (SHOULD)
- wrapper は SVG output が user-controlled なデータを含み得ることを前提に扱う
  べき (SHOULD)。特に HTML context で script 実行が可能な状況に埋め込む場合は
  事前 sanitize が必要
- `sekien` は display infrastructure (Linux: Xvfb, macOS: WKWebView, Windows:
  WebView2) を要し、それらの security posture を継承する
- `sekien` の environment variable (`SEKIEN_*`) は CLI flag に上書きされない
  限り `sekien` の振る舞いに影響する。wrapper は section 4.5 に従い caller の
  env を遮断すべき (SHOULD)

## 8. References

- [RFC 5234](https://datatracker.ietf.org/doc/html/rfc5234) — Augmented BNF for
  Syntax Specifications: ABNF
- [RFC 2119](https://datatracker.ietf.org/doc/html/rfc2119) — Key words for use
  in RFCs to Indicate Requirement Levels
- [RFC 8174](https://datatracker.ietf.org/doc/html/rfc8174) — Ambiguity of
  Uppercase vs Lowercase in RFC 2119 Key Words
- [Mermaid.js](https://mermaid.js.org/) — diagram syntax and rendering engine
  used internally by `sekien`
