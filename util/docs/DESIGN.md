# sekien — 設計方針

## 概要

sekien は Mermaid 図を SVG にレンダリングする CLI バイナリ。stdin (または
ファイル引数) で Mermaid を受け取り、stdout に SVG を返す streaming プロセス。

## 関連リポジトリ

- [sekien-api](api/rust/): sekien を Rust から呼ぶ wrapper (lib)
- [sekien-pandoc](../2026-05-20-sekien-pandoc): Pandoc filter (binary)

3 つは streaming protocol (`\0` 区切り stdin/stdout) で連携する。protocol
仕様の source of truth は本ドキュメント (sekien バイナリ側)。

## アーキテクチャの中心思想

**sekien バイナリは cat のような streaming プロセス**。stdin (またはファイル) から
Mermaid を受け取り、`\0` を区切りに 1 block ずつ SVG を stdout へ流す。EOF まで
生存し、block 単位の失敗は stderr に流して継続する (continue-on-error)。

CLI として人間が使うときも、Rust ライブラリ経由で別アプリから使うときも、
Pandoc フィルタが内部で使うときも、**全く同じ stdin/stdout/stderr protocol** で
動作する。

```
人間 (shell)         ─→ sekien diagram.mmd > out.svg
sekien-api (Rust)    ─→ Command::new("sekien").spawn() で stdio を使う
sekien-pandoc        ─→ sekien-api 経由で同上
他言語 (Python等)    ─→ Command::new("sekien") で同上
```

sekien バイナリの動作モードは **1 種類のみ**。単発 CLI 利用も bulk 利用も
対話利用も、すべて同じ streaming protocol。複数 Mermaid を一括処理したい場合は
**`\0` (NUL byte) で区切って** stdin に流す。

## sekien バイナリの仕様

### 入力フォーマット (stdin or ファイル引数)

stdin (またはファイル引数の内容) は **Mermaid コード**。複数ブロックを一括処理
する場合は `\0` (NUL byte) で区切る:

```
graph LR\n  A --> B<NUL>graph TD\n  X --> Y<NUL>graph LR\n  P --> Q<EOF>
```

ここで `<NUL>` は `\0` (1 バイト)。

- 区切り 0 個 (`\0` 無し) → 1 ブロック (= 単独 CLI 利用の通常ケース)
- 区切り N 個 → N+1 ブロック
- 空入力 → 0 ブロック (即 exit 0)

**末尾 `\0` の扱い**: ストリーム末尾 (EOF 直前) の `\0` **1 個だけ**は無視する。
これは `printf '%s\0' a b c` や `find -print0` のような Unix 慣習で trailing
NUL が自然に付くケースを救うため:

| 入力 | ブロック数 | 解釈 |
|---|---|---|
| `m1\0m2` | 2 | 通常の 2 ブロック |
| `m1\0m2\0` | 2 | 末尾 `\0` 1 個は drop (空 block にしない) |
| `m1\0m2\0\0` | 3 | 2 個目以降の trailing は空 block (`m1`, `m2`, `""`) |
| `\0` | 1 | 空 block 1 件 (区切りが先) |

`m1\0m2\0\0` のような形は実用上稀だが、規約として "trailing `\0` は 1 個だけ
無視" を固定することで、Unix ツール出力の取り扱いと境界の挙動が一意になる。

### 出力フォーマット (stdout)

**成功した block の SVG だけ** を入力順で stdout に出す。SVG 間は `\0` で
区切る (先頭・末尾には `\0` を付けない)。

全 block が成功するときの例:

```
入力: m1<EOF>              → 出力: <svg1><EOF>                  (CLI 単発)
入力: m1<NUL>m2<EOF>       → 出力: <svg1><NUL><svg2><EOF>       (2 ブロック)
入力: m1<NUL>m2<NUL>m3<EOF>→ 出力: <svg1><NUL><svg2><NUL><svg3><EOF>
入力: <EOF>                → 出力: <EOF>                        (0 ブロック)
```

block 単位の失敗があると、その block 分は stdout に出ず、代わりに stderr に
`Error: mermaid block N: <msg>` が 1 行流れる。block 2 が失敗した例:

```
入力: m1<NUL>BAD<NUL>m3<EOF>
  → stdout: <svg1><NUL><svg3><EOF>
  → stderr: Error: mermaid block 2: <msg>\n
  → exit 0
```

stdout の output は streaming で逐次 flush される。caller (sekien-api 等) は
`wait_with_output` で全部読んでから処理しても、`read` で streaming 受信しても
良い。

### CLI フラグ

設定は CLI フラグまたは環境変数で渡す:

| フラグ | 環境変数 | 説明 |
|---|---|---|
| `--font <name>` | `SEKIEN_FONT` | フォント (CSS font-family 形式) |
| `--theme <name>` | `SEKIEN_THEME` | mermaid.js テーマ |
| `--look <name>` | `SEKIEN_LOOK` | 描画スタイル |
| `--block-id` | — | 各 SVG 出力の先頭にブロック ID (<!-- block: N -->) を付与 |
| `--version`, `-v` | — | バージョン表示 |
| `--help`, `-h` | — | ヘルプ表示 |

CLI フラグが優先、未指定時は環境変数。**1 回の sekien 起動内では設定は共通**
(ブロックごとに異なる設定にはできない)。

### 終了ステータス

| code | 意味 |
|---|---|
| 0 | EOF まで処理完了 (per-block の成否を問わない) |
| 1 | sekien 自身の失敗 (CLI 引数エラー、display 初期化失敗、malformed IPC、stdout 書き込み失敗等) |

per-block の Mermaid 解析エラーは **exit code に反映しない**。これは sekien が
"成果物を作る変換器" ではなく "stream を処理し続けるフィルタ" だから:

- bulk 入力では一部成功・一部失敗が日常的に起きる。集約的成否を 1 つの exit
  code で表すと、caller (sekien-api / shell pipeline) は結局個別の結果を
  per-block で確認することになる
- exit 1 をすべての失敗パスに統合すると、sekien バイナリ自身の internal error
  (報告すべき重大な失敗) と per-block ユーザーエラー (継続して良い軽微な失敗)
  が区別できなくなる

per-block の成否は **stderr の `Error: mermaid block N:` 行** で伝える。
sekien-api はこれを parse して per-block の `BlockOutcome` に再構成する。
shell pipeline は無視するか `grep` で集計するかを選べる。

### stderr error 出力規約

失敗 block ごとに stderr へ次の形式で 1 行を出す:

```
Error: mermaid block <N>: <msg>
```

- `<N>` は input 順での 1-origin block 番号 (必ず `1..=入力ブロック数` の範囲)
- `<msg>` は mermaid.js が投げた error message。改行は空白に置換して 1 行に収める

この行形式は **sekien バイナリ ↔ sekien-api 間の protocol contract**。両者を
同時に変更する必要があり、片方だけ変えると per-block 結果の再構成が壊れる。

sekien は上記以外にも stderr に診断メッセージを書くことがある (例: `Error:
malformed IPC from webview: ...`)。sekien-api は `Error: mermaid block ` で
始まる行だけを per-block 失敗として拾い、それ以外の行は無視する。

error 型 IPC で `<N>` または `<msg>` に対応するフィールドが欠落していた場合、
sekien は malformed IPC として exit 1 する (`Error: malformed IPC from webview`
を stderr へ)。これは silent な誤帰属を避けるため。

### 対話モード

sekien は cat と同じく terminal からも対話的に使える:

```text
$ sekien
graph LR
  A --> B
^@
<svg がその場で出る>
graph TD
  X --> Y
^@
<svg がその場で出る>
^D
$
```

terminal の canonical mode で `Ctrl + @` が NUL byte (`\0`) の入力手段、
`Ctrl + D` が EOF を投げる手段。block 末尾の改行は mermaid が無視するので、
`Ctrl + @` の前に Enter を打っても問題ない (実用上 Enter → `^@` → Enter の
順で打つことになる)。

## 区切り文字: `\0` (NUL byte)

### なぜ `\0` か

- **mermaid / SVG content に出現しない**: 両者ともテキスト (printable ASCII + UTF-8)
  なので NUL バイトは出現しない
- **Unix tool 慣習の中心**: `find -print0`, `xargs -0`, `sort -z`, `grep -z`,
  `tr '\0' '\n'`, bash の `read -d ''` 等、"改行を含むかもしれないデータの
  区切り" として `\0` を使う慣習が確立している。sekien はこの ecosystem に
  そのまま乗れる
- **POSIX shell でも書きやすい**: `printf '%s\0' a b c` で複数引数を `\0` 区切り
  で出力できる
- **言語横断的に扱いやすい**: Rust (`Read::read_until(0, ...)`)、Python
  (`bytes.split(b'\\x00')`)、Node (`buffer.split('\\0')`) など、どの言語からも素直

### 用法

本 protocol では `\0` を **separator (区切り)** として使う。N ブロックの間に
N-1 個の `\0` が入る:

```
m1\0m2\0m3<EOF>
```

### Unix pipeline 利用例

たいていのユースケースは ".mmd N 個 → .svg N 個" なので shell loop で十分:

```bash
for f in docs/*.mmd; do
  sekien "$f" > "${f%.mmd}.svg"
done
```

文書 1 つに大量の図がある等で Xvfb 起動コスト (~200ms × N) を抑えたい場合は、
`\0` 区切り protocol を活かして 1 回の sekien 起動にまとめ、stream を `\0` で
分けて書き戻せる:

```bash
files=(docs/*.mmd)
for f in "${files[@]}"; do cat "$f"; printf '\0'; done \
  | sekien \
  | awk -v list="${files[*]}" '
      BEGIN { RS="\0"; n = split(list, a, " ") }
      { svg = a[NR]; sub(/\.mmd$/, ".svg", svg); print > svg }'
```

`-0` `-z` `RS="\0"` 等の Unix tool 機構が直接使えるため、delimiter 変換が不要。

## 内部実装

### 全体フロー

```
main():
  1. CLI 引数を parse (font, theme, look, file 等)
  2. file 引数 or stdin から Box<dyn Read + Send> を作る (open_reader)
  3. render::run_stream(reader, config) を呼ぶ
     - Linux なら ensure_display() で Xvfb 起動 + DISPLAY 設定
     - reader thread を spawn:
         入力を 8KiB ずつ read、\0 で分割して LoopEvent::Block / InputEnd /
         InputError を EventLoopProxy で送る
     - tao イベントループ起動、WebView 生成、mermaid.js load
     - StreamState (queue + awaiting + webview_ready) で event を捌く:
         LoopEvent::Block       → queue へ。webview_ready かつ idle なら dispatch
         LoopEvent::Ipc("ready") → webview_ready = true、queue の先頭を dispatch
         LoopEvent::Ipc("svg")   → stdout に SVG を flush (separator は SVG 間)、次の block を dispatch
         LoopEvent::Ipc("error") → stderr に "Error: mermaid block N: <msg>" を 1 行、次の block を dispatch
         LoopEvent::InputEnd    → end_received=true、queue 消化後 process::exit(0)
         LoopEvent::InputError  → process::exit(1)
```

### streaming の要点

- **reader と event loop の分離**: stdin の blocking read は別 thread に置き、
  EventLoopProxy 経由で event を送る。tao の event loop は main thread に
  pinning されており、blocking read を直接書けないため
- **queue 経由の dispatch**: input は webview の render より速く到着しうるので、
  StreamState 内に `VecDeque<(id, content)>` を持つ。awaiting (= 1 件だけ
  render 中) と webview_ready の両方が揃ったときに queue の先頭を pop
- **block id は 1-origin**: `next_index` から消費。webview の `mermaid.render`
  には `d{id}` の DOM id として渡す。stderr に出す `mermaid block N` の N
  もこの 1-origin
- **stdout の即時 flush**: `io::stdout().lock()` + `flush()` で SVG ごとに
  pipe へ push する。これにより `sekien | head -1` のような pipeline でも
  最初の SVG が即座に下流に届く
- **per-block 失敗は exit code に出さない**: 失敗時も `awaiting = None` に
  戻して queue 消化を続ける。`exit 1` するのは reader I/O 失敗、malformed
  IPC、stdout write 失敗、display 初期化失敗等の sekien 自身の障害のみ

### イベントループ (tao)

イベントループは tao を使う。winit より Linux サポートが充実しているため。

ウィンドウサイズと配置は OS ごとに事情が異なる:

- **macOS / Windows**: 実画面にウィンドウが描画される。ユーザーから見えないよう
  に画面外 (`-10000, -10000`) に配置する。サイズは 1x1 で問題ない
- **Linux**: Xvfb の仮想 framebuffer 内で完結する (実画面が存在しない) ため、
  画面外配置は不要。一方、GTK は 1x1 のウィンドウサイズで GDK のアサーション
  エラーを起こすため、`#[cfg(target_os = "linux")]` で 100x100 に拡大する

`event_loop.run()` は `-> !` で呼び出し元に戻らないため、`run_stream` 内部で
`std::process::exit` を呼んで終了する。これは sekien バイナリの 1 回の起動内で
完結する設計なので問題にならない (sekien-api 等の caller は別プロセスから
sekien を spawn するので影響を受けない)。

### Linux display 解決

`run_stream` の冒頭、GTK 初期化より前に display backend を解決する
(`linux_display::ensure_display`)。

#### GDK backend は常に X11 を強制

`GDK_BACKEND=x11` を必ずセットする。後段で起動する Xvfb は X server なので、
GDK にも X11 backend を選ばせる必要があるため。これを指定しないと Wayland
セッションでは GDK が `$WAYLAND_DISPLAY` を優先し、`DISPLAY` で指した Xvfb を
無視して Wayland コンポジタに接続してしまう。

#### Display の確保

`$DISPLAY` の有無に関わらず、常に内部で Xvfb を spawn して `$DISPLAY` を上書きする。

Xvfb (in-memory framebuffer) を使う理由は、**実画面に描画させないため**。X11
セッションや Wayland セッション (Xwayland 経由) で実画面に描画すると、
レンダリング中の数百ミリ秒だけウィンドウが画面に flash してしまう。Xvfb は
そもそも画面を持たないため flash しない。Linux のセッション種別に関わらず
一律 Xvfb に統一することで、環境差を吸収して常に invisible なレンダリングを
保証する。

Xvfb は `-displayfd 1 -terminate -screen 0 100x100x24 -nolisten tcp` で起動し、
Xvfb 自身が空き display 番号を選んで stdout に書き出すのを待つ
(`-displayfd` は X server が client 受付可能になったタイミングで発火する)。
socket file の存在だけでは server 完全 ready の前に GTK が接続を試みて失敗する
ため、このシグナルを使う。

`-terminate` により sekien 終了時に Xvfb も自動的に停止するため、明示的な
プロセス管理は不要。

複数 block を一括処理する場合 (sekien-api / sekien-pandoc 経由) でも、
**1 回の sekien 起動につき Xvfb は 1 つ**。N blocks 処理しても Xvfb 起動コストは
1 回分のみ。これが `\0` 区切り protocol の主要な性能上のメリット。

#### GTK4 headless への将来的な移行

GTK 4.10+ で `GDK_BACKEND=headless` が利用可能になり、display server 自体が
不要になる。ただし wry 0.55 は GTK3 / webkit2gtk-4.x にハードコードされており、
GTK4 を選ぶ feature flag が無い。wry の GTK4 対応後、Xvfb 経路を headless に
差し替え可能。

## 設定の受け渡し

### 経路 A: sekien CLI を直接利用 (人間が shell から)

```
shell の env (SEKIEN_FONT 等) + CLI 引数 (--font 等)
        ↓ sekien main が両方を読む (CLI 引数優先、未指定時に env)
mermaid.initialize() の引数
```

sekien は CLI フラグと環境変数の両方を受け付ける。CLI フラグが優先、未指定時に
env を見る:

```rust
let config = RenderConfig {
    font_family: cli_opts.font_family.or_else(|| env::var("SEKIEN_FONT").ok()),
    theme:       cli_opts.theme      .or_else(|| env::var("SEKIEN_THEME").ok()),
    look:        cli_opts.look       .or_else(|| env::var("SEKIEN_LOOK").ok()),
};
```

sekien-api / sekien-pandoc 経由の設定の流れは
[sekien-api/DESIGN.md](api/rust/DESIGN.md) と
[sekien-pandoc/DESIGN.md](../2026-05-20-sekien-pandoc/DESIGN.md) を参照。

## 性能特性

### sekien 単独利用 (CLI、1 block)

1 起動の wall time は次のコストの合計:

- **display 初期化**: Linux は Xvfb 起動 + GTK 初期化、macOS / Windows は OS
  ネイティブ WebView の初期化のみ
- **mermaid.js load**: HTML テンプレートに同梱した `mermaid.min.js` の評価
- **render**: 図の複雑さに依存 (`bench/diagrams/` の図で数十〜数百 ms)

直近の実測値は [README.md - mmdc との比較](README.md#mmdc-との比較) を参照。
内訳の比率は OS / arch / 図の複雑さで変動するが、**起動コストが render コストに
対して支配的** な傾向は変わらない。これが `\0` 区切り protocol で複数 block を
1 起動に束ねる設計の根拠 (sekien-api / sekien-pandoc 経由のケース)。

複数 block 一括処理 (sekien-api 経由) では起動コストが 1 回分に償却され、
render コストだけが N 倍になる。詳細は
[sekien-api/DESIGN.md](api/rust/DESIGN.md) 参照。

## 公開方針

```bash
cargo install sekien
```

`sekien` バイナリは sekien-api / sekien-pandoc の全用途で必須 (両者が内部で
`sekien` を spawn するため、PATH 上に `sekien` が居る必要がある)。

## なぜこの設計か

### なぜ sekien は単一 mode なのか

モード切替フラグを持たず、stdin を `\0` 区切りで読む streaming プロセスに
した理由:
- sekien の "顔" が 1 つに保たれる (`--help` も簡潔)
- ユーザーが学ぶことが少ない
- 単発 CLI 利用も bulk 利用も対話利用も同じ interface
- "1 input → 1 output (もしくは 1 error)" が per-block で一貫する

`\0` 区切り + streaming protocol によって、interface 数を増やさずに bulk
処理と対話処理を可能にしている。

### なぜ per-block 失敗で exit せず継続するのか (continue-on-error)

旧設計は "1 block でも失敗したら全 SVG を捨てて exit 1"。これを streaming +
continue-on-error に変えた理由:

- **bulk 入力では一部失敗が日常**: pandoc filter で 20 個の Mermaid を処理する
  時、1 個 typo があるだけで残り 19 個も捨てるのは過剰
- **対話モードとの整合**: 対話的に使うとき、1 件 typo したら sekien が die する
  と再起動コスト (Xvfb / WebView 初期化) を毎回払うことになる。streaming で
  生き残ってくれれば修正版をすぐ流し直せる
- **failure 情報の粒度**: 集約 exit code 1 は "何が失敗したか" の情報を持たない。
  per-block で stderr に 1 行ずつ出せば、caller は具体的に block N が壊れた
  ことを知れる
- **shell pipeline での扱いやすさ**: `sekien | extract-svgs.sh` のような構成で
  上流が部分的に失敗しても下流に成功分だけが流れる。`grep` 等の Unix ツール
  と同じ "stream を加工し続ける" モデル

### なぜ Linux で Xvfb を常時起動するのか

詳細は前述の "Linux display 解決" 節を参照。要約:
- 実画面に描画させると数百ミリ秒のウィンドウ flash が発生する (X11 / Xwayland いずれも)
- Xvfb は in-memory framebuffer で画面が存在しないため flash しない
- Linux のセッション種別に依存せず invisible なレンダリングを保証できる
