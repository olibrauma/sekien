mod pandoc;
mod renderer;

use anyhow::{Context, Result};
use std::fs;
use std::io::{self, Read};

const LUA_FILTER: &str = include_str!("../assets/sekien.lua");

fn usage() -> &'static str {
    "sekien — Mermaid Drawer

Usage:
  sekien [--font <font>] [file.mmd]   Mermaid → SVG (stdout)
  cat diagram.mmd | sekien                   Mermaid → SVG (stdout)
  pandoc --filter sekien                     Pandoc filter (called automatically by pandoc)

Options:
  --font <font>          Font family for diagram text (default: mermaid.js default)
                         Also configurable via SEKIEN_FONT_FAMILY environment variable.
                         In pandoc filter mode, use the environment variable instead.
  --print-lua-filter     Print the bundled Lua filter for non-HTML PDF output (see below)
  --version, -v          Show version
  --help, -h             Show this help

Non-HTML PDF output:
  sekien outputs RawBlock(\"html\", svg), which PDF engines that don't process raw HTML
  (e.g. typst, pdflatex) will drop. Use the bundled Lua filter to convert SVG blocks
  to Image nodes that these engines can include:

    sekien --print-lua-filter > sekien.lua
    pandoc input.md -o output.pdf --pdf-engine=typst --filter sekien --lua-filter sekien.lua

  To install globally (reference by name without path):
    sekien --print-lua-filter > ~/.local/share/pandoc/filters/sekien.lua
    pandoc input.md -o output.pdf --pdf-engine=typst --filter sekien --lua-filter sekien.lua"
}

// パース済み引数
struct Args {
    font_family: Option<String>,
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
    let mut font_family = None;
    let mut rest: Vec<String> = Vec::new();
    let mut iter = raw.into_iter();

    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--help" | "-h"         => return Ok(Args { font_family: None, command: Command::Help }),
            "--version" | "-v"      => return Ok(Args { font_family: None, command: Command::Version }),
            "--print-lua-filter"    => return Ok(Args { font_family: None, command: Command::PrintLuaFilter }),
            "--font" => {
                font_family = Some(iter.next().ok_or("--font requires a value")?);
            }
            _ => rest.push(arg),
        }
    }

    // pandoc は filter を `<binary> <output-format>` で呼び出す。
    // 引数が 1 つでフラグでもファイルパスでもなければ pandoc filter モードと判定する。
    let command = if rest.len() == 1
        && !rest[0].starts_with('-')
        && !rest[0].contains('/')
        && !rest[0].contains('.')
    {
        Command::PandocFilter
    } else {
        let files: Vec<String> = rest.into_iter().filter(|a| !a.starts_with('-')).collect();
        if files.len() > 1 {
            return Err(format!(
                "error: too many arguments (sekien takes at most one file)\n\
                 hint:  for multiple files, use a shell loop:\n\
                 \t for f in *.mmd; do sekien \"$f\" > \"${{f%.mmd}}.svg\"; done"
            ));
        }
        Command::Render { file: files.into_iter().next() }
    };

    Ok(Args { font_family, command })
}

fn resolve_font_family(flag: Option<String>) -> Option<String> {
    flag.or_else(|| std::env::var("SEKIEN_FONT_FAMILY").ok())
}

fn resolve_theme() -> Option<String> {
    std::env::var("SEKIEN_THEME").ok()
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
    fn file_with_dot_is_render_not_pandoc() {
        // "diagram.mmd" はドットを含むので Render モード
        let a = parse_args(args(&["diagram.mmd"])).unwrap();
        assert!(matches!(a.command, Command::Render { .. }));
    }

    #[test]
    fn file_with_slash_is_render_not_pandoc() {
        let a = parse_args(args(&["./diagram.mmd"])).unwrap();
        assert!(matches!(a.command, Command::Render { .. }));
    }

    #[test]
    fn font_flag() {
        let a = parse_args(args(&["--font", "Arial"])).unwrap();
        assert_eq!(a.font_family, Some("Arial".to_string()));
    }

    #[test]
    fn font_flag_with_file() {
        let a = parse_args(args(&["--font", "Arial", "diagram.mmd"])).unwrap();
        assert_eq!(a.font_family, Some("Arial".to_string()));
        assert!(matches!(a.command, Command::Render { .. }));
    }

    #[test]
    fn font_flag_missing_value_is_error() {
        assert!(parse_args(args(&["--font"])).is_err());
    }

    #[test]
    fn too_many_files_is_error() {
        assert!(parse_args(args(&["a.mmd", "b.mmd"])).is_err());
    }
}

fn main() -> Result<()> {
    let raw: Vec<String> = std::env::args().skip(1).collect();

    let args = parse_args(raw).unwrap_or_else(|e| {
        eprintln!("{}", e);
        std::process::exit(1);
    });

    let font_family = resolve_font_family(args.font_family);
    let theme = resolve_theme();

    match args.command {
        Command::Help => {
            println!("{}", usage());
        }
        Command::Version => {
            println!("sekien {}", env!("CARGO_PKG_VERSION"));
        }
        Command::PrintLuaFilter => {
            print!("{}", LUA_FILTER);
        }
        Command::PandocFilter => {
            let mut input = String::new();
            io::stdin().read_to_string(&mut input)?;
            print!("{}", pandoc::filter(&input, font_family.as_deref(), theme.as_deref())?);
        }
        Command::Render { file } => {
            let code = read_mermaid(file.as_deref())?;
            let svgs = renderer::render_all(vec![code], font_family.as_deref(), theme.as_deref())?;
            println!("{}", svgs[0]);
        }
    }

    Ok(())
}
