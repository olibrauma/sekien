# Changelog

## [0.4.3] - 2026-09-29

### Updated

- wry updated from 0.55 to 0.57, and tao from 0.35 to 0.37. This includes a
  tao fix for `EventLoopProxy::send_event` sometimes not delivering an event
  until the next call; sekien uses it to deliver render results from the
  WebView.

### Changed

- `rust-version` raised from 1.76 to 1.88. wry 0.57 and tao 0.37 require
  1.85, and transitive dependencies (icu, time) require 1.88. Those
  transitive dependencies already required 1.88 in 0.4.2, so in practice
  this corrects the declared version rather than raising the real minimum.
- tao is now built without its `dbus` feature, which only read and watched
  the desktop's light/dark preference over the session bus (it never
  affected the output). sekien no longer connects to the session bus for
  that, and building it no longer needs the libdbus development headers.
- Documented a known issue: on Linux, `render_stream` sets `DISPLAY` and four
  other environment variables in the calling process, which is unsound
  while another thread reads the environment. Behaviour is unchanged; see
  the `render_stream` docs.

### Fixed

- `render_stream` now returns `Err` if its window is closed before all
  diagrams are rendered. Previously it returned `Ok(())`, silently dropping
  the remaining diagrams.
- `render_stream` now returns `Err` if the WebView reports readiness a second
  time (e.g. after a page reload). Previously it dispatched another render
  while one was still in flight.

## [0.4.2] - 2026-09-05

### Updated

- Bundled mermaid.js updated from 11.16.0 to 11.17.2, matching
  mermaid-cli 11.17.0.

### Fixed

- Build-time mermaid version detection: mermaid.js 11.17.x embeds a
  `version:"0.0.0"` build placeholder before the real version string, which
  made `MERMAID_VERSION` (and therefore `sekien --version`) report `0.0.0`.
  The placeholder is now skipped.

## [0.4.1] - 2026-07-09

### Updated

- Bundled mermaid.js updated from 11.15.0 to 11.16.0.

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

### Updated

- Bundled mermaid.js updated from 11.14.0 to 11.15.0.

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
