mod render;
#[cfg(target_os = "linux")]
mod linux_display;

use anyhow::{bail, Context, Result};
use render::{RenderConfig, MERMAID_VERSION};
use std::env;
use std::fs;
use std::io::{self, Read};

fn usage() -> String {
    format!(
        "sekien — Mermaid Drawer (mermaid.js {MERMAID_VERSION})

Usage:
  sekien [options] [file.mmd]         Mermaid → SVG (stdout)
  cat diagram.mmd | sekien            Mermaid → SVG (stdout)

sekien は cat のような streaming プロセス。stdin (またはファイル) を EOF まで
読み続け、`\\0` (NUL byte) を block 区切りとして 1 つずつ Mermaid → SVG に変換し、
SVG を `\\0` 区切りで stdout に流す。

block 単位の Mermaid 解析エラーは stderr に `Error: mermaid block N: <msg>`
を 1 行出して継続する (exit 0)。sekien 自身の失敗 (display 初期化失敗、
malformed IPC、stdout 書き込み失敗等) は exit 1。

対話モードで使う場合、terminal 上で Ctrl + @ が NUL byte を入力する手段。
EOF (Ctrl + D) で sekien を終了させる。

Options:
  --font <font>          Font family for diagram text (default: mermaid.js default)
                         Also configurable via SEKIEN_FONT env var.
  --theme <theme>        Mermaid theme (default | base | dark | forest | neutral |
                           neo | neo-dark | redux | redux-dark | null)
                         Also configurable via SEKIEN_THEME env var.
  --look <look>          Diagram look (classic | handDrawn | neo)
                         handDrawn is supported for flowchart/graph only.
                         Also configurable via SEKIEN_LOOK env var.
  --block-id             Prepend an XML comment with the block ID to each SVG output
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
    show_block_ids: bool,
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
            "--block-id" => {
                options.show_block_ids = true;
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

    Ok((options, Command::Render { file: rest.into_iter().next() }))
}

fn open_reader(file_path: Option<&str>) -> Result<Box<dyn Read + Send>> {
    match file_path {
        Some(p) => {
            let f = fs::File::open(p).with_context(|| format!("cannot read '{p}'"))?;
            Ok(Box::new(f))
        }
        None => Ok(Box::new(io::stdin())),
    }
}

fn main() -> Result<()> {
    #[cfg(target_os = "linux")]
    {
        // To avoid window flicker on Wayland and ensure reliable headless rendering,
        // we force xvfb-run if we are in a Wayland session or if no DISPLAY is set.
        // We also force GDK_BACKEND=x11 to work correctly with Xvfb.
        let is_wayland = std::env::var("WAYLAND_DISPLAY").is_ok();
        let no_display = std::env::var("DISPLAY").is_err();
        let already_in_xvfb = std::env::var("SEKIEN_XVFBRUN").is_ok();

        if (is_wayland || no_display) && !already_in_xvfb {
            let status = std::process::Command::new("xvfb-run")
                .arg("-a")
                .arg("-s")
                .arg("-screen 0 1280x1024x24")
                .env("GDK_BACKEND", "x11")
                .env("LIBGL_ALWAYS_SOFTWARE", "1") // Silence driver warnings
                .env("SEKIEN_XVFBRUN", "1") // Prevent infinite recursion
                .arg(std::env::current_exe()?)
                .args(std::env::args().skip(1))
                .status()
                .context("failed to execute xvfb-run. please ensure xvfb is installed.")?;
            std::process::exit(status.code().unwrap_or(0));
        }
    }

    let raw: Vec<String> = env::args().skip(1).collect();
    let (options, command) = parse_args(raw)?;

    let config = RenderConfig {
        font_family: options.font_family.or_else(|| env::var("SEKIEN_FONT").ok()),
        theme:       options.theme      .or_else(|| env::var("SEKIEN_THEME").ok()),
        look:        options.look       .or_else(|| env::var("SEKIEN_LOOK").ok()),
        show_block_ids: options.show_block_ids,
    };

    match command {
        Command::Help => println!("{}", usage()),
        Command::Version => {
            println!("sekien {} (mermaid.js {})", env!("CARGO_PKG_VERSION"), MERMAID_VERSION);
        }
        Command::Render { file } => {
            let reader = open_reader(file.as_deref())?;
            render::run_stream(reader, &config)?;
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
