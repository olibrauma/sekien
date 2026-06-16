# sekien — Design

## Core architecture

**sekien is a streaming process, like cat.** It reads Mermaid from stdin (or a
file), converts each `\0`-delimited block to SVG, and streams the results to
stdout. It stays alive until EOF, writing errors to stderr and continuing
(continue-on-error).

sekien has exactly **one operating mode**. Single-shot CLI use, batch use, and
interactive use all go through the same streaming protocol. Multiple diagrams
are processed by sending them `\0`-delimited on stdin.

## The `\0` delimiter

### Why `\0`

- **Never appears in Mermaid or SVG content**: both are text (printable ASCII +
  UTF-8), so NUL bytes cannot occur naturally.
- **Established Unix convention**: `find -print0`, `xargs -0`, `sort -z`,
  `grep -z`, `tr '\0' '\n'`, `read -d ''` — NUL as a delimiter for
  newline-bearing data is a well-known idiom. sekien slots directly into this
  ecosystem.
- **Easy to produce in POSIX shell**: `printf '%s\0' a b c` emits
  `\0`-separated values.
- **Works in any language**: Rust (`Read::read_until(0, ...)`), Python
  (`bytes.split(b'\x00')`), Node (`buffer.split('\0')`), etc.

### Separator, not terminator

`\0` on stdout is a **separator** (one between each pair of blocks), not a
**terminator** (one after each block). N blocks produce N−1 NUL bytes; there
is no trailing NUL.

The main reason is **single-file conversion convenience**:

```bash
sekien input.mmd > output.svg   # the most common use case
```

With terminator semantics, `output.svg` would end with `\0` and need
post-processing before being handed to other tools or SVG viewers. With
separator semantics, a single-block output is simply `<svg>\n` — a clean file.

Consumers of multi-block output (e.g. awk with `RS="\0"`) also handle the
absence of a trailing NUL naturally; a terminator would produce a spurious
empty final element.

The stdin rule that drops a single trailing `\0` is the symmetric counterpart:
it prevents Unix tools (`find -print0`, `printf '%s\0' ...`) from accidentally
producing an empty extra block.

### Unix pipeline examples

The most common use case — N `.mmd` files to N `.svg` files — is covered by a
shell loop:

```bash
for f in docs/*.mmd; do
  sekien "$f" > "${f%.mmd}.svg"
done
```

When Xvfb startup cost (~200 ms × N) matters, bundle all diagrams into one
sekien invocation:

```bash
files=(docs/*.mmd)
for f in "${files[@]}"; do cat "$f"; printf '\0'; done \
  | sekien \
  | awk -v list="${files[*]}" '
      BEGIN { RS="\0"; n = split(list, a, " ") }
      { svg = a[NR]; sub(/\.mmd$/, ".svg", svg); print > svg }'
```

The `-0` / `-z` / `RS="\0"` flags in standard Unix tools make this pipeline
work without any delimiter conversion.

## Internals

### Library / CLI split

sekien is a `[lib]` + `[[bin]]` crate: `src/lib.rs` re-exports a small public
API from `src/render.rs`, whose sole entry point is

```rust
fn render_stream(
    diagrams: impl IntoIterator<Item = String> + Send + 'static,
    config_json: Option<&str>,
    on_result: impl FnMut(usize, RenderOutcome) + Send + 'static,
) -> Result<()>;
```

`config_json` is a JSON object string spread into mermaid.initialize() (e.g.
`{"theme":"dark","fontFamily":"Arial"}`), or `None` for defaults.

It renders each `String` in `diagrams` to SVG, one at a time, and calls
`on_result(id, outcome)` for each — `id` is the 1-origin position of the
diagram in `diagrams`, and results are delivered in that same order.
`Err` is returned only for sekien's own fatal failures (display init, WebView
creation, malformed IPC); per-diagram Mermaid errors are reported via
`RenderOutcome::Error`, not `Err`.

`src/main.rs` (the CLI) is an ordinary consumer of this API: it reads
stdin/file and splits on `\0` (`read_blocks`), feeds the resulting blocks to
`render_stream` over an `mpsc::channel`, and writes `on_result`'s output back
to stdout/stderr with `\0`/`--meta` framing (`write_framed`). The `\0`
protocol described in this document is entirely a CLI concern — `render_stream`
has no knowledge of it, which lets other Rust programs (e.g. sekien-pandoc)
call it directly without going through the wire protocol at all.

### Pure core / impure shell

`render_stream` itself is split into:

- **`Collector`** (pure): a state machine that takes one input event — a new
  diagram, end-of-input, or an IPC message from the WebView — and returns the
  `Action`s (`Dispatch` / `Emit` / `Done` / `Fatal`) that should happen next.
  It touches neither the WebView, the event loop, nor any I/O, so it is
  unit-tested directly without a display.
- **`render_stream`** (impure): owns the WebView/event loop, feeds events into
  the `Collector`, and executes the `Action`s it returns (evaluate a render
  script, call `on_result`, or exit the loop).

This separation is what makes the renderer's sequencing guarantee — exactly
one render in flight, results delivered in input order — testable without
spinning up a WebView.

### Streaming design

- **Reader/event-loop separation**: in the CLI, blocking stdin reads run on a
  dedicated thread and are forwarded to `render_stream` via an
  `mpsc::channel`. Inside `render_stream`, a feeder thread relays the
  `diagrams` iterator to the tao event loop via `EventLoopProxy`, which is
  pinned to the main thread and cannot block.
- **Queue-based dispatch**: blocks arrive faster than the WebView can render,
  so `Collector` holds a `VecDeque<(id, content)>`. The next block is
  dispatched only when the pipeline is `Idle` (no render in flight).
- **1-origin block IDs**: assigned by `render_stream` via `enumerate()` over
  `diagrams`. The WebView receives each block as `renderMermaid(id, ...)` and
  the DOM element is named `d{id}`, preventing silent misattribution of
  results.
- **Per-block errors do not exit**: a Mermaid render failure is reported as
  `RenderOutcome::Error` via `on_result`; the pipeline returns to `Idle` and
  the queue continues draining. `render_stream`'s `Err` (and the CLI's exit 1)
  is reserved for sekien's own failures (reader I/O error, malformed IPC,
  display init, output write failure).

### Event loop (tao)

tao is preferred over winit for its stronger Linux support.

Window size and placement differ by OS:

- **macOS / Windows**: the window renders on the real screen, so it is placed
  off-screen at (−10000, −10000). Size 1×1 is sufficient.
- **Linux**: rendering happens entirely inside Xvfb (no real screen), so
  placement is irrelevant. GTK raises an assertion at 1×1, so the window is
  sized to 100×100 under `#[cfg(target_os = "linux")]`.

`render_stream` uses `event_loop.run_return()`, not `run()`: it returns
control to the caller once `Collector` signals `Done` or a fatal error occurs,
rather than calling `process::exit`. wry's `Drop` impls tear down the
WebView/window cleanly on return, so `render_stream` can be called from a
long-lived host process (e.g. sekien-pandoc) and the caller continues
normally afterwards. `std::process::exit` is only called from the CLI
(`main.rs`), for its own fatal errors and write failures.

`EventLoopBuilder::build()` panics if not called on the main thread (tao
imposes this on all platforms for cross-platform consistency, not just where
the OS requires it — see `EventLoopBuilderExtUnix::with_any_thread`, which
`render_stream` does not use). So `render_stream` must run on the host
process's main thread; concurrent work (e.g. the feeder thread that relays
`diagrams` into the event loop) must happen on other threads.

### Linux display resolution

`ensure_display()` (in `linux_display.rs`) is called at the start of
`render_stream`, before GTK is initialised.

#### Why X11 is forced

`GDK_BACKEND=x11` is always set. Xvfb is an X server, so GDK must use the X11
backend. Without this, on a Wayland session GDK would prefer `$WAYLAND_DISPLAY`
and ignore the `$DISPLAY` that points to Xvfb.

#### Why Xvfb is always used

`$DISPLAY` is always overwritten with a freshly spawned Xvfb, even if one
already exists.

Rendering via Xwayland or a real X server causes the window to flash on-screen
for the duration of the render (typically hundreds of milliseconds). Xvfb has
no screen, so it never flashes. Using Xvfb unconditionally makes the behaviour
independent of the desktop environment.

Xvfb is launched with `-displayfd 1 -terminate -screen 0 100x100x24
-nolisten tcp`. The `-displayfd` mechanism writes the chosen display number to
stdout once the X server is ready to accept clients — polling the socket file
alone is insufficient, as GTK may connect before the server is fully
initialised. (`-terminate` shuts Xvfb down automatically when sekien exits.)

For batch processing (multiple `\0`-delimited blocks), a single sekien
invocation means **one Xvfb** regardless of block count. The startup cost is
amortised over all blocks.

#### Future: GTK4 headless

GTK 4.10+ supports `GDK_BACKEND=headless`, which eliminates the need for any
display server. wry 0.55 is pinned to GTK3/webkit2gtk-4.x with no GTK4 feature
flag; once wry adds GTK4 support, the Xvfb path can be replaced with headless.

## Performance

Wall time for one invocation is the sum of:

- **Display init**: Linux — Xvfb launch + GTK init; macOS/Windows — OS-native
  WebView init only.
- **mermaid.js load**: evaluation of the bundled `mermaid.min.js`.
- **Render**: depends on diagram complexity (tens to hundreds of ms for the
  diagrams in `util/bench/diagrams/`).

For current measurements see [README.md — vs mmdc](../../README.md#vs-mmdc).
The ratio varies by OS, architecture, and diagram complexity, but **startup
cost dominates render cost**. This is the motivation for the `\0`-delimited
protocol: bundling multiple blocks into one invocation amortises the startup
overhead.

## Mermaid.js error output

### What Mermaid.js does on a syntax error

When `mermaid.render()` is called with invalid input, the flow inside the
bundled `mermaid.min.js` is:

1. `Sx.fromText(code)` invokes the diagram's parser.
   - **jison-based parsers** (flowchart/graph, sequenceDiagram, classDiagram,
     gantt, and most other diagram types) call `parseError(str, hash)` on
     failure.  The default implementation throws `new Error(str)` where `str` is
     the formatted diagnostic:
     ```
     Parse error on line 2:
     ...  A --> BADTOKEN
     ----------------^
     Expecting 'NEWLINE', 'EOF', got 'BADTOKEN'
     ```
     The `--------^` pointer comes from jison's `lexer.showPosition()`.
   - **Langium-based parsers** (architecture, packet, and a few newer types)
     collect `lexerErrors`/`parserErrors` and throw `MermaidParseError` whose
     message is:
     ```
     Parsing failed: Parse error on line N, column M: <token description>
     ```
     No `--------^` pointer is produced.

2. `cvt` (the internal `mermaidAPI.render`) catches the thrown error (`A`),
   renders a fallback "error" SVG diagram (the red-X icon), then
   **re-throws** the original error (`if (v) throw v`).  The fallback SVG is
   rendered but never returned to the caller.

3. The outer `mermaid.render` wrapper's rejection handler calls
   `Q.error("Error parsing", u)`.  `Q` is Mermaid's internal logger; with the
   default log level (`fatal`) this call is a **no-op**.

### How sekien relays the error

`render.html` wraps `mermaid.render()` in a `try/catch`:

```js
try {
  const { svg } = await mermaid.render('d' + id, code);
  // ... serialize and send svg via IPC
} catch (e) {
  window.ipc.postMessage(JSON.stringify({ type: 'error', id, error: e.message }));
}
```

`e.message` is the string described above.  Rust receives it as
`RenderOutcome::Error(msg)` and `main.rs` writes it to stderr via
`write_framed`.

For jison-based parsers this means the full `--------^` diagnostic reaches
stderr.  For Langium-based parsers the line/column information reaches stderr
but without the pointer line.

### Design decision

The current design is intentional: sekien relays `e.message` to stderr, which
carries the full diagnostic including `--------^` for jison parsers.

The jison runtime template that Mermaid embeds contains a recoverable-error
branch (`if (Ue.recoverable) this.trace(Fe)`), but searching `mermaid.min.js`
confirms that no Mermaid grammar sets `recoverable: true` anywhere — the branch
is dead code.  No special handling is needed for it.

## Known downstream dependency conflict

### Symptom

Projects that depend on sekien (e.g. gazu) cannot upgrade `toml` past `0.8.2`
within the same build graph.  `cargo update` is blocked from moving `toml` into
the `0.8.23+` range.

### Root cause

sekien → tao/wry → gtk v0.18 → glib v0.18 → glib-macros v0.18
  → **proc-macro-crate v2.0.2** → toml v0.8.2 → **toml_datetime =0.6.3** (exact pin)

`proc-macro-crate v2.0.2` uses an exact-version requirement (`=0.6.3`) for
`toml_datetime`.  Because `toml 0.8.23+` requires `toml_datetime ^0.6.11`,
Cargo cannot unify the two requirements within the same `0.6.x` series and
refuses to resolve the graph.

### Fix status

The fix already exists upstream:

- `proc-macro-crate v3.x` (released 2024-01-04, current 3.5.0) drops the exact
  pin entirely, using `toml_edit ^0.25` instead.
- `glib-macros v0.20+` (2024-07) and `v0.22+` (2026-04) already use
  `proc-macro-crate ^3.x`.

However, `tao v0.35.3` and `wry v0.55.1` (both current latest as of 2026-06)
are still pinned to `gtk = "0.18"` / `glib = "0.18"`.  Upgrading them requires
migrating from GTK3 to GTK4, a large breaking change.  The relevant Tauri
issues — [tao #1051](https://github.com/tauri-apps/tao/issues/1051) and
[wry #1474](https://github.com/tauri-apps/wry/issues/1474) — have been open
since January 2025 with no active development as of the time of writing.

### Workaround for downstream consumers

There is no safe workaround while sekien depends on tao/wry.

- `[patch.crates-io]` to replace `proc-macro-crate 2.x` with `3.x` will fail
  to compile because `glib-macros 0.18` uses the v2 API, which is incompatible
  with v3.
- Committing `Cargo.lock` and pinning `toml` to `0.8.2` prevents the conflict
  from surfacing but does not resolve it.

Resolution requires tao/wry to complete their GTK4 migration.

## Why this design

### Why single-mode streaming

Reasons for choosing a single streaming mode with no mode-switching flags:

- sekien presents one face to users (`--help` is concise).
- Less to learn.
- Single-shot, batch, and interactive use share the same interface.
- "1 input → 1 output (or 1 error)" is consistent at the per-block level.

The `\0`-delimited streaming protocol delivers batch and interactive processing
without adding interface complexity.

### Why continue-on-error

The old design exited 1 on the first failure. Reasons for switching to
streaming continue-on-error:

- **Partial failure is normal in batch use**: with 20 diagrams, one typo should
  not discard the other 19.
- **Interactive mode consistency**: a typo that kills sekien means paying the
  Xvfb/WebView startup cost again for the corrected version. Staying alive
  allows immediate retry.
- **Failure granularity**: an aggregate exit code 1 says nothing about which
  block failed. Per-block stderr output lets callers identify exactly which
  block N was broken.
- **Unix pipeline composability**: in `sekien | extract-svgs.sh`, only the
  successful SVGs reach downstream. This matches the "keep processing the
  stream" model of tools like `grep`.

### Why Xvfb is always used on Linux

- Rendering on the real screen (X11 or Xwayland) causes a window to flash
  for the duration of the render.
- Xvfb has no screen, so there is never any visible flash.
- Using Xvfb unconditionally guarantees invisible rendering regardless of the
  Linux session type.
