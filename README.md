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
sekien diagram.mmd > diagram.svg                     # file
printf 'graph LR\n  A --> B' | sekien > diagram.svg  # stdin
sekien                                               # interactive — type diagram, Ctrl+D to render
```

### Options

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

### Multiple diagrams

File, stdin, and interactive input all accept multiple diagrams separated by
`\0` (NUL byte). The WebView stays alive across all diagrams, paying startup
cost only once. Mermaid parse errors are written to stderr and processing
continues (continue-on-error). All options above apply.

```text
$ sekien
graph LR
  A --> B
^@
<svg for block 1>
graph TD
  X --> Y
^@
<svg for block 2>
^D
```

`Ctrl+@` sends `\0` between blocks; `Ctrl+D` exits. Output SVGs are
`\0`-separated. With `--meta`, each output is preceded by `<!-- {"id": N} -->`.

## Platforms

| OS | Requirement |
|---|---|
| macOS (Apple Silicon) | Display required (WKWebView) |
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
| Mac (Apple Silicon) | **~10 MB** | 330 MB | 97% smaller |
| Linux | **4.8 MB** | 401 MB | 99% smaller |

### Speed

| Platform | sekien | mmdc | Advantage |
|---|---|---|---|
| Mac (Apple Silicon) | **~360 ms** | ~1.1 s | **67% faster** |
| Linux | **~1.1 s** | ~1.6 s | **31% faster** |

### Memory

| Platform | sekien | mmdc | Advantage |
|---|---|---|---|
| Mac (Apple Silicon) | **~90 MB** | ~690 MB | **87% less** |
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
