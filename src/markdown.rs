use crate::renderer::render_all;
use std::error::Error;

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

pub fn transform(input: &str) -> Result<String, Box<dyn Error>> {
    let blocks = extract_blocks(input);
    if blocks.is_empty() {
        return Ok(input.to_string());
    }

    let codes: Vec<String> = blocks.iter().map(|b| b.code.clone()).collect();
    let svgs = render_all(codes)?;

    // 後ろから置換することで文字位置がずれない
    let mut output = input.to_string();
    for (block, svg) in blocks.iter().zip(svgs.iter()).rev() {
        output.replace_range(block.start..block.end, svg);
    }

    Ok(output)
}
