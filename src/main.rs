mod pandoc;
mod renderer;

use anyhow::Result;
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
        // On Linux, if we are in a Wayland session, we MUST use GDK_BACKEND=x11
        // to avoid "width > 0" assertion failures when creating hidden windows.
        // If GDK_BACKEND is not set and we are on Wayland, restart ourselves.
        if std::env::var("WAYLAND_DISPLAY").is_ok() && std::env::var("GDK_BACKEND").is_err() {
            let status = std::process::Command::new(std::env::current_exe()?)
                .env("GDK_BACKEND", "x11")
                .args(std::env::args().skip(1))
                .status()?;
            std::process::exit(status.code().unwrap_or(0));
        }

        // If no DISPLAY is available at all, try to use xvfb-run.
        if std::env::var("DISPLAY").is_err() {
            let status = std::process::Command::new("xvfb-run")
                .arg("-a")
                .arg(std::env::current_exe()?)
                .args(std::env::args().skip(1))
                .status()?;
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
