mod markdown;
mod pandoc;
mod renderer;

use std::error::Error;
use std::fs;
use std::io::{self, Read};

fn usage() -> &'static str {
    "Usage:
  mmsvg [file.md]           Markdown in → Markdown out (Mermaid → SVG)
  mmsvg --pandoc-filter     Pandoc AST JSON in → AST JSON out
  cat file.md | mmsvg"
}

fn read_input(path: Option<&str>) -> Result<String, Box<dyn Error>> {
    match path {
        Some(p) => Ok(fs::read_to_string(p)?),
        None => {
            let mut buf = String::new();
            io::stdin().read_to_string(&mut buf)?;
            Ok(buf)
        }
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("{}", usage());
        return Ok(());
    }

    let is_pandoc_filter = args.iter().any(|a| a == "--pandoc-filter");
    let file_path = args.iter().find(|a| !a.starts_with('-')).map(|s| s.as_str());

    let input = read_input(file_path)?;

    let output = if is_pandoc_filter {
        pandoc::filter(&input)?
    } else {
        markdown::transform(&input)?
    };

    print!("{}", output);
    Ok(())
}
