use anyhow::{bail, Context, Result};
use sekien::{render_stream, RenderConfig, RenderOutcome, MERMAID_VERSION};
use std::env;
use std::fs;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;

fn usage() -> String {
    format!(
        "Sekien draws Mermaids natively. (mermaid.js {MERMAID_VERSION})

Usage:
  sekien [options] [file.mmd]         Mermaid → SVG (stdout)
  cat diagram.mmd | sekien            Mermaid → SVG (stdout)

Sekien is a streaming process, like cat. It reads stdin (or a file) until EOF,
splitting on `\\0` (NUL byte), and converts each Mermaid block to SVG, streaming
results to stdout. A single trailing `\\0` on stdin is ignored.

Mermaid parse errors are written to stderr and processing continues (exit 0).
With --meta, each error is preceded by `<!-- {{\"id\": N}} -->`. Fatal failures
(display init, malformed IPC, etc.) exit 1.

In interactive mode, use Ctrl+@ to send a NUL byte and Ctrl+D to exit.

Options:
  --font <font>          Font family for diagram text (default: mermaid.js default)
  --theme <theme>        Mermaid theme (default | base | dark | forest | neutral |
                           neo | neo-dark | redux | redux-dark | null)
  --look <look>          Diagram look (classic | handDrawn | neo)
                         handDrawn is supported for flowchart/graph only.
  --config <file>        JSON config file for mermaid.initialize()
                         (see https://mermaid.js.org/config/schema-docs/config.html)
                         CLI flags (--theme etc.) take precedence over this file.
  --meta                 Prepend <!-- {{\"id\": N}} --> before each stdout (SVG) and
                         stderr (error) output
  --version, -v          Show version
  --help, -h             Show this help"
    )
}

#[derive(Default)]
struct Options {
    font_family: Option<String>,
    theme: Option<String>,
    look: Option<String>,
    show_meta: bool,
    config_file: Option<String>,
}

enum Command {
    Help,
    Version,
    Render { file: Option<String> },
}

fn parse_args(raw: Vec<String>) -> Result<(Options, Command)> {
    let mut options = Options::default();
    let mut rest: Vec<String> = Vec::new();
    let mut iter = raw.into_iter();

    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--help" | "-h" => return Ok((Options::default(), Command::Help)),
            "--version" | "-v" => return Ok((Options::default(), Command::Version)),
            "--font" => {
                options.font_family = Some(iter.next().context("--font requires a value")?);
            }
            "--theme" => {
                options.theme = Some(iter.next().context("--theme requires a value")?);
            }
            "--look" => {
                options.look = Some(iter.next().context("--look requires a value")?);
            }
            "--meta" => {
                options.show_meta = true;
            }
            "--config" => {
                options.config_file = Some(iter.next().context("--config requires a value")?);
            }
            _ if arg.starts_with('-') => {
                bail!("unknown option: {arg}");
            }
            _ => rest.push(arg),
        }
    }

    if rest.len() > 1 {
        bail!(
            "too many arguments (sekien takes at most one file)\n\
             hint:  for multiple files, use a shell loop:\n\
             \t for f in *.mmd; do sekien \"$f\" > \"${{f%.mmd}}.svg\"; done"
        );
    }

    Ok((
        options,
        Command::Render {
            file: rest.into_iter().next(),
        },
    ))
}

fn load_config_json(path: &str) -> Result<String> {
    let raw =
        fs::read_to_string(path).with_context(|| format!("cannot read config file '{path}'"))?;
    let value: serde_json::Value =
        serde_json::from_str(&raw).with_context(|| format!("invalid JSON in '{path}'"))?;
    if !value.is_object() {
        bail!("'{path}': expected a JSON object");
    }
    Ok(value.to_string())
}

/// Reads `reader`, splits on `\0`, and calls `on_block` for each block (in order).
///
/// On `\0`: emits the current buffer (even if empty) as a block.
/// On EOF: emits the buffer only if non-empty (drops a single trailing `\0`).
/// On I/O or UTF-8 error: returns `Err` immediately without reading further
/// (blocks already passed to `on_block` are unaffected).
fn read_blocks<R: Read>(reader: R, mut on_block: impl FnMut(String)) -> Result<(), String> {
    let mut reader = BufReader::new(reader);
    let mut buf: Vec<u8> = Vec::new();
    loop {
        match reader.read_until(0, &mut buf) {
            Ok(0) => return Ok(()),
            Ok(_) => {
                let is_nul = buf.last() == Some(&0);
                if is_nul {
                    buf.pop();
                }
                let should_emit = is_nul || !String::from_utf8_lossy(&buf).trim().is_empty();
                if should_emit {
                    match String::from_utf8(std::mem::take(&mut buf)) {
                        Ok(s) => on_block(s),
                        Err(e) => return Err(format!("input is not valid UTF-8: {e}")),
                    }
                }
                buf.clear();
            }
            Err(e) => return Err(format!("failed to read input: {e}")),
        }
    }
}

/// Writes one framed unit to `out`: an optional `\0` separator, an optional
/// `--meta` comment (`<!-- {"id": N} -->`), `content`, and a trailing newline.
fn write_framed(
    mut out: impl Write,
    id: usize,
    content: &str,
    show_meta: bool,
    write_separator: bool,
) -> io::Result<()> {
    if write_separator {
        out.write_all(&[0])?;
    }
    if show_meta {
        writeln!(out, "<!-- {{\"id\": {id}}} -->")?;
    }
    writeln!(out, "{content}")?;
    out.flush()
}

fn main() -> Result<()> {
    let raw: Vec<String> = env::args().skip(1).collect();
    let (options, command) = parse_args(raw)?;

    let config_json = options
        .config_file
        .as_deref()
        .map(load_config_json)
        .transpose()?;

    let config = RenderConfig {
        font_family: options.font_family,
        theme: options.theme,
        look: options.look,
        config_json,
    };

    match command {
        Command::Help => println!("{}", usage()),
        Command::Version => {
            println!(
                "sekien {} (mermaid.js {})",
                env!("CARGO_PKG_VERSION"),
                MERMAID_VERSION
            );
        }
        Command::Render { file } => {
            let reader: Box<dyn Read + Send> = match file.as_deref() {
                Some(p) => {
                    Box::new(fs::File::open(p).with_context(|| format!("cannot read '{p}'"))?)
                }
                None => Box::new(io::stdin()),
            };

            let (tx, rx) = mpsc::channel::<String>();
            let read_error: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
            {
                let read_error = read_error.clone();
                thread::spawn(move || {
                    if let Err(e) = read_blocks(reader, |s| {
                        let _ = tx.send(s);
                    }) {
                        *read_error.lock().unwrap() = Some(e);
                    }
                });
            }

            let show_meta = options.show_meta;
            let mut wrote_svg = false;
            let mut wrote_err = false;
            render_stream(rx, &config, move |id, outcome| match outcome {
                RenderOutcome::Svg(svg) => {
                    if let Err(e) = write_framed(io::stdout().lock(), id, &svg, show_meta, wrote_svg)
                    {
                        eprintln!("Error: failed to write SVG to stdout: {e}");
                        std::process::exit(1);
                    }
                    wrote_svg = true;
                }
                RenderOutcome::Error(err) => {
                    if let Err(e) =
                        write_framed(io::stderr().lock(), id, &err, show_meta, wrote_err)
                    {
                        eprintln!("Error: failed to write error to stderr: {e}");
                        std::process::exit(1);
                    }
                    wrote_err = true;
                }
            })?;

            let err = read_error.lock().unwrap().take();
            if let Some(e) = err {
                bail!(e);
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn help_long() {
        let (_, cmd) = parse_args(args(&["--help"])).unwrap();
        assert!(matches!(cmd, Command::Help));
    }

    #[test]
    fn help_short() {
        let (_, cmd) = parse_args(args(&["-h"])).unwrap();
        assert!(matches!(cmd, Command::Help));
    }

    #[test]
    fn version_long() {
        let (_, cmd) = parse_args(args(&["--version"])).unwrap();
        assert!(matches!(cmd, Command::Version));
    }

    #[test]
    fn version_short() {
        let (_, cmd) = parse_args(args(&["-v"])).unwrap();
        assert!(matches!(cmd, Command::Version));
    }

    #[test]
    fn render_no_args() {
        let (_, cmd) = parse_args(args(&[])).unwrap();
        assert!(matches!(cmd, Command::Render { file: None }));
    }

    #[test]
    fn render_with_file() {
        let (_, cmd) = parse_args(args(&["diagram.mmd"])).unwrap();
        assert!(
            matches!(cmd, Command::Render { ref file } if file.as_deref() == Some("diagram.mmd"))
        );
    }

    #[test]
    fn font_flag() {
        let (opts, _) = parse_args(args(&["--font", "Arial"])).unwrap();
        assert_eq!(opts.font_family, Some("Arial".to_string()));
    }

    #[test]
    fn font_flag_with_file() {
        let (opts, cmd) = parse_args(args(&["--font", "Arial", "diagram.mmd"])).unwrap();
        assert_eq!(opts.font_family, Some("Arial".to_string()));
        assert!(matches!(cmd, Command::Render { .. }));
    }

    #[test]
    fn font_flag_missing_value_is_error() {
        assert!(parse_args(args(&["--font"])).is_err());
    }

    #[test]
    fn theme_flag() {
        let (opts, _) = parse_args(args(&["--theme", "dark"])).unwrap();
        assert_eq!(opts.theme, Some("dark".to_string()));
    }

    #[test]
    fn theme_flag_missing_value_is_error() {
        assert!(parse_args(args(&["--theme"])).is_err());
    }

    #[test]
    fn look_flag() {
        let (opts, _) = parse_args(args(&["--look", "handDrawn"])).unwrap();
        assert_eq!(opts.look, Some("handDrawn".to_string()));
    }

    #[test]
    fn look_flag_missing_value_is_error() {
        assert!(parse_args(args(&["--look"])).is_err());
    }

    #[test]
    fn config_flag() {
        let (opts, _) = parse_args(args(&["--config", "config.json"])).unwrap();
        assert_eq!(opts.config_file, Some("config.json".to_string()));
    }

    #[test]
    fn config_flag_missing_value_is_error() {
        assert!(parse_args(args(&["--config"])).is_err());
    }

    #[test]
    fn too_many_files_is_error() {
        assert!(parse_args(args(&["a.mmd", "b.mmd"])).is_err());
    }

    #[test]
    fn unknown_flag_is_error() {
        assert!(parse_args(args(&["--unknown"])).is_err());
    }

    #[test]
    fn unknown_flag_with_file_is_error() {
        assert!(parse_args(args(&["--unknown", "diagram.mmd"])).is_err());
    }

    // ------ read_blocks ------

    fn run_reader(bytes: &[u8]) -> (Vec<String>, Result<(), String>) {
        let mut blocks: Vec<String> = Vec::new();
        let result = read_blocks(std::io::Cursor::new(bytes.to_vec()), |s| blocks.push(s));
        (blocks, result)
    }

    #[test]
    fn reader_empty_input() {
        let (blocks, result) = run_reader(b"");
        assert!(blocks.is_empty());
        assert_eq!(result, Ok(()));
    }

    #[test]
    fn reader_single_block() {
        let (blocks, result) = run_reader(b"graph LR\n  A --> B");
        assert_eq!(blocks, vec!["graph LR\n  A --> B"]);
        assert_eq!(result, Ok(()));
    }

    #[test]
    fn reader_two_blocks() {
        let (blocks, result) = run_reader(b"m1\0m2");
        assert_eq!(blocks, vec!["m1", "m2"]);
        assert_eq!(result, Ok(()));
    }

    #[test]
    fn reader_three_blocks() {
        let (blocks, _) = run_reader(b"a\0b\0c");
        assert_eq!(blocks, vec!["a", "b", "c"]);
    }

    #[test]
    fn reader_trailing_null_is_dropped() {
        let (blocks, _) = run_reader(b"m1\0m2\0");
        assert_eq!(blocks, vec!["m1", "m2"]);
    }

    #[test]
    fn reader_double_trailing_null_yields_one_empty() {
        let (blocks, _) = run_reader(b"m1\0m2\0\0");
        assert_eq!(blocks, vec!["m1", "m2", ""]);
    }

    #[test]
    fn reader_invalid_utf8_stops_reader() {
        let (_, result) = run_reader(&[0xff, 0xff]);
        assert!(result.is_err());
    }

    #[test]
    fn reader_invalid_utf8_after_separator() {
        let (blocks, result) = run_reader(&[b'a', 0, 0xff, 0xff]);
        assert_eq!(blocks, vec!["a"]);
        assert!(result.is_err());
    }
}
