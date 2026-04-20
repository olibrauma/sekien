use crate::renderer::render_all;
use anyhow::{Context, Result};
use serde_json::{json, Value};

fn is_mermaid_block(block: &Value) -> bool {
    block["t"] == "CodeBlock"
        && block["c"][0][1]
            .as_array()
            .map(|classes| classes.iter().any(|c| c == "mermaid"))
            .unwrap_or(false)
}

pub fn filter(input: &str, font_family: Option<&str>, theme: Option<&str>) -> Result<String> {
    let mut ast: Value = serde_json::from_str(input).context("invalid pandoc AST")?;

    // Mermaid ブロックを収集
    let blocks = ast["blocks"].as_array().context("no blocks in pandoc AST")?;
    let (mermaid_indices, codes): (Vec<usize>, Vec<String>) = blocks
        .iter()
        .enumerate()
        .filter(|(_, b)| is_mermaid_block(b))
        .map(|(i, b)| (i, b["c"][1].as_str().unwrap_or("").to_string()))
        .unzip();

    if mermaid_indices.is_empty() {
        return Ok(input.to_string());
    }

    let svgs = render_all(codes, font_family, theme)?;

    // CodeBlock → RawBlock("html", svg) に置換
    let blocks_mut = ast["blocks"].as_array_mut().unwrap();
    for (&idx, svg) in mermaid_indices.iter().zip(svgs.iter()) {
        blocks_mut[idx] = json!({ "t": "RawBlock", "c": ["html", svg] });
    }

    Ok(serde_json::to_string(&ast).context("failed to serialize pandoc AST")?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn mermaid_codeblock_is_detected() {
        let block = json!({
            "t": "CodeBlock",
            "c": [["", ["mermaid"], []], "graph LR\n  A --> B"]
        });
        assert!(is_mermaid_block(&block));
    }

    #[test]
    fn non_mermaid_codeblock_is_ignored() {
        let block = json!({
            "t": "CodeBlock",
            "c": [["", ["rust"], []], "fn main() {}"]
        });
        assert!(!is_mermaid_block(&block));
    }

    #[test]
    fn non_codeblock_is_ignored() {
        let block = json!({ "t": "Para", "c": [] });
        assert!(!is_mermaid_block(&block));
    }

    #[test]
    fn codeblock_with_multiple_classes_including_mermaid() {
        let block = json!({
            "t": "CodeBlock",
            "c": [["", ["language-mermaid", "mermaid"], []], "graph LR\n  A --> B"]
        });
        assert!(is_mermaid_block(&block));
    }

    #[test]
    fn filter_with_no_mermaid_blocks_returns_input_unchanged() {
        let input = r#"{"pandoc-api-version":[1,23],"meta":{},"blocks":[{"t":"Para","c":[]}]}"#;
        let output = filter(input, None, None).unwrap();
        assert_eq!(output, input);
    }

    #[test]
    fn filter_invalid_json_returns_error() {
        let result = filter("not json", None, None);
        assert!(result.is_err());
    }
}
