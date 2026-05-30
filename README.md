# sekien

Sekien draws Mermaids natively.

Mermaid → SVG on the command line, using the OS-native WebView instead of
bundling Chromium — lighter, faster, and far smaller than
[`mmdc`](https://github.com/mermaid-js/mermaid-cli).

## Install

```bash
cargo install sekien
```

On Linux, WebKitGTK development packages are required (Ubuntu example):

```bash
sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev
```

## Usage

```bash
# File argument → SVG on stdout
sekien diagram.mmd > diagram.svg

# stdin → SVG on stdout
cat diagram.mmd | sekien > diagram.svg

# Multiple diagrams in one invocation (\0-delimited)
printf 'graph LR\n  A --> B\0graph TD\n  X --> Y' | sekien > out.bin
```

Sekien is a streaming process, like cat. It reads stdin until EOF, flushing each
SVG to stdout as soon as it is ready. Mermaid parse errors are written to stderr
and processing continues (continue-on-error).

### Interactive mode

Launch directly from a terminal and enter diagrams one block at a time:

```text
$ sekien
graph LR
  A --> B
^@
<svg appears here>
^D
$
```

`Ctrl+@` sends a NUL byte (`\0`) to end a block; `Ctrl+D` sends EOF to exit.

## Options

| Flag | Description |
|---|---|
| `--font <name>` | Font family (CSS font-family syntax) |
| `--theme <name>` | mermaid.js theme |
| `--look <name>` | Diagram style |
| `--config <file>` | JSON config file for mermaid.initialize() |
| `--meta` | Prepend `<!-- {"id": N} -->` metadata before each output block |
| `--version`, `-v` | Print version |
| `--help`, `-h` | Print help |

Persist common options in a shell alias:

```bash
alias sekien='sekien --config ~/.config/sekien.json'
```

## Platforms

| OS | Requirement |
|---|---|
| macOS | Display required (WKWebView) |
| Windows | Display required (WebView2) |
| Linux | Xvfb (launched internally — no session or display needed) |

### macOS: Gatekeeper warning

Binaries downloaded from GitHub Releases are unsigned. If Gatekeeper blocks the
first launch, remove the quarantine attribute:

```bash
xattr -d com.apple.quarantine sekien
```

Alternatively, allow it via System Settings → Privacy & Security.

### Linux: Xvfb required

sekien launches its own Xvfb on every run and renders into that virtual display.
The desktop environment (X11 / Wayland / Xwayland) and `$DISPLAY` are ignored.

## vs mmdc

By using the OS-native WebView rather than bundling Chromium, sekien is
significantly lighter than `mmdc`.

- Figures are the median of the 3 diagrams in `util/bench/`. mmdc 11.14.0 / sekien 0.1.0 (mermaid.js 11.14.0)
- Max RSS includes all child processes (Xvfb/WebKit/Chromium) — see `util/bench/bench.sh`

### Binary size

| Platform | sekien | mmdc | Advantage |
|---|---|---|---|
| Mac | **~10 MB** | 330 MB | 97% smaller |
| Linux | **4.8 MB** | 401 MB | 99% smaller |

### Speed

| Platform | sekien | mmdc | Advantage |
|---|---|---|---|
| Mac | **~360 ms** | ~1.1 s | **67% faster** |
| Linux | **~1.1 s** | ~1.6 s | **31% faster** |

### Memory

| Platform | sekien | mmdc | Advantage |
|---|---|---|---|
| Mac | **~90 MB** | ~690 MB | **87% less** |
| Linux | **~430 MB** | ~630 MB | **32% less** |

## Internals

Protocol spec: [protocol.md](util/docs/protocol.md). Design rationale: [DESIGN.md](util/docs/DESIGN.md).

**wry** provides the OS-native WebView; **tao** handles the event loop and window management.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE) or https://www.apache.org/licenses/LICENSE-2.0)
- MIT License ([LICENSE-MIT](LICENSE-MIT) or https://opensource.org/licenses/MIT)

at your option.

### Bundled Assets

- `mermaid.js`: Licensed under the [MIT License](assets/mermaid.LICENSE). Copyright (c) 2014 - 2024 Knut Sveidqvist and contributors.

### Contribution

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall
be dual licensed as above, without any additional terms or conditions.
