mod pandoc;

use sekien::{RenderConfig, MERMAID_VERSION};
use std::io::{self, Read};

const LUA_FILTER: &str = include_str!("../assets/sekien.lua");

fn usage() -> String {
    format!(
        "sekien-pandoc — Pandoc filter for Mermaid diagrams (mermaid.js {MERMAID_VERSION})

Usage:
  pandoc input.md -o output.html --filter sekien-pandoc
  pandoc input.md -o output.pdf --pdf-engine=typst \\
    --filter sekien-pandoc \\
    --lua-filter <(sekien-pandoc --print-lua-filter)

Options:
  --print-lua-filter     Print the bundled Lua filter for non-HTML PDF output
  --version, -v          Show version
  --help, -h             Show this help

Rendering options (via environment variables):
  SEKIEN_FONT            Font family for diagram text
  SEKIEN_THEME           Mermaid theme (default | base | dark | forest | neutral |
                           neo | neo-dark | redux | redux-dark | null)
  SEKIEN_LOOK            Diagram look (classic | handDrawn | neo)"
    )
}

fn main() {

    let args: Vec<String> = std::env::args().skip(1).collect();

    for arg in &args {
        match arg.as_str() {
            "--help" | "-h" => {
                println!("{}", usage());
                return;
            }
            "--version" | "-v" => {
                println!("sekien-pandoc {} (mermaid.js {})", env!("CARGO_PKG_VERSION"), MERMAID_VERSION);
                return;
            }
            "--print-lua-filter" => {
                print!("{}", LUA_FILTER);
                return;
            }
            _ => {} // pandoc が渡す format 引数。無視する。
        }
    }

    let mut input = String::new();
    if let Err(e) = io::stdin().read_to_string(&mut input) {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }

    let config = RenderConfig {
        font_family: std::env::var("SEKIEN_FONT").ok(),
        theme:       std::env::var("SEKIEN_THEME").ok(),
        look:        std::env::var("SEKIEN_LOOK").ok(),
    };
    if let Err(e) = pandoc::filter(&input, &config) {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}


