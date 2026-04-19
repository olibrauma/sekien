use crate::renderer::render_all;
use serde_json::{json, Value};
use std::error::Error;

fn is_mermaid_block(block: &Value) -> bool {
    block["t"] == "CodeBlock"
        && block["c"][0][1]
            .as_array()
            .map(|classes| classes.iter().any(|c| c == "mermaid"))
            .unwrap_or(false)
}

pub fn filter(input: &str, font_family: &str) -> Result<String, Box<dyn Error>> {
    let mut ast: Value = serde_json::from_str(input)?;

    // Mermaid ブロックを収集
    let blocks = ast["blocks"].as_array().ok_or("no blocks")?;
    let (mermaid_indices, codes): (Vec<usize>, Vec<String>) = blocks
        .iter()
        .enumerate()
        .filter(|(_, b)| is_mermaid_block(b))
        .map(|(i, b)| (i, b["c"][1].as_str().unwrap_or("").to_string()))
        .unzip();

    if mermaid_indices.is_empty() {
        return Ok(input.to_string());
    }

    let svgs = render_all(codes, font_family)?;

    // CodeBlock → RawBlock("html", svg) に置換
    let blocks_mut = ast["blocks"].as_array_mut().unwrap();
    for (&idx, svg) in mermaid_indices.iter().zip(svgs.iter()) {
        blocks_mut[idx] = json!({ "t": "RawBlock", "c": ["html", svg] });
    }

    Ok(serde_json::to_string(&ast)?)
}
