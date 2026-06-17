# Changelog

## [0.4.0] - 2026-06-17

### Changed

- **Breaking**: `on_result` callback signature changed from
  `impl FnMut(usize, RenderOutcome)` to `impl FnMut(RenderOutcome)`.
  The `id` argument (1-origin position) has been removed. Results are
  delivered in input order, so callers that need an index can maintain
  their own counter.

- **Breaking**: `Error::Display`, `Error::Window`, `Error::WebView`, and
  `Error::Ipc` have been replaced by a single `Error::Internal(String)`.
  All four were fatal and indistinguishable from the caller's perspective.
  `Error::Config` remains as the only user-actionable variant.

## [0.3.2] - 2026-06-16

### Changed

- `render_stream`'s `on_result` callback no longer requires `Send + 'static`.
  It is called exclusively on the main thread inside the tao event loop, so
  the bounds were unnecessarily restrictive. This is a backward-compatible
  change: existing callers are unaffected, and new callers can now pass
  closures that capture non-`Send` or borrowed state (e.g. a local `Vec`).

## [0.3.1] - 2026-06-15

### Fixed

- SVG output is now serialized via `XMLSerializer` instead of `innerHTML`.
  This produces well-formed standalone XML: namespace declarations such as
  `xmlns:xlink` are present, making the output compatible with strict XML
  parsers (e.g. usvg, used by Typst).

## [0.3.0] - 2026-06-14

### Changed

- **Breaking**: `render_stream`'s second parameter is now `config_json: Option<&str>`
  instead of `&RenderConfig`. The `RenderConfig` struct (with `font_family`, `theme`,
  `look`, and `config_json` fields) has been removed.

  Before:

  ```rust
  render_stream(diagrams, &RenderConfig { theme: Some("dark".into()), ..Default::default() }, on_result)
  ```

  After:

  ```rust
  render_stream(diagrams, Some(r#"{"theme":"dark"}"#), on_result)
  ```

  The CLI's `--font`/`--theme`/`--look`/`--config` flags are unaffected.

### Added

- `Error::Config`: returned immediately if `config_json` doesn't parse as a JSON
  object, instead of hanging while the WebView waits to become ready.

## [0.2.0] - 2026-06-13

### Added

- sekien is now a `[lib]` + `[[bin]]` crate. `render_stream` is the public
  library entry point, callable directly from Rust without spawning a child
  process.

  ```rust
  use sekien::{render_stream, RenderOutcome};

  render_stream(diagrams, &RenderConfig::default(), |id, outcome| {
      match outcome {
          RenderOutcome::Svg(svg) => { /* ... */ }
          RenderOutcome::Error(e) => { /* ... */ }
      }
  })?;
  ```

### Changed

- **Breaking**: The crate now exports `render_stream`, `RenderConfig`,
  `RenderOutcome`, `Error`, and `MERMAID_VERSION`. Previous versions had no
  library API.

## [0.1.1] - 2026-05-31

### Fixed

- Build reproducibility: normalize line endings before SHA-256 hashing in
  `build.rs` to prevent checksum mismatches on Windows checkouts.

## [0.1.0] - 2026-05-31

### Added

- Initial release. Mermaid → SVG CLI using an OS-native WebView (WKWebView on
  macOS, WebView2 on Windows, WebKitGTK + Xvfb on Linux).
- Streaming `\0`-delimited protocol: reads stdin until EOF, converts each
  NUL-separated block to SVG, writes results to stdout, errors to stderr.
- `--font`, `--theme`, `--look`, `--config` flags.
- `--meta` flag: prepends `<!-- {"id": N} -->` before each output block.
- Bundled `mermaid.min.js`; SHA-256 integrity check at compile time.
- Linux: internal Xvfb management (no external `xvfb-run` required).
- Prebuilt binaries for macOS (Apple Silicon, Intel) and Linux (x86-64).
