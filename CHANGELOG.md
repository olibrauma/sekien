# Changelog

## [Unreleased]

### Changed

- `util/bench/bench.sh` now measures time and memory in separate runs
  (sampling memory slowed the process down), alternates sekien and mmdc,
  swapping which goes first, and pauses between runs. README figures updated
  to sekien 0.5.1 vs mmdc 12.0.0 on Linux.

## [0.5.1] - 2026-09-30

### Changed

- Faster: rendering one diagram takes about 0.2–0.3 s less than in 0.5.0
  (e.g. one pie chart 1145 → 836 ms, one flowchart 1557 → 1336 ms on Linux).
  mermaid.js is now bundled as its ES module build and served to the
  WebView from memory, so code for a diagram type (e.g. the ELK layout
  engine) is only loaded when a diagram needs it. Output is unchanged.

### Fixed

- Windows: rendering failed with WebView2 error 0x80070057 ("The parameter
  is incorrect"), in every earlier version (#5). The page, with mermaid.js
  inlined, was passed as an HTML string, which WebView2 limits to 2 MB; it
  is now a small page loaded from a URL. The E2E tests now pass on the
  GitHub Actions Windows runner and are required in CI. Not yet tried on a
  desktop Windows machine.

## [0.5.0] - 2026-09-29

### Changed

- **Breaking**: Bundled mermaid.js updated from 11.17.2 to 12.0.0, matching
  mermaid-cli 12.0.0. The same input now renders differently by default:
  - The default layout is ELK instead of dagre (flowchart, state, class, ER,
    requirement and use case diagrams).
  - Flowchart, sequence, class, state, ER, requirement and use case diagrams
    default to the `redux-color` theme and the `neo` look.
  - Flowchart and state nodes have a new minimum width (`minNodeWidth: 120`),
    and flowchart labels wrap at 120px instead of 200px.

  To keep the previous output, pass this with `--config`:

  ```json
  {
    "theme": "default",
    "look": "classic",
    "flowchart": { "layout": "dagre", "minNodeWidth": 0, "wrappingWidth": 200 },
    "state": { "layout": "dagre", "minNodeWidth": 0, "wrappingWidth": 200 },
    "class": { "layout": "dagre" },
    "er": { "layout": "dagre" },
    "requirement": { "layout": "dagre" }
  }
  ```

  Set `layout` per diagram type as above, not at the top level: a top-level
  `layout: "dagre"` also forces mindmaps off their cose-bilkent layout.
  In our comparison against 0.4.2 with this config, flowchart, state and ER
  output was byte-identical, and class, requirement and sequence diagrams had
  the same size, positions and colours. Mindmap layouts shift by a few pixels
  regardless of config. (Some diagram types, e.g. gitGraph and architecture,
  are not byte-for-byte reproducible between runs even within one version.)

- **Breaking**: The `flowchart.defaultRenderer`, `class.defaultRenderer` and
  `state.defaultRenderer` config options are now ignored by mermaid.js. Use
  the top-level `layout` option instead.

- **Breaking**: mermaid.js 12 targets ES2024, which raises the minimum OS
  WebView: Safari 17.4+ on macOS and WebKitGTK 2.44+ on Linux. See
  Platforms in the README.

- The README now lists Windows as untested, with a known issue: WebView2
  creation fails on the GitHub Actions Windows runner, also for 0.4.2 (#5).
  Rendering on a real Windows machine has not been verified.

### Added

- `util/update-mermaid.sh <version>` updates the bundled mermaid.js from npm,
  verifying the tarball against the registry's integrity hash.

### Fixed

- Build-time mermaid version detection no longer parses the minified bundle,
  which has no stable version marker. The version is now recorded in
  `assets/mermaid.version` (from the npm `package.json`) and checked against
  the bundle at build time.
- `--theme` help and `util/docs/cli.md` now list the `redux-color` and
  `redux-dark-color` themes.

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
