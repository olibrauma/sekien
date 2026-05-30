# sekien CLI 仕様 (v1.0)

この文書は `sekien` コマンドの引数・オプション・環境変数・終了コードを定義する。
入出力の通信規約は [protocol.md](protocol.md) を参照。

## 1. 構文 (Synopsis)

```
sekien [options] [<file>]
sekien --version | -v
sekien --help    | -h
```

## 2. 引数

### `<file>` (省略可)

読み込む Mermaid ファイルのパス。省略時は stdin を読む。

- ファイルと stdin は排他。`<file>` を指定した場合、stdin は参照されない。
- 2 つ以上指定した場合はエラーメッセージを stderr に出力して **exit 1**。

## 3. オプション

オプションは任意の順序で指定できる。`<file>` の前後どちらでも構わない。

### `--font <font>`

図中のテキストに使うフォントファミリー。CSS の `font-family` と同じ書式を受け付ける。

- デフォルト: mermaid.js の既定値

### `--theme <theme>`

mermaid.js のテーマ。以下の値を受け付ける:

`default` | `base` | `dark` | `forest` | `neutral` | `neo` | `neo-dark` | `redux` | `redux-dark` | `null`

- デフォルト: mermaid.js の既定値 (`default`)
- 値の妥当性検証は行わない。不正な値は mermaid.js 側でフォールバックされる。

### `--look <look>`

描画スタイル。以下の値を受け付ける:

`classic` | `handDrawn` | `neo`

- デフォルト: mermaid.js の既定値
- `handDrawn` は flowchart / graph 型のみ対応
- 値の妥当性検証は行わない。

### `--config <file>`

`mermaid.initialize()` に渡す設定を JSON ファイルで指定する。
ファイルはトップレベルが JSON オブジェクトでなければならない。

```json
{
  "flowchart": { "curve": "basis" },
  "sequence":  { "showSequenceNumbers": true },
  "themeVariables": { "primaryColor": "#ff0000" }
}
```

設定できる項目の一覧は
[mermaid.js 設定スキーマ](https://mermaid.js.org/config/schema-docs/config.html) 参照。

- CLI フラグ (`--theme` 等) は config ファイルの同名キーより**優先**される。
- `startOnLoad` / `htmlLabels` は sekien の動作に必須のため、config ファイルの値に
  関わらず常に上書きされる。

### `--block-id`

stdout (SVG) と stderr (エラー) の各出力ブロックの先頭に
`<!-- {"id": N} -->` を付与する。N は入力の 1-origin ブロック番号。

値をとらないフラグ。

### `--version`, `-v`

バージョン情報を stdout に出力して **exit 0**。出力形式:

```
sekien <semver> (mermaid.js <semver>)
```

他のオプションより先に現れた場合も後に現れた場合も、それより前に指定された
オプションは無視され、このコマンドが優先される。

### `--help`, `-h`

ヘルプテキストを stdout に出力して **exit 0**。
`--version` と同様、他のオプションを無視してこのコマンドが優先される。

## 4. 環境変数

環境変数によるデフォルト設定はサポートしない。
永続的なデフォルトが必要な場合はシェルエイリアスで代替する:

```bash
alias sekien='sekien --config ~/.config/sekien.json'
```

## 5. 終了コード

| コード | 条件 |
|---|---|
| `0` | stdin の EOF に達し、全ブロックの処理（成否不問）を完了した。または `--help` / `--version` を実行した。 |
| `1` | 不正な引数・オプション。sekien 自身の致命的失敗（display 初期化失敗、malformed IPC、I/O エラー等）。 |

個々の Mermaid ブロックの解析失敗は exit 1 にならない。エラーメッセージを
stderr に出力して次のブロックの処理を続ける（continue-on-error）。
詳細は [protocol.md §3](protocol.md#3-通信の性質) 参照。

## 6. 制約・非自明な挙動

- **ファイルは最大 1 つ**: 複数ファイルはエラー。複数ファイルを処理したい場合は
  シェルループか NUL 区切りで stdin に渡す。
  ```bash
  for f in *.mmd; do sekien "$f" > "${f%.mmd}.svg"; done
  printf '%s\0' *.mmd | xargs -0 cat | sekien
  ```
- **`--help` / `--version` は他のオプションを無視する**: パース途中で検出した
  時点で即座に返り、それ以前に解釈されたオプションは破棄される。
- **オプション値の未検証**: `--font` / `--theme` / `--look` の値は mermaid.js に
  そのまま渡される。不正値でも sekien は exit 0 し、mermaid.js がフォールバック
  または描画エラーとして処理する。
- **未知のフラグ**: `-` で始まる未知の引数はエラーとして **exit 1**。
