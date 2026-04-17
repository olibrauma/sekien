mod pandoc;
mod renderer;

use std::error::Error;
use std::fs;
use std::io::{self, Read};

fn usage() -> &'static str {
    "Usage:
  mmsvg [file.mmd]          Mermaid → SVG (stdout)
  cat diagram.mmd | mmsvg   Mermaid → SVG (stdout)
  mmsvg --pandoc-filter     Pandoc AST JSON in → AST JSON out"
}

fn read_mermaid(file_path: Option<&str>) -> Result<String, Box<dyn Error>> {
    match file_path {
        Some(p) => Ok(fs::read_to_string(p)?),
        None => {
            let mut buf = String::new();
            io::stdin().read_to_string(&mut buf)?;
            Ok(buf)
        }
    }
}

// pandoc は filter を `<binary> <output-format>` で呼び出す。
// 引数が 1 つでフラグでもファイルパスでもなければ pandoc filter モードと判定する。
fn is_pandoc_filter(args: &[String]) -> bool {
    if args.iter().any(|a| a == "--pandoc-filter") {
        return true;
    }
    args.len() == 1
        && !args[0].starts_with('-')
        && !args[0].contains('/')
        && !args[0].contains('.')
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("{}", usage());
        return Ok(());
    }

    if is_pandoc_filter(&args) {
        let mut input = String::new();
        io::stdin().read_to_string(&mut input)?;
        print!("{}", pandoc::filter(&input)?);
        return Ok(());
    }

    // Mermaid → SVG (stdout)
    let file_path = args.iter().find(|a| !a.starts_with('-')).map(|s| s.as_str());
    let code = read_mermaid(file_path)?;
    let svgs = renderer::render_all(vec![code])?;
    print!("{}", svgs[0]);
    Ok(())
}
