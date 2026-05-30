# sekien CLI Specification (v1.0)

This document defines the arguments, options, exit codes, and non-obvious
behaviour of the `sekien` command.
For the stdin/stdout/stderr wire format, see [protocol.md](protocol.md).

## 1. Synopsis

```
sekien [options] [<file>]
sekien --version | -v
sekien --help    | -h
```

## 2. Arguments

### `<file>` (optional)

Path to a Mermaid file to read. If omitted, stdin is read instead.

- `<file>` and stdin are mutually exclusive. When a file is given, stdin is ignored.
- Specifying two or more files prints an error to stderr and exits with **exit 1**.

## 3. Options

Options may appear in any order, before or after `<file>`.

### `--font <font>`

Font family for diagram text. Accepts CSS `font-family` syntax.

- Default: mermaid.js default

### `--theme <theme>`

mermaid.js theme. Accepted values:

`default` | `base` | `dark` | `forest` | `neutral` | `neo` | `neo-dark` | `redux` | `redux-dark` | `null`

- Default: mermaid.js default (`default`)
- Values are not validated; invalid values are silently ignored or produce a fallback in mermaid.js.

### `--look <look>`

Diagram style. Accepted values:

`classic` | `handDrawn` | `neo`

- Default: mermaid.js default
- `handDrawn` is supported for flowchart/graph diagrams only.
- Values are not validated.

### `--config <file>`

JSON config file for `mermaid.initialize()`.
The file must be a top-level JSON object.

```json
{
  "flowchart": { "curve": "basis" },
  "sequence":  { "showSequenceNumbers": true },
  "themeVariables": { "primaryColor": "#ff0000" }
}
```

For the full list of available keys, see the
[mermaid.js config schema](https://mermaid.js.org/config/schema-docs/config.html).

- CLI flags (`--theme`, etc.) override the same key in the config file.
- `startOnLoad` and `htmlLabels` are always overridden by sekien (required for correct operation).
- `securityLevel` defaults to `"strict"` but can be overridden via the config file.

### `--meta`

Prepends `<!-- {"id": N} -->` before each stdout (SVG) and stderr (error) output.
N is the 1-origin block number from the input.

Value-less flag. The metadata fields may be extended in future versions.

### `--version`, `-v`

Prints version information to stdout and exits **0**. Output format:

```
sekien <semver> (mermaid.js <semver>)
```

If `--version` or `--help` is encountered during argument parsing, all
previously accumulated options are discarded and only the corresponding
command runs.

### `--help`, `-h`

Prints help text to stdout and exits **0**.
Same early-exit behaviour as `--version`.

## 4. Environment variables

Default configuration via environment variables is not supported.
For persistent defaults, use a shell alias instead:

```bash
alias sekien='sekien --config ~/.config/sekien.json'
```

## 5. Exit codes

| Code | Condition |
|---|---|
| `0` | EOF reached; all blocks processed (regardless of per-block success or failure). Also: `--help` / `--version` executed. |
| `1` | Invalid argument or option. Fatal failure of sekien itself (display init, malformed IPC, I/O error, etc.). |

Per-block Mermaid parse failures do not produce exit 1. The error message is
written to stderr and processing continues (continue-on-error).
See [protocol.md §3](protocol.md#3-protocol-properties) for details.

## 6. Constraints and non-obvious behaviour

- **At most one file**: Multiple files are an error. To process multiple files,
  use a shell loop or NUL-delimited stdin.
  ```bash
  for f in *.mmd; do sekien "$f" > "${f%.mmd}.svg"; done
  printf '%s\0' *.mmd | xargs -0 cat | sekien
  ```
- **`--help` / `--version` discard preceding options**: When detected during
  parsing, the parser returns immediately and any options seen so far are dropped.
- **Option values are not validated**: Values for `--font`, `--theme`, and
  `--look` are passed through to mermaid.js as-is. An invalid value results in
  exit 0 with mermaid.js falling back or emitting a render error.
- **Unknown flags**: Any unrecognised argument starting with `-` is an error
  (**exit 1**).
