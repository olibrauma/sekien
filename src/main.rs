mod pandoc;
mod renderer;

use anyhow::{Context, Result};
use renderer::{RenderConfig, MERMAID_VERSION};
use std::fs;
use std::io::{self, Read};

const LUA_FILTER: &str = include_str!("../assets/sekien.lua");

fn usage() -> String {
    format!("sekien — Mermaid Drawer (mermaid.js {})

Usage:
  sekien [options] [file.mmd]         Mermaid → SVG (stdout)
  cat diagram.mmd | sekien            Mermaid → SVG (stdout)
  pandoc --filter sekien              Pandoc filter (called automatically by pandoc)

Options:
  --font <font>          Font family for diagram text (default: mermaid.js default)
                         Also configurable via SEKIEN_FONT env var.
  --theme <theme>        Mermaid theme (default | base | dark | forest | neutral |
                           neo | neo-dark | redux | redux-dark | null)
                         Also configurable via SEKIEN_THEME env var.
  --look <look>          Diagram look (classic | handDrawn | neo)
                         handDrawn is supported for flowchart/graph only.
                         Also configurable via SEKIEN_LOOK env var.
  --print-lua-filter     Print the bundled Lua filter for non-HTML PDF output (see below)
  --version, -v          Show version
  --help, -h             Show this help",
    MERMAID_VERSION)
}

#[derive(Default)]
struct Options {
    font_family: Option<String>,
    theme: Option<String>,
    look: Option<String>,
}

struct Args {
    options: Options,
    command: Command,
}

enum Command {
    Help,
    Version,
    PrintLuaFilter,
    PandocFilter,
    Render { file: Option<String> },
}

fn parse_args(raw: Vec<String>) -> Result<Args, String> {
    let mut options = Options::default();
    let mut rest: Vec<String> = Vec::new();
    let mut iter = raw.into_iter();

    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--help" | "-h"         => return Ok(Args { options: Options::default(), command: Command::Help }),
            "--version" | "-v"      => return Ok(Args { options: Options::default(), command: Command::Version }),
            "--print-lua-filter"    => return Ok(Args { options: Options::default(), command: Command::PrintLuaFilter }),
            "--font" => {
                options.font_family = Some(iter.next().ok_or("--font requires a value")?);
            }
            "--theme" => {
                options.theme = Some(iter.next().ok_or("--theme requires a value")?);
            }
            "--look" => {
                options.look = Some(iter.next().ok_or("--look requires a value")?);
            }
            _ => rest.push(arg),
        }
    }

    let command = if rest.len() == 1
        && !rest[0].starts_with('-')
        && !rest[0].contains('/')
        && !rest[0].contains('.')
    {
        Command::PandocFilter
    } else {
        let files: Vec<String> = rest.into_iter().filter(|a| !a.starts_with('-')).collect();
        if files.len() > 1 {
            return Err("error: too many arguments".to_string());
        }
        Command::Render { file: files.into_iter().next() }
    };

    Ok(Args { options, command })
}

fn read_mermaid(file_path: Option<&str>) -> Result<String> {
    match file_path {
        Some(p) => fs::read_to_string(p).map_err(anyhow::Error::from),
        None => {
            let mut buf = String::new();
            io::stdin().read_to_string(&mut buf)?;
            Ok(buf)
        }
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

    let raw: Vec<String> = std::env::args().skip(1).collect();
    let args = parse_args(raw).unwrap_or_else(|e| {
        eprintln!("{}", e);
        std::process::exit(1);
    });

    let config = RenderConfig {
        font_family: args.options.font_family.or_else(|| std::env::var("SEKIEN_FONT").ok()),
        theme:       args.options.theme.or_else(|| std::env::var("SEKIEN_THEME").ok()),
        look:        args.options.look.or_else(|| std::env::var("SEKIEN_LOOK").ok()),
    };

    match args.command {
        Command::Help => println!("{}", usage()),
        Command::Version => println!("sekien {} (mermaid.js {})", env!("CARGO_PKG_VERSION"), MERMAID_VERSION),
        Command::PrintLuaFilter => print!("{}", LUA_FILTER),
        Command::PandocFilter => {
            let mut input = String::new();
            io::stdin().read_to_string(&mut input)?;
            pandoc::filter(&input, &config)?;
        }
        Command::Render { file } => {
            let code = read_mermaid(file.as_deref())?;
            renderer::render_all(vec![code], &config, |svgs| {
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
        let a = parse_args(args(&["--help"])).unwrap();
        assert!(matches!(a.command, Command::Help));
    }

    #[test]
    fn help_short() {
        let a = parse_args(args(&["-h"])).unwrap();
        assert!(matches!(a.command, Command::Help));
    }

    #[test]
    fn version_long() {
        let a = parse_args(args(&["--version"])).unwrap();
        assert!(matches!(a.command, Command::Version));
    }

    #[test]
    fn version_short() {
        let a = parse_args(args(&["-v"])).unwrap();
        assert!(matches!(a.command, Command::Version));
    }

    #[test]
    fn print_lua_filter() {
        let a = parse_args(args(&["--print-lua-filter"])).unwrap();
        assert!(matches!(a.command, Command::PrintLuaFilter));
    }

    // pandoc は `sekien <format>` で呼び出す。フォーマット名はドット・スラッシュ・ハイフンを含まない。
    #[test]
    fn pandoc_filter_mode_common_formats() {
        for fmt in &["html", "latex", "markdown", "docx", "rst"] {
            let a = parse_args(args(&[fmt])).unwrap();
            assert!(
                matches!(a.command, Command::PandocFilter),
                "expected PandocFilter for format: {fmt}"
            );
        }
    }

    #[test]
    fn render_no_args() {
        let a = parse_args(args(&[])).unwrap();
        assert!(matches!(a.command, Command::Render { file: None }));
    }

    #[test]
    fn render_with_file() {
        let a = parse_args(args(&["diagram.mmd"])).unwrap();
        assert!(matches!(
            a.command,
            Command::Render { ref file } if file.as_deref() == Some("diagram.mmd")
        ));
    }

    #[test]
    fn file_with_slash_is_render_not_pandoc() {
        let a = parse_args(args(&["./diagram.mmd"])).unwrap();
        assert!(matches!(a.command, Command::Render { .. }));
    }

    #[test]
    fn font_flag() {
        let a = parse_args(args(&["--font", "Arial"])).unwrap();
        assert_eq!(a.options.font_family, Some("Arial".to_string()));
    }

    #[test]
    fn font_flag_with_file() {
        let a = parse_args(args(&["--font", "Arial", "diagram.mmd"])).unwrap();
        assert_eq!(a.options.font_family, Some("Arial".to_string()));
        assert!(matches!(a.command, Command::Render { .. }));
    }

    #[test]
    fn font_flag_missing_value_is_error() {
        assert!(parse_args(args(&["--font"])).is_err());
    }

    #[test]
    fn theme_flag() {
        let a = parse_args(args(&["--theme", "dark"])).unwrap();
        assert_eq!(a.options.theme, Some("dark".to_string()));
    }

    #[test]
    fn theme_flag_with_file() {
        let a = parse_args(args(&["--theme", "forest", "diagram.mmd"])).unwrap();
        assert_eq!(a.options.theme, Some("forest".to_string()));
        assert!(matches!(a.command, Command::Render { .. }));
    }

    #[test]
    fn theme_flag_missing_value_is_error() {
        assert!(parse_args(args(&["--theme"])).is_err());
    }

    #[test]
    fn font_and_theme_flags() {
        let a = parse_args(args(&["--font", "Arial", "--theme", "dark", "diagram.mmd"])).unwrap();
        assert_eq!(a.options.font_family, Some("Arial".to_string()));
        assert_eq!(a.options.theme, Some("dark".to_string()));
    }

    #[test]
    fn look_flag() {
        let a = parse_args(args(&["--look", "handDrawn"])).unwrap();
        assert_eq!(a.options.look, Some("handDrawn".to_string()));
    }

    #[test]
    fn look_flag_missing_value_is_error() {
        assert!(parse_args(args(&["--look"])).is_err());
    }

    #[test]
    fn too_many_files_is_error() {
        assert!(parse_args(args(&["a.mmd", "b.mmd"])).is_err());
    }
}
