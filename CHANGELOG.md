# Changelog

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
