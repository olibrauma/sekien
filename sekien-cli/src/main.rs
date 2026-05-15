use anyhow::{Context, Result};
use sekien::{RenderConfig, MERMAID_VERSION};
use std::env;
use std::fs;
use std::io::{self, Read};

fn usage() -> String {
    format!(
        "sekien — Mermaid Drawer (mermaid.js {MERMAID_VERSION})

Usage:
  sekien [options] [file.mmd]         Mermaid → SVG (stdout)
  cat diagram.mmd | sekien            Mermaid → SVG (stdout)

Options:
  --font <font>          Font family for diagram text (default: mermaid.js default)
                         Also configurable via SEKIEN_FONT env var.
  --theme <theme>        Mermaid theme (default | base | dark | forest | neutral |
                           neo | neo-dark | redux | redux-dark | null)
                         Also configurable via SEKIEN_THEME env var.
  --look <look>          Diagram look (classic | handDrawn | neo)
                         handDrawn is supported for flowchart/graph only.
                         Also configurable via SEKIEN_LOOK env var.
  --version, -v          Show version
  --help, -h             Show this help

For Pandoc integration, use sekien-pandoc instead."
    )
}

#[derive(Default)]
struct Options {
    font_family: Option<String>,
    theme: Option<String>,
    look: Option<String>,
}

enum Command {
    Help,
    Version,
    Render { file: Option<String> },
}

fn parse_args(raw: Vec<String>) -> Result<(Options, Command), String> {
    let mut options = Options::default();
    let mut rest: Vec<String> = Vec::new();
    let mut iter = raw.into_iter();

    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--help" | "-h" => return Ok((Options::default(), Command::Help)),
            "--version" | "-v" => return Ok((Options::default(), Command::Version)),
            "--font" => {
                options.font_family = Some(iter.next().ok_or("--font requires a value")?);
            }
            "--theme" => {
                options.theme = Some(iter.next().ok_or("--theme requires a value")?);
            }
            "--look" => {
                options.look = Some(iter.next().ok_or("--look requires a value")?);
            }
            _ if arg.starts_with('-') => {
                return Err(format!("error: unknown option: {arg}"));
            }
            _ => rest.push(arg),
        }
    }

    let files: Vec<String> = rest;
    if files.len() > 1 {
        return Err(format!(
            "error: too many arguments (sekien takes at most one file)\n\
             hint:  for multiple files, use a shell loop:\n\
             \t for f in *.mmd; do sekien \"$f\" > \"${{f%.mmd}}.svg\"; done"
        ));
    }

    Ok((options, Command::Render { file: files.into_iter().next() }))
}

fn read_mermaid(file_path: Option<&str>) -> Result<String> {
    match file_path {
        Some(p) => fs::read_to_string(p).with_context(|| format!("cannot read '{p}'")),
        None => {
            let mut buf = String::new();
            io::stdin().read_to_string(&mut buf).context("failed to read stdin")?;
            Ok(buf)
        }
    }
}

fn main() -> Result<()> {

    let raw: Vec<String> = std::env::args().skip(1).collect();
    let (options, command) = parse_args(raw).unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(1);
    });

    let config = RenderConfig {
        font_family: options.font_family.or_else(|| env::var("SEKIEN_FONT").ok()),
        theme:       options.theme      .or_else(|| env::var("SEKIEN_THEME").ok()),
        look:        options.look       .or_else(|| env::var("SEKIEN_LOOK").ok()),
    };

    match command {
        Command::Help => println!("{}", usage()),
        Command::Version => {
            println!("sekien {} (mermaid.js {})", env!("CARGO_PKG_VERSION"), MERMAID_VERSION);
        }
        Command::Render { file } => {
            let code = read_mermaid(file.as_deref())?;
            sekien::render_all(vec![code], &config, |svgs| {
                if !svgs.is_empty() {
                    println!("{}", svgs[0]);
                }
            })?;
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
        assert!(matches!(cmd, Command::Render { ref file } if file.as_deref() == Some("diagram.mmd")));
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
}


