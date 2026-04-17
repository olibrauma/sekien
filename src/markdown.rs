use crate::renderer::render_all;
use std::env;
use std::error::Error;
use std::fs;
use std::path::Path;

// ```mermaid\n...\n``` にマッチ
const FENCE_OPEN: &str = "```mermaid\n";
const FENCE_CLOSE: &str = "\n```";

struct MermaidBlock {
    start: usize, // ``` の開始位置
    end: usize,   // ``` の終了位置 (closing ``` の末尾)
    code: String,
}

fn extract_blocks(input: &str) -> Vec<MermaidBlock> {
    let mut blocks = Vec::new();
    let mut search_from = 0;

    while let Some(open) = input[search_from..].find(FENCE_OPEN) {
        let open = open + search_from;
        let code_start = open + FENCE_OPEN.len();

        if let Some(close) = input[code_start..].find(FENCE_CLOSE) {
            let close_pos = code_start + close;
            let end = close_pos + FENCE_CLOSE.len();
            blocks.push(MermaidBlock {
                start: open,
                end,
                code: input[code_start..close_pos].to_string(),
            });
            search_from = end;
        } else {
            break;
        }
    }

    blocks
}

// input_path: ファイル入力の場合は Some。SVG の置き場所と参照パスの形式を決める。
// - Some(path): 入力 .md の隣に {stem}-{i}.svg を書き出し、相対パスで参照 (mmdc 互換)
// - None (stdin): temp dir に書き出し、絶対パスで参照
pub fn transform(input: &str, input_path: Option<&Path>) -> Result<String, Box<dyn Error>> {
    let blocks = extract_blocks(input);
    if blocks.is_empty() {
        return Ok(input.to_string());
    }

    let codes: Vec<String> = blocks.iter().map(|b| b.code.clone()).collect();
    let svgs = render_all(codes)?;

    // SVG ファイルの書き出し先と参照パスを決定
    let svg_refs: Vec<(std::path::PathBuf, String)> = match input_path {
        Some(p) => {
            let dir = p.parent().unwrap_or(Path::new("."));
            let stem = p.file_stem().unwrap_or_default().to_string_lossy();
            svgs.iter()
                .enumerate()
                .map(|(i, svg)| {
                    let name = format!("{}-{}.svg", stem, i);
                    let abs = dir.join(&name);
                    fs::write(&abs, svg)?;
                    Ok::<_, Box<dyn Error>>((abs, name))
                })
                .collect::<Result<Vec<_>, _>>()?
        }
        None => {
            let pid = std::process::id();
            let tmp = env::temp_dir();
            svgs.iter()
                .enumerate()
                .map(|(i, svg)| {
                    let abs = tmp.join(format!("mmsvg_{}_{}.svg", pid, i));
                    fs::write(&abs, svg)?;
                    let ref_str = abs.to_string_lossy().into_owned();
                    Ok::<_, Box<dyn Error>>((abs, ref_str))
                })
                .collect::<Result<Vec<_>, _>>()?
        }
    };

    // 後ろから置換することで文字位置がずれない
    let mut output = input.to_string();
    for (block, (_, ref_str)) in blocks.iter().zip(svg_refs.iter()).rev() {
        output.replace_range(block.start..block.end, &format!("![]({})", ref_str));
    }

    Ok(output)
}
