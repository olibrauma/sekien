use crate::renderer::{render_all, RenderConfig};
use anyhow::{Context, Result};
use serde_json::{json, Value};

fn is_mermaid_block(block: &Value) -> bool {
    block["t"] == "CodeBlock"
        && block["c"][0][1]
            .as_array()
            .map(|classes| classes.iter().any(|c| c == "mermaid"))
            .unwrap_or(false)
}

fn collect_mermaid(blocks: &[Value]) -> Vec<(usize, String)> {
    blocks
        .iter()
        .enumerate()
        .filter(|(_, b)| is_mermaid_block(b))
        .map(|(i, b)| (i, b["c"][1].as_str().unwrap_or("").to_string()))
        .collect()
}

pub fn filter(input: &str, config: &RenderConfig) -> Result<String> {
    let mut ast: Value = serde_json::from_str(input).context("invalid pandoc AST")?;

    let mermaid = {
        let blocks = ast["blocks"].as_array().context("no blocks in pandoc AST")?;
        collect_mermaid(blocks)
    };

    if mermaid.is_empty() {
        return Ok(input.to_string());
    }

    let (indices, codes): (Vec<usize>, Vec<String>) = mermaid.into_iter().unzip();
    let svgs = render_all(codes, config)?;

    let blocks_mut = ast["blocks"].as_array_mut().expect("blocks is array (verified above)");
    for (&idx, svg) in indices.iter().zip(svgs.iter()) {
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
    fn collect_mermaid_finds_correct_indices_and_codes() {
        let blocks = vec![
            json!({ "t": "Para", "c": [] }),
            json!({ "t": "CodeBlock", "c": [["", ["mermaid"], []], "graph LR\n  A --> B"] }),
            json!({ "t": "CodeBlock", "c": [["", ["rust"], []], "fn main() {}"] }),
            json!({ "t": "CodeBlock", "c": [["", ["mermaid"], []], "graph TD\n  X --> Y"] }),
        ];
        let result = collect_mermaid(&blocks);
        assert_eq!(result, vec![
            (1, "graph LR\n  A --> B".to_string()),
            (3, "graph TD\n  X --> Y".to_string()),
        ]);
    }

    #[test]
    fn filter_with_no_mermaid_blocks_returns_input_unchanged() {
        let input = r#"{"pandoc-api-version":[1,23],"meta":{},"blocks":[{"t":"Para","c":[]}]}"#;
        let config = RenderConfig::default();
        let output = filter(input, &config).unwrap();
        assert_eq!(output, input);
    }

    #[test]
    fn filter_invalid_json_returns_error() {
        let config = RenderConfig::default();
        assert!(filter("not json", &config).is_err());
    }
}
