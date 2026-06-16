use anyhow::{bail, Context, Result};
use sekien::{render_stream, RenderOutcome, MERMAID_VERSION};
use serde_json::{json, Value};
use std::env;
use std::fs;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::sync::mpsc;
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

fn load_config_value(path: &str) -> Result<Value> {
    let raw =
        fs::read_to_string(path).with_context(|| format!("cannot read config file '{path}'"))?;
    let value: Value =
        serde_json::from_str(&raw).with_context(|| format!("invalid JSON in '{path}'"))?;
    if !value.is_object() {
        bail!("'{path}': expected a JSON object");
    }
    Ok(value)
}

/// Builds the `config_json` string passed to `render_stream`, from `--config
/// <file>` and the `--font`/`--theme`/`--look` shorthand flags, which take
/// precedence over (and are merged into) the config file's
/// `fontFamily`/`theme`/`look` keys.
fn build_config_json(options: &Options) -> Result<String> {
    let mut config = match options.config_file.as_deref() {
        Some(path) => load_config_value(path)?,
        None => json!({}),
    };
    let obj = config.as_object_mut().expect("validated as object");
    for (key, value) in [
        ("fontFamily", &options.font_family),
        ("theme", &options.theme),
        ("look", &options.look),
    ] {
        if let Some(value) = value {
            obj.insert(key.to_string(), Value::String(value.clone()));
        }
    }
    Ok(config.to_string())
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
                } else {
                    buf.clear();
                }
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

    let config_json = build_config_json(&options)?;

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
            let handle = thread::spawn(move || {
                read_blocks(reader, |s| {
                    let _ = tx.send(s);
                })
            });

            let show_meta = options.show_meta;
            let mut wrote_svg = false;
            let mut wrote_err = false;
            render_stream(rx, Some(&config_json), move |id, outcome| match outcome {
                RenderOutcome::Svg(svg) => {
                    if let Err(e) =
                        write_framed(io::stdout().lock(), id, &svg, show_meta, wrote_svg)
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

            if let Err(e) = handle.join().unwrap() {
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
    fn help_and_version_flags() {
        assert!(matches!(
            parse_args(args(&["--help"])).unwrap().1,
            Command::Help
        ));
        assert!(matches!(
            parse_args(args(&["-h"])).unwrap().1,
            Command::Help
        ));
        assert!(matches!(
            parse_args(args(&["--version"])).unwrap().1,
            Command::Version
        ));
        assert!(matches!(
            parse_args(args(&["-v"])).unwrap().1,
            Command::Version
        ));
    }

    #[test]
    fn render_no_args() {
        let (_, cmd) = parse_args(args(&[])).unwrap();
        assert!(matches!(cmd, Command::Render { file: None }));
    }

    #[test]
    fn flags_and_file_are_parsed() {
        let (opts, cmd) = parse_args(args(&[
            "--font",
            "Arial",
            "--theme",
            "dark",
            "--look",
            "handDrawn",
            "--config",
            "config.json",
            "--meta",
            "diagram.mmd",
        ]))
        .unwrap();
        assert_eq!(opts.font_family, Some("Arial".to_string()));
        assert_eq!(opts.theme, Some("dark".to_string()));
        assert_eq!(opts.look, Some("handDrawn".to_string()));
        assert_eq!(opts.config_file, Some("config.json".to_string()));
        assert!(opts.show_meta);
        assert!(
            matches!(cmd, Command::Render { ref file } if file.as_deref() == Some("diagram.mmd"))
        );
    }

    #[test]
    fn build_config_json_defaults_to_empty_object() {
        let opts = Options::default();
        assert_eq!(build_config_json(&opts).unwrap(), "{}");
    }

    #[test]
    fn build_config_json_includes_shorthand_flags() {
        let opts = Options {
            font_family: Some("Arial".to_string()),
            theme: Some("dark".to_string()),
            look: Some("handDrawn".to_string()),
            ..Default::default()
        };
        let json: Value = build_config_json(&opts).unwrap().parse().unwrap();
        assert_eq!(json["fontFamily"], "Arial");
        assert_eq!(json["theme"], "dark");
        assert_eq!(json["look"], "handDrawn");
    }

    #[test]
    fn build_config_json_flags_override_config_file() {
        let dir = std::env::temp_dir().join(format!(
            "sekien-test-config-{:?}",
            std::thread::current().id()
        ));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");
        fs::write(&path, r#"{"theme":"dark","flowchart":{"curve":"basis"}}"#).unwrap();

        let opts = Options {
            theme: Some("forest".to_string()),
            config_file: Some(path.to_str().unwrap().to_string()),
            ..Default::default()
        };
        let json: Value = build_config_json(&opts).unwrap().parse().unwrap();
        assert_eq!(json["theme"], "forest");
        assert_eq!(json["flowchart"]["curve"], "basis");

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn build_config_json_rejects_non_object_config_file() {
        let dir = std::env::temp_dir().join(format!(
            "sekien-test-config-bad-{:?}",
            std::thread::current().id()
        ));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.json");
        fs::write(&path, "[1, 2, 3]").unwrap();

        let opts = Options {
            config_file: Some(path.to_str().unwrap().to_string()),
            ..Default::default()
        };
        assert!(build_config_json(&opts).is_err());

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn flags_missing_value_is_error() {
        for flag in ["--font", "--theme", "--look", "--config"] {
            assert!(
                parse_args(args(&[flag])).is_err(),
                "{flag} requires a value"
            );
        }
    }

    #[test]
    fn invalid_arguments_are_errors() {
        assert!(parse_args(args(&["--unknown"])).is_err());
        assert!(parse_args(args(&["--unknown", "diagram.mmd"])).is_err());
        assert!(parse_args(args(&["a.mmd", "b.mmd"])).is_err());
    }

    // ------ read_blocks ------

    fn run_reader(bytes: &[u8]) -> (Vec<String>, Result<(), String>) {
        let mut blocks: Vec<String> = Vec::new();
        let result = read_blocks(std::io::Cursor::new(bytes.to_vec()), |s| blocks.push(s));
        (blocks, result)
    }

    #[test]
    fn read_blocks_splits_on_nul() {
        let cases: &[(&[u8], &[&str])] = &[
            (b"", &[]),
            (b"graph LR\n  A --> B", &["graph LR\n  A --> B"]),
            (b"m1\0m2", &["m1", "m2"]),
            (b"a\0b\0c", &["a", "b", "c"]),
            (b"m1\0m2\0", &["m1", "m2"]), // single trailing \0 is dropped
            (b"m1\0m2\0\0", &["m1", "m2", ""]),
            (b"   \n   ", &[]),            // whitespace-only input is dropped
            (b"a\0   \n   ", &["a"]),      // whitespace-only trailing content is dropped
        ];
        for (input, expected) in cases {
            let (blocks, result) = run_reader(input);
            assert_eq!(blocks, *expected, "input: {input:?}");
            assert_eq!(result, Ok(()));
        }
    }

    #[test]
    fn read_blocks_invalid_utf8_is_error() {
        let (blocks, result) = run_reader(&[0xff, 0xff]);
        assert!(blocks.is_empty());
        assert!(result.is_err());

        let (blocks, result) = run_reader(&[b'a', 0, 0xff, 0xff]);
        assert_eq!(blocks, vec!["a"]);
        assert!(result.is_err());
    }
}
