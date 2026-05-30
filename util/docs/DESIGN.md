# sekien — 設計方針

## アーキテクチャの中心思想

**sekien バイナリは cat のような streaming プロセス**。stdin (またはファイル) から
Mermaid を受け取り、`\0` を区切りに 1 block ずつ SVG を stdout へ流す。EOF まで
生存し、block 単位の失敗は stderr に流して継続する (continue-on-error)。

sekien バイナリの動作モードは **1 種類のみ**。単発 CLI 利用も bulk 利用も
対話利用も、すべて同じ streaming protocol。複数 Mermaid を一括処理したい場合は
**`\0` (NUL byte) で区切って** stdin に流す。

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

### streaming の要点

- **reader と event loop の分離**: stdin の blocking read は別 thread に置き、
  EventLoopProxy 経由で event を送る。tao の event loop は main thread に
  pinning されており、blocking read を直接書けないため
- **queue 経由の dispatch**: input は webview の render より速く到着しうるので、
  StreamState 内に `VecDeque<(id, content)>` を持つ。awaiting (= 1 件だけ
  render 中) と webview_ready の両方が揃ったときに queue の先頭を pop
- **block id は 1-origin**: `next_index` から消費。webview の `mermaid.render`
  には `d{id}` の DOM id として渡す。`--block-id` 時に出す N もこの 1-origin
- **stdout の即時 flush**: `io::stdout().lock()` + `flush()` で SVG ごとに
  pipe へ push する。これにより `sekien | head -1` のような pipeline でも
  最初の SVG が即座に下流に届く
- **per-block 失敗は exit code に出さない**: 失敗時も pipeline を Idle に
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
完結する設計なので問題にならない。

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

複数 block を一括処理する場合でも、**1 回の sekien 起動につき Xvfb は 1 つ**。
N blocks 処理しても Xvfb 起動コストは 1 回分のみ。これが `\0` 区切り protocol
の主要な性能上のメリット。

#### GTK4 headless への将来的な移行

GTK 4.10+ で `GDK_BACKEND=headless` が利用可能になり、display server 自体が
不要になる。ただし wry 0.55 は GTK3 / webkit2gtk-4.x にハードコードされており、
GTK4 を選ぶ feature flag が無い。wry の GTK4 対応後、Xvfb 経路を headless に
差し替え可能。

## 性能特性

1 起動の wall time は次のコストの合計:

- **display 初期化**: Linux は Xvfb 起動 + GTK 初期化、macOS / Windows は OS
  ネイティブ WebView の初期化のみ
- **mermaid.js load**: HTML テンプレートに同梱した `mermaid.min.js` の評価
- **render**: 図の複雑さに依存 (`util/bench/diagrams/` の図で数十〜数百 ms)

直近の実測値は [README.md - mmdc との比較](../../README.md#mmdc-との比較) を参照。
内訳の比率は OS / arch / 図の複雑さで変動するが、**起動コストが render コストに
対して支配的** な傾向は変わらない。これが `\0` 区切り protocol で複数 block を
1 起動に束ねる設計の根拠。

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

- **bulk 入力では一部失敗が日常**: 20 個の Mermaid を処理する時、1 個 typo が
  あるだけで残り 19 個も捨てるのは過剰
- **対話モードとの整合**: 対話的に使うとき、1 件 typo したら sekien が die する
  と再起動コスト (Xvfb / WebView 初期化) を毎回払うことになる。streaming で
  生き残ってくれれば修正版をすぐ流し直せる
- **failure 情報の粒度**: 集約 exit code 1 は "何が失敗したか" の情報を持たない。
  per-block で stderr に出せば、caller は具体的に block N が壊れたことを知れる
- **shell pipeline での扱いやすさ**: `sekien | extract-svgs.sh` のような構成で
  上流が部分的に失敗しても下流に成功分だけが流れる。`grep` 等の Unix ツール
  と同じ "stream を加工し続ける" モデル

### なぜ Linux で Xvfb を常時起動するのか

詳細は前述の "Linux display 解決" 節を参照。要約:
- 実画面に描画させると数百ミリ秒のウィンドウ flash が発生する (X11 / Xwayland いずれも)
- Xvfb は in-memory framebuffer で画面が存在しないため flash しない
- Linux のセッション種別に依存せず invisible なレンダリングを保証できる
