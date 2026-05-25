# プロジェクトレビュー：sekien

## 総評
完成度の高い趣味プロジェクト。Unix 哲学を真面目に解釈した設計、型で不正状態を排した実装、根拠付きの設計判断記録が光る。気になるのは **CI 不在**、**`mermaid.min.js` の完全性検証不在**、**Lua filter の E2E test 不在** の 3 点。

---

## 1. 維持可能性

### 強い点
* **設計判断の "なぜ" がコードと `DESIGN.md` の両方に蓄積されている**
  * 例: `sekien-pandoc/DESIGN.md:130-178` の "採らなかった代替案" は、案 B/C を不採用にした理由（`graphicx` の拡張子依存、`os.tmpname() + ".svg"` の TOCTOU race）まで書かれている。将来 "simpler に戻そう" としたとき、不採用理由ごと判断履歴を読める。
* **型で不正状態を排除**
  * `render.rs:219-224` の `Pipeline { NotReady, Idle, Awaiting(usize) }` は "ready なのに awaiting でない" 等を構文レベルで disallow。`Awaiting(N)` が `N` を持つので、後段 of IPC で id mismatch を検出できる（`render.rs:312-317`）。
* **過去のバグがテスト名と doc に残る**
  * `api/rust/tests/e2e.rs:104-128` の `empty_block_in_input_does_not_shift_positions` は「過去の bug: `["m1", "", "m2"]` を渡すと末尾 `\0` を書かない実装が...」と回帰理由を明記。
* **`build.rs` で mermaid.js のバージョンを抽出して焼き込む（`build.rs:35-41`）**
  * `assets/mermaid.min.js` を差し替えれば `--version` 出力と doc が同期する。version 文字列の手動同期点を消した良設計。
* **pure 関数として `build_outcomes` を独立させて unit test 可能に（`api/rust/src/lib.rs:323-352`）**
  * コメントに「`render_blocks` から実バイナリ依存を切り離して unit test使える pure 関数として独立させている」と明記。

### 懸念点
* **CI が無い（`.github/workflows` 不在）**
  * テストは充実しているが PR で自動実行されない。`SEKIEN_TEST_BIN` を経由する e2e は手動でしか走らない。少なくとも `cargo test` / `cargo clippy` を GitHub Actions で動かせば release 後の回帰を防げる。
* **`assets/mermaid.min.js` の完全性検証が無い**
  * 手動 cp 運用のため、悪意あるコミット（または事故差し替え）を検出できない。SHA256 を `Cargo.toml` か `build.rs` に固定値として持って mismatch で fail するだけで防御になる。
* **`extract_version`（`build.rs:35`）のパターン破綻リスク**
  * 将来壊れる可能性を本人が認識済み（*"mermaid の bundle 構造が変わってこのパターンが複数出るようになったら..."*）。一発で気付くよう、`assets/mermaid.min.js` のバージョン期待値を `README` / `Cargo.toml` に書いておくと差し替え時の sanity check になる。
* **Lua filter の E2E test が無い**
  * `sekien-pandoc/tests/integration.rs:127` は `--print-lua-filter` の出力に `"function"` が含まれることしか検証しない。typst PDF を生成して画像埋め込みを実際に確認するテストがあると、Lua 側の regression が捕捉できる。
* **`api/rust/Cargo.lock` の commit**
  * lib crate の `Cargo.lock` は通常 commit しない（Rust 慣習）。bin の `sekien` / `sekien-pandoc` / `bench` 側は commit でよい。

---

## 2. セキュリティ

### 強い点
* **WebView 側の hardening が二重**
  * `assets/render.html:13-14` で `securityLevel: "strict"` + `htmlLabels: false`。
* **HTML 埋め込み経路の XSS 対策が明示的にテストされている（`render.rs:472-484`）**
  * `</script>` payload を入れても script タグから抜けないことを assertion で固定。`js_string_in_html`（`render.rs:87-92`）で `</>` を `\u003c/\u003e` へ。
* **HTML 埋め込みと `evaluate_script` で escape policy を区別**
  * `dispatch_render`（`render.rs:337-346`）のコメントに「`evaluate_script` は HTML parser を介さないので `</script>` の追加 escape は不要」と明記。同じ問題に同じ escape を二重適用していない（＝過剰防御による複雑化を避けている）。
* **Lua filter の tempfile が共有 `/tmp` の TOCTOU race を意識**
  * `/dev/urandom` 8 bytes ($2^{64}$) で予測不可能化（`assets/sekien.lua:29-42`）。`DESIGN.md (130-178)` で `os.tmpname() + ".svg"` を採らない理由として「attacker process が `/tmp/lua_*` を inotify watch していれば...」と具体的攻撃シナリオを書いている。多くの pandoc filter はこのレベルで考えていない。
* **env hygiene（環境変数の衛生管理）**
  * `sekien-api` が `SEKIEN_FONT/THEME/LOOK` を whitelist で `env_remove` する（`api/rust/src/lib.rs:73, 197-203`）。prefix 一括除去ではないので caller アプリ独自の `SEKIEN_BIN` 等は touched にならない。テストで両方検証（`build_command_does_not_touch_other_sekien_prefixed_env`）。
* **Xvfb は `-nolisten tcp`（`linux_display.rs:42`）**
  * X server を network listen させない。
* **unsafe ブロック無し**
  * `sekien` 本体、`api`、`pandoc` すべてにおいて徹底されている。

### 懸念点
* **`render_blocks` に timeout が無い**
  * `sekien` が WebView 初期化でハングした場合、caller は無限に待つ。pandoc filter として大量のドキュメントを CI で回すような用途で DoS 的になる可能性。`child.wait_timeout` か signal で kill する手段を追加した方が安全。
* **`wait_with_output()` は stdout/stderr を全部メモリに乗せる（`api/rust/src/lib.rs:287`）**
  * 攻撃用途 of ドキュメントで `sekien` が極端に大きい SVG を吐くケースは想定にないが、cap（容量制限）が無いことは記録しておくと良い。
* **Lua filter の `os.tmpname()` fallback (Windows)（`assets/sekien.lua:31-33, 37-38`）**
  * コメントには「Windows の `os.tmpname` は stub を作らないので問題ない」とあるが、これは Lua 実装依存。MinGW Lua 等で `mkstemp` が呼ばれる実装だと race 経路が復活する。Windows 用に明示的に分岐するか、boundary をテストで固定するとより堅い。
* **`assets/mermaid.min.js` の出所（サプライチェーン）検証が無い**
  * 前述の SHA256 固定で十分対処できる。
* **Mermaid 自体のサンドボックス**
  * `securityLevel: "strict"` でだいたい守れているが、mermaid 本体に 0-day があれば WebView 内で JS 実行が可能になる。これは mermaid の問題で `sekien` の責任範囲外だが、`htmlLabels: false` 等の追加防御は良い実践。

---

## 3. 開発者フレンドリ性

### 強い点
* **`DESIGN.md` の存在と質**
  * 両 repo とも `DESIGN.md` が「アーキテクチャ」＋「なぜこの設計か」＋「採らなかった代替案」の 3 段で書かれている。新規貢献者が「なぜ単一 mode なのか」「なぜ continue-on-error か」「なぜ Linux で Xvfb 強制か」を全部読める。
* **API error の variant 設計が ergonomic（人間工学的）**
  * `SekienApiError::Spawn { source }` を match して `source.kind() == ErrorKind::NotFound` で「インストールしてください」のメッセージを出す例が rustdoc に書かれている（`api/rust/src/lib.rs:32-43`）。利用者が手抜きせず error を扱える。
* **doctest が `rust,no_run` で書かれている（`api/rust/src/lib.rs:13-28`）**
  * type check は通すが実行はしない、bin 依存 lib 例として正しい使い方。
* **test name が contract（契約・仕様）そのもの**
  * `continue_on_error_three_blocks_middle_fails`, `partial_failure_preserves_position`, `reader_double_trailing_null_yields_one_empty` 等。test list を眺めるだけで仕様が読める。
* **`SEKIEN_TEST_BIN` で release build を test 経路に乗せられる（`tests/integration.rs:18-20`）**
  * `(skip)` の出力もあって「全 test 通って見えるが実は何も走っていなかった」を防いでいる。

### 懸念点
* **`CONTRIBUTING.md` と CI の不在**（前述）
* **`api/rust/` というディレクトリ命名**
  * `api/python/` 等のマルチランゲージ展開の意図があるなら良いが、無いなら `rust-api/` か `crates/sekien-api/` の方が一目で分かる。
* **`rust-toolchain.toml` 不在**
  * 再現性が contributor の rustc バージョンに依存。MSRV（最低サポートRustバージョン）を `README` で明示するか、`rust-toolchain.toml` で pin すると良い。
* **`bench/` に README が無い**
  * `bench/src/main.rs` の冒頭 doc comment で説明されているが、ディレクトリトップに README があると初見の動線が短くなる。

---

## 4. ユーザーフレンドリ性

### 強い点
* **`--help` が環境変数等価物も列挙（`main.rs:30-38`）**
  * `--font` と `SEKIEN_FONT` を並記。
* **`--version` 出力に mermaid.js のバージョンが入る**
  * 例: `sekien 0.1.0 (mermaid.js 11.14.0)`。サポート問い合わせ時に有用。
* **multi-file 引数でエラーにしつつ shell loop を提示（`main.rs:85-89`）**
  * `for f in *.mmd; do sekien "$f" > "${f%.mmd}.svg"; done` を error message に書く親切さ。
* **continue-on-error の思想**
  * 20 個中 1 つ typo でも残り 19 個の SVG は出る。`sekien-pandoc` 経由でも失敗した block は元のコードブロックを残して「出力先で何が起きたか分かる」graceful fallback（`pandoc.rs:66-72`）。
* **stdout streaming**
  * `sekien | head -1` で 1 個目の SVG が即時に届く。`io::stdout().lock() + flush()` を SVG ごとに実行している（`render.rs:348-355`）。
* **macOS の WebView focus 奪取問題への言及**
  * `README` に「なぜ起きるか」＋「回避策」＋「再発しないこと」まで明記されている（`README.md:66-70`）。

### 懸念点
* **`--output` / `-o` フラグが無い**
  * `> out.svg` のリダイレクトで十分とはいえ、リダイレクトを忘れてターミナルに SVG が大量に flush される事故を防ぐためにあると親切。
* **theme/look値が mermaid.js の用語そのまま**
  * `redux-dark` や `handDrawn` 等は説明が無いと選びにくい。mermaid docs へのリンクがあると親切。
* **Linux 初回実行で xvfb が無いときの error 経路**
  * `linux_display.rs:34` で context に `"install xvfb"` を書いているが、distro ごとの apt/dnf コマンドも入れると親切（README にはあるがエラー文には無いため）。

---

## 5. ハッカー的美学
これが本プロジェクトの真骨頂。

### 強い点（褒めどころ）
* **`\0` 区切りプロトコルの選択と説明**
  * `DESIGN.md:177-202` の "なぜ `\0` か" は `find -print0` / `xargs -0` / `sort -z` / `grep -z` / `tr '\0' '\n'` / `bash read -d ''` を全部列挙して「sekien はこの ecosystem にそのまま乗れる」と書く。単に区切り文字を選んだのではなく POSIX 慣習に同化させている。
* **"cat-like streaming process" という中心思想**
  * これがあるおかげで `\0` 区切り、continue-on-error、対話 mode が同じ設計から自然に出てくる。`DESIGN.md:381-410` の "なぜ" セクションがこの metaphor（メタファー）を裏付ける。
* **Pipeline state machine が宣言的**
  * `NotReady` → `Idle` → `Awaiting(N)` → `Idle` の遷移が enum で書かれていて、`try_dispatch_next`（`render.rs:290-301`）で gate される。手書きの flag（`bool ready`, `bool awaiting`）より格段に読みやすい。
* **Xvfb の `-displayfd 1`**
  * socket file 存在ではなく X server の readiness signal を待つ（`linux_display.rs:38-50, 67-72`）。`xvfb-run` 由来の technique を理解して適用している。
* **bench の RSS sampling**
  * `ps -ax` 1 回で全 process tree を取って reverse index 構築 → DFS で descendant 合計（`bench/src/main.rs:73-131`）。10ms 間隔で Xvfb の約 200ms 寿命を 20 sample で捕まえる、という設計根拠（line 28-30）もコメントに書かれている。
* **`build.rs` で mermaid.js から version 抽出**
  * bundle に焼き込んで `--version` で問い合わせ可能。API は const ではなく runtime query（`api/rust/src/lib.rs:159-174`）で「source of truth は同梱 `mermaid.min.js`」を保証。
* **HTML 埋め込みと `evaluate_script` の escape policy が非対称で正しく説明されている（`render.rs:337-341`）**
  * 簡単に "両方 escape しとけ" にせず、必要なときだけ escape する判断。
* **mermaid block の id を `d{id}` として DOM id に渡し、IPC でも同じ id を返させる（`render.html:18-20, render.rs:312-317`）**
  * silent な misattribution（データの誤紐付け）を確率的に潰す。
* **Lua filter の tempfile に対する攻撃想定**
  * `/dev/urandom` 8 bytes ($2^{64}$ unpredictability) ＋ `os.tmpname()` の stub を避ける理由付き。pandoc filter で TOCTOU を考えるのは稀。
* **3 crate 分離が原則端正**
  * `sekien-api` の `DESIGN.md:255-274`「なぜ sekien-api を経由するのか」「なぜ lib のみで bin を含まないか」「なぜ caller に sekien path を必須にしているか」がすべて答えられている。

### 控えめな批判
* **`std::process::exit(0)` を event loop closure から直接呼ぶ（`render.rs:391-401, 406-412`）**
  * `event_loop.run()` が `-> !` なので避けようがないが、`run_stream` のシグネチャ `Result<()>` は若干誤解を招く（`Result<!>` の意図）。コメントには書かれている。
* **100x100 Linux window size workaround（`render.rs:118`）**
  * GDK の 1x1 アサーション回避のマジックナンバー。理由付きだが、優美ではなく実用的な妥協。
* **`extract_version` の string search（`build.rs:35-41`）**
  * 十分働くが、`name:"mermaid"` 近接抽出の fallback まで書いてあると本人も認識する fragility（脆さ）。

---

## 推奨アクション（優先度順）

1. **CI 追加（`.github/workflows/ci.yml`）**
   * `cargo test`, `cargo clippy -D warnings`, `cargo fmt --check`。`SEKIEN_TEST_BIN` を Linux ジョブで設定して e2e も含める。テストの充実度に対して自動化が無いのが一番もったいない。
2. **`assets/mermaid.min.js` の SHA256 固定**
   * `build.rs` で checksum、または `Cargo.toml` の `[package.metadata]` に書いて手動 verify script。サプライチェーンで一番現実的なリスク。
3. **Lua filter の E2E test**
   * pandoc + typst が CI 環境に入るなら、actual PDF 生成まで通す。少なくとも `--lua-filter` を pandoc に渡して image 埋め込み AST が出ることまで確認。
4. **`render_blocks` に optional timeout**
   * `std::time::Duration` を引数にとって超過時 kill。lib として CI で使われるなら hang は致命的。
5. **`api/rust/Cargo.lock` の commit 取り消し（lib なので）**
6. **`rust-toolchain.toml` または README に MSRV 明記**
7. **bench に `bench/README.md` 追加（中身は `src/main.rs` 冒頭のコピーで十分）**

---
---

# 続報レビュー：別観点（市場性・応用可能性・エコシステム等）

## 1. 市場性
実需要は確実に存在するがニッチ。Mermaid の利用は GitHub native rendering（2022年）以降爆発しているが、"SSR で SVG を作りたい層" は限定的。多くのプラットフォーム（Notion, Joplin, GitLab）は browser rendering で済ませる。

### sekien が市場を切り取れる範囲
* **pandoc → PDF/LaTeX/typst パイプライン**
  * ここは SSR が必須なので `mmdc` / `sekien` の出番。`mermaid-filter` (Node) と `pandoc-mermaid` (Python) が既存競合。Rust 製で `cargo install` 一発の `sekien-pandoc` は配布容易性で勝てる。
* **CI で大量の Mermaid を一括変換する用途**
  * Docker layer に Chromium 200MB+ を入れる苦痛は実害として認知されている。sekien の "4.7 MB" は CI ユーザーに刺さる。
* **オフライン環境 / privacy 重視**
  * `mermaid.ink` (HTTP API) を使えない層。

### 導入・普及の現実
* **Switching Cost:** `mmdc` を CI に既に組み込んでいるチームは "70 倍小さい" だけでは乗り換えない（Docker layer をすでに払っているため）。ただし、新規導入時の選択肢としては極めて優位（特に "Chromium download に 5 分待ちたくない" 個人）。
* **Branding:** HN/Reddit の Show post 価値として、*"Mermaid CLI without bundling Chromium, 70x smaller"* は非常にキャッチー。1 週間で 200-500 stars 程度は現実的な見立て。

---

## 2. ライブラリ応用可能性

### `sekien-api` を library として使う候補
* **Static site generator の SSR plugin:** Zola / mdBook 用 mermaid 拡張。今は browser rendering 派が多数だが、PDF export を伴うドキュメントサイトでは SSR が要る。
* **Markdown editor の preview:** helix / zed / lapce の plugin として「保存時に SVG 化してインライン表示」。
* **Documentation generator の図埋め込み:** rustdoc 拡張、Sphinx 代替システム。
* **Chat bot の図描画:** Slack / Discord bot が `/diagram` で Mermaid を受け取って画像化。
* **メール/レポート自動生成:** 業務システムから Mermaid 図入り PDF を吐く。

### 応用を制限する 3 つの構造的要因
1. **Spawn-based protocol (FFI ではない):** `Command::new()` 前提なので「1 リクエストごとに sekien 起動コスト」がかかる（Linux で 約 360ms）。high-throughput RPC サーバの裏で使うには厳しい。
2. **Display 依存:** macOS / Windows では実画面前提。サーバ用途は Linux + Xvfb 一択。
3. **同期 API:** `render_blocks` は blocking。tokio 環境で使うなら `spawn_blocking` 経由が必要。

### 長期的応用余地（将来性）
* wry の GTK4 headless 対応後 → Xvfb 不要化、Linux サーバ用途のハードルが消える。
* WebKit2GTK6 や CEF への移行 → font / OS 依存差の縮小。
* Python / Node wrapper crate: pip / npm パッケージとして sekien binary を bundle 配布できれば、Rust 知識不要な層への普及経路ができる。`api/rust/` 命名はこの将来を示唆している。

---

## 3. エコシステム適合

| 環境 | 適合 | 理由 |
| :--- | :---: | :--- |
| **pandoc + typst/LaTeX PDF** | **◎** | sekien-pandoc が直接該当 |
| **pandoc + HTML/weasyprint** | **◎** | SVG をそのまま埋める |
| **MkDocs material** | **△** | mermaid を browser rendering する派、SSR は plugin 自作要 |
| **Hugo / Zola** | **△** | 同上 |
| **Docusaurus / VuePress** | **×** | browser rendering 派、SSR 要望は薄い |
| **Quarto** | **○** | mmdc 経由をすでに使うので置き換え候補 |
| **自前 Markdown → PDF パイプライン** | **◎** | sekien-api がドンピシャ |
| **Notion-clone (AppFlowy 等)** | **△** | browser rendering 派が多数だが export 用に sekien は使える |

* **最大の整合性:** 純朴な pandoc / SSR markdown 派、社内 CI のドキュメント自動生成、オフライン環境の文書ビルド。
* **Ecosystem ギャップ:** Quarto と統合できれば一気に普及する（Quarto は政策的に reproducible publishing を目指しているので sekien の "4.7 MB" 体質は刺さる）。また、`pandoc-crossref` / `pandoc-citeproc` と並ぶ filter エコシステムへの登録（community filter list）は最初の発見ルートとなる。

---

## 4. 配布戦略

現状の配布想定（README から）：
```bash
cargo install sekien
cargo install sekien-pandoc
cargo add sekien-api
```

### `cargo install` の壁
* `wry` + `tao` + `webkit2gtk-sys` のビルドが重い。Linux で 5-10 分、初回は依存（`libwebkit2gtk-4.x-dev` 等）の `apt install` が要る。
* "70 倍小さい" の体験が install 時にはビルド時間の長さで逆転する（mmdc は npm なので導入が早い）。

### 対策：Prebuilt Binary 配布が必須
* GitHub Releases で `cargo-dist` か `cross` を使った静的バイナリ配布（Rust なので cross-compile は容易）。
* macOS arm64/x86_64、Linux x86_64/aarch64、Windows x86_64 で 5-6 個の build を CI で回す。
* これがないと "70 倍小さい" のキャッチコピーが install 体験で裏切られる。

### 各種パッケージマネージャ・コンテナ対応
* **Package manager:** `brew install sekien`（homebrew-core 入りはハードル高いので tap から）、AUR (Arch)、scoop (Windows) ※これらは community に任せられる。
* **Docker image:** `tau-moneyforward/sekien:0.1.0` で Linux 用（xvfb 同梱）を提供すると CI での採用がさらに容易に。公式から出すか、docker pull した時点で「4.7 MB」のキャッチコピーが完成形になる。

---

## 5. ライセンス・帰属
主体は clean：`MIT OR Apache-2.0` dual license は Rust crate の標準。両 LICENSE ファイル同梱。

### 気になる点
* **`assets/mermaid.min.js` の帰属が薄い**
  * mermaid の MIT license なので同梱配布は完全に合法だが、README に *"Includes mermaid.js (MIT License), © Mermaid contributors"* のような帰属表示が無い。`assets/mermaid.LICENSE` を別途同梱するのが理想。
  * mermaid v11 の license は MIT。再配布 OK だが「license と copyright notice を含めること」が条件。現状、該当 license ファイルが repo に無い（確認できる範囲では）。
  * **これは 30 分でクリアできる作業:** `LICENSE-MERMAID` 追加 ＋ README に 1 行追記。
* **相対パスによる警告リスク**
  * `LICENSE-APACHE` / `LICENSE-MIT` は両 repo にあるが、`sekien-api` の README が `LICENSE-APACHE` を相対パスで `../../LICENSE-APACHE` と指している。crates.io publish 時に warning が出る可能性（lib crate に LICENSE ファイルが無いと判定されるため）。

---

## 6. 拡張余地

### 採るべき拡張（実用性高）
* `--width` / `--height` フラグ: mermaid.js の rendering box 制御。今は WebView 1x1/100x100 固定だが、ユーザーから要望は確実に出る。
* `--config <json>`: `mermaid.initialize()` の全 option を JSON で渡せると power user の出口になる（例: `theme.flowchart.curve = "basis"` のような細かい指定）。
* `--quiet`: stderr の `Error: mermaid block N` を抑制したい CI 用途。

### 採るべきでない拡張（Scope Creep / スコープ肥大化）
* **PNG 出力:** `resvg` か `rsvg-convert` でパイプすれば足りる。sekien に持たせると WebView と filesystem 副作用が増えて単一プロセス美学が崩れる。
* **watch mode:** streaming protocol で `inotifywait | sekien` で十分。
* **D2 / PlantUML 対応:** 名前が "drawer of Mermaids" に限定されている。scope を保つ判断が良い。

### 長期的なアーキテクチャの余地
* wry GTK4 headless 対応後の Xvfb 除去（README に記載あり）。最大の Linux 配布障壁が消える。
* mermaid v12 への追従: 自動化されていない（`build.rs` は version 抽出だけで API 互換性チェックは無い）。E2E test を CI に乗せるのが最良の防御。
* **WASM target:** wry/tao が WASM をサポートする日が来れば、ブラウザ上で sekien 相当の処理を library として動かせる可能性。

---

## 7. 長期持続性とリスク

### 一人趣味プロジェクトの構造的リスク
* maintainer 単一点障害（半年放置で不動になるリスク）。
* ただし scope が非常に小さい（4 crate, 1700 lines）ので容易に fork 可能。
* デペンデンシー数も少ない（`wry`, `tao`, `serde`, `anyhow`, `thiserror`）。bitrot（コードの陳腐化）速度は遅い。

### 外部依存リスク

| 依存 | リスク | 影響 |
| :--- | :--- | :--- |
| **wry 0.55** | API 破壊あり（年 1-2 回） | major bump 時に build 不可、追従必要 |
| **tao 0.35** | 同上 | 同上 |
| **mermaid.min.js 11.x** | mermaid v12 で render API 変更可能性 | `assets/render.html` の `mermaid.render()` 呼び出し書き換え |
| **Xvfb** | 公式 maintainer 不足（X.Org 全般） | 直近は問題ないが 5-10 年スパンでは疑問 |
| **cargo install UX** | Rust 化進行で改善継続 | 中立 |

---

## 8. ネーミングとブランディング

### "sekien" の解釈
* おそらく **鳥山石燕**（とりやま せきえん、江戸時代の妖怪絵師）から。"Mermaid Drawer" の対応として、文学的で非常にセンスの良い命名。
* 英語圏での発音問題: *"say-key-en?" "seh-key-en?"* — 検索流入や口コミ時に表記・発音がややブレる可能性。
* crates.io の name namespace は確保できそう（短い ＋ unique）。

### ブランディング強化案
* README の冒頭に 1 行 **"sekien (named after Toriyama Sekien, 18th century yokai painter)"** を入れるだけで物語（コンテキスト）が立つ。
* GitHub repo description に **"sekien — Mermaid → SVG, 70x smaller than mmdc"** のような catchy line を配置する。

### 比較で sekien が刺さるキャッチコピー
* *"Mermaid CLI without Chromium"*（主訴求）
* *"4.7 MB Mermaid renderer"*（数字訴求）
* *"Single-binary mermaid for CI"*（用途訴求）

---

## まとめ

| 観点 | 評価 |
| :--- | :--- |
| **市場性** | ニッチだが熱量あり、HN（Hacker News）で 500 stars 級を狙える |
| **応用可能性** | spawn-based の制約があるが SSG / CI / 文書化で多様 |
| **エコシステム適合** | pandoc ＋ 静的サイト ＋ CI で最大のシナジー |
| **配布戦略** | **prebuilt binary が無いと "70 倍小さい" が install 体験で裏切られる（最優先）** |
| **ライセンス** | clean だが mermaid 帰属を明示（LICENSE-MERMAID 同梱）すべき |
| **拡張余地** | `--config <json>` と `--width / --height` のみ採用、他は scope keep |
| **長期持続性** | `wry / tao` の API 破壊が最大リスク。scope が小さく fork 容易なのが保険 |
| **Naming** | 命名背景の 1 行がストーリー性と discoverability（発見しやすさ）に効く |

### 🚀 publish 直前にやるべき 3 つのアクション
（前回の「CI / SHA256 / Lua filter E2E」に加えて）

1. **prebuilt binary release を GitHub Actions で自動化（`cargo-dist` 等の導入）**
2. **mermaid の LICENSE を `LICENSE-MERMAID` で同梱 ＋ README に帰属表示を追加**
3. **README に 1 行ストーリーを追加**
   * *"Named after Toriyama Sekien, 18th century yokai painter"* (branding + discoverability)
