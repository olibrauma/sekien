# Changelog

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
