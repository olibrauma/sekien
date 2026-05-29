# sekien Streaming Protocol Specification (v1.0)

この文書は、`sekien` バイナリの標準入出力（stdin, stdout, stderr）における通信規約を定義する。

## 1. 構文定義 (EBNF)

```ebnf
(* セパレータ: NUL バイト *)
separator     ::= "\0"

(* ストリーム定義: 0個以上のブロックの連続 *)
stdin         ::= [ mermaid_text { separator mermaid_text } ] [ separator ]
stdout        ::= [ stdout_unit { separator stdout_unit } ]
stderr        ::= [ stderr_unit { separator stderr_unit } ]

(* 構成要素: 各コンテンツは改行で終端される。ブロック間は separator で区切られる *)
stdout_unit   ::= [ json_meta ] svg_text "\n"
stderr_unit   ::= [ json_meta ] error_message "\n"

(* メタデータ定義: JSON オブジェクトを XML コメント形式でラップし、改行で終端 *)
json_meta     ::= "<!-- " json_object " -->\n"
json_object   ::= ? JSON object (RFC 8259) ?

(* コンテンツ定義: separator (\0) を含まない任意の UTF-8 文字列 *)
mermaid_text  ::= { character - separator }
svg_text      ::= { character - separator }
error_message ::= { character - separator }

(* 文字定義: 任意の有効な UTF-8 文字 *)
character     ::= ? any UTF-8 character ?
```

## 2. メタデータのデータ構造 (json_object)

`json_meta` に埋め込まれる `json_object` は、現在は以下の構造を持つ。

*   **id** (number): 入力順に基づく 1-origin のブロック番号。

例: `{"id": 1}`

将来的に、レンダリング日時や mermaid.js の詳細なバージョン情報などが追加される可能性がある。

## 3. 通信の性質

1.  **逐次処理（Streaming）**:
    `sekien` は `stdin` から `separator` を受け取るまで入力を読み込み、1 ブロック完成するごとに即座にレンダリングを開始する。結果が準備でき次第、`stdout` または `stderr` に出力する。
2.  **エラー継続（Continue-on-error）**:
    特定のブロックのレンダリングに失敗してもプロセスは終了せず、次のブロックの入力を待ち続ける。エラー情報は `stderr` に書き出される。
3.  **末尾セパレータの扱い**:
    `stdin` の末尾（EOF 直前）にある 1 つの `separator` は無視される。これにより、Unix 慣習的な `find -print0` 等の出力をそのまま処理できる。
4.  **終了ステータス**:
    *   `0`: `stdin` の EOF に達し、すべてのブロックの処理（成否不問）を完了した。
    *   `1`: プロセス自身の致命的な失敗（メモリ不足、ディスプレイ初期化失敗、I/O エラー等）。
