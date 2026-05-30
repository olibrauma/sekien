# sekien Streaming Protocol Specification (v1.0)

This document defines the stdin, stdout, and stderr communication contract
for the `sekien` binary.

## 1. Syntax (EBNF)

```ebnf
(* Separator: NUL byte *)
separator     ::= "\0"

(* Stream definitions: zero or more blocks in sequence *)
stdin         ::= [ mermaid_text { separator mermaid_text } ] [ separator ]
stdout        ::= [ stdout_unit { separator stdout_unit } ]
stderr        ::= [ stderr_unit { separator stderr_unit } ]

(* Units: each content is newline-terminated; units are separated by separator *)
stdout_unit   ::= [ json_meta ] svg_text "\n"
stderr_unit   ::= [ json_meta ] error_message "\n"

(* Metadata: a JSON object wrapped in an XML comment, newline-terminated *)
json_meta     ::= "<!-- " json_object " -->\n"
json_object   ::= ? JSON object (RFC 8259) ?

(* Content: any UTF-8 string that does not contain separator (\0) *)
mermaid_text  ::= { character - separator }
svg_text      ::= { character - separator }
error_message ::= { character - separator }

(* Character: any valid UTF-8 character *)
character     ::= ? any UTF-8 character ?
```

## 2. Metadata structure (json_object)

The `json_object` embedded in `json_meta` currently has the following structure.

- **id** (number): 1-origin block number based on input order.

Example: `{"id": 1}`

Additional fields (render timestamp, mermaid.js version details, etc.) may be
added in future versions.

## 3. Protocol properties

1. **Streaming**: sekien reads stdin until it receives a `separator`, then
   immediately begins rendering the completed block. Results are written to
   `stdout` or `stderr` as soon as they are ready.

2. **Continue-on-error**: If rendering a block fails, the process does not
   exit. The error is written to `stderr` and sekien continues waiting for
   the next block.

3. **Trailing separator**: A single `separator` immediately before EOF on
   `stdin` is ignored. This allows the output of tools like `find -print0`
   to be piped directly into sekien.

4. **Exit status**:
   - `0`: EOF reached; all blocks processed (regardless of per-block success or failure).
   - `1`: Fatal failure of sekien itself (out of memory, display init failure, I/O error, etc.).
