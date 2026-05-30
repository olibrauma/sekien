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

### Streaming design

- **Reader/event-loop separation**: blocking stdin reads run on a dedicated
  thread and send events via `EventLoopProxy`. The tao event loop is pinned to
  the main thread and cannot block.
- **Queue-based dispatch**: blocks arrive faster than the WebView can render,
  so `StreamState` holds a `VecDeque<(id, content)>`. The next block is
  dispatched only when the pipeline is `Idle` (no render in flight).
- **1-origin block IDs**: assigned from `next_index`. The WebView receives each
  block as `renderMermaid(id, ...)` and the DOM element is named `d{id}`,
  preventing silent misattribution of results.
- **Per-block stdout flush**: `io::stdout().lock()` + `flush()` on every SVG
  so that `sekien | head -1` receives the first SVG immediately.
- **Per-block errors do not exit**: on failure, the pipeline returns to `Idle`
  and the queue continues draining. Exit 1 is reserved for sekien's own
  failures (reader I/O error, malformed IPC, stdout write failure, display init).

### Event loop (tao)

tao is preferred over winit for its stronger Linux support.

Window size and placement differ by OS:

- **macOS / Windows**: the window renders on the real screen, so it is placed
  off-screen at (−10000, −10000). Size 1×1 is sufficient.
- **Linux**: rendering happens entirely inside Xvfb (no real screen), so
  placement is irrelevant. GTK raises an assertion at 1×1, so the window is
  sized to 100×100 under `#[cfg(target_os = "linux")]`.

`event_loop.run()` never returns (`-> !`), so `run_stream` calls
`std::process::exit` directly. This is safe because sekien is a single-shot
binary.

### Linux display resolution

`ensure_display()` (in `linux_display.rs`) is called at the start of
`run_stream`, before GTK is initialised.

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
