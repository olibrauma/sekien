use crate::renderer::render_all;
use serde_json::{json, Value};
use std::env;
use std::error::Error;
use std::fs;

fn is_mermaid_block(block: &Value) -> bool {
    block["t"] == "CodeBlock"
        && block["c"][0][1]
            .as_array()
            .map(|classes| classes.iter().any(|c| c == "mermaid"))
            .unwrap_or(false)
}

pub fn filter(input: &str) -> Result<String, Box<dyn Error>> {
    let mut ast: Value = serde_json::from_str(input)?;

    // Mermaid ブロックを収集
    let blocks = ast["blocks"].as_array().ok_or("no blocks")?;
    let mermaid_indices: Vec<usize> = blocks
        .iter()
        .enumerate()
        .filter(|(_, b)| is_mermaid_block(b))
        .map(|(i, _)| i)
        .collect();

    if mermaid_indices.is_empty() {
        return Ok(input.to_string());
    }

    let codes: Vec<String> = mermaid_indices
        .iter()
        .map(|&i| blocks[i]["c"][1].as_str().unwrap_or("").to_string())
        .collect();

    let svgs = render_all(codes)?;

    // SVG をファイルに書き出し、CodeBlock → Para(Image) に差し替え
    let pid = std::process::id();
    let tmp = env::temp_dir();

    let blocks_mut = ast["blocks"].as_array_mut().unwrap();
    for (i, (&idx, svg)) in mermaid_indices.iter().zip(svgs.iter()).enumerate() {
        let path = tmp.join(format!("mmsvg_{}_{}.svg", pid, i));
        fs::write(&path, svg)?;
        let path_str = path.to_string_lossy().into_owned();
        blocks_mut[idx] = json!({
            "t": "Para",
            "c": [{ "t": "Image", "c": [["", [], []], [], [path_str, ""]] }]
        });
    }

    Ok(serde_json::to_string(&ast)?)
}
