//! Pure construction of what is sent to the WebView: the page, and the
//! script that asks it to render a diagram.

use crate::error::{Error, Result};

const MERMAID_JS: &str = include_str!("../../assets/mermaid.min.js");
const HTML_TEMPLATE: &str = include_str!("../../assets/render.html");

/// Escapes `<` and `>` to Unicode escapes for safe embedding inside `<script>`.
fn escape_for_script(s: &str) -> String {
    s.replace('<', "\\u003c").replace('>', "\\u003e")
}

/// Validates that `config_json` (if present) parses as a JSON object.
///
/// A `config_json` that isn't a valid JS object literal breaks
/// `mermaid.initialize()` in the WebView before it can signal readiness,
/// which would otherwise hang [`crate::render_stream`] forever instead of
/// returning an error.
pub(super) fn validate_config_json(config_json: Option<&str>) -> Result<()> {
    if let Some(s) = config_json {
        let value: serde_json::Value =
            serde_json::from_str(s).map_err(|e| Error::Config(e.to_string()))?;
        if !value.is_object() {
            return Err(Error::Config(format!("expected a JSON object, got: {s}")));
        }
    }
    Ok(())
}

pub(super) fn build_html(config_json: Option<&str>) -> String {
    let config_json = config_json.map_or_else(|| "{}".to_string(), escape_for_script);

    // The user's config is substituted last, so it is never scanned for
    // placeholders; mermaid.js must not contain `{{CONFIG_JSON}}` (tested).
    HTML_TEMPLATE
        .replace("{{MERMAID_JS}}", MERMAID_JS)
        .replace("{{CONFIG_JSON}}", &config_json)
}

/// The script that asks the page to render diagram `id` (`content`). The page
/// reports the result back via IPC with the same `id`.
pub(super) fn render_script(id: usize, content: &str) -> String {
    // serde_json produces a valid JS string literal (escaping `"`, `\`,
    // control chars, U+2028/U+2029). evaluate_script bypasses the HTML parser,
    // so `</script>` does not need the extra escaping that build_html requires.
    let literal = serde_json::to_string(content).expect("serialize Mermaid block content");
    format!("renderMermaid({id}, {literal})")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mermaid_js_contains_no_later_placeholder() {
        assert!(!MERMAID_JS.contains("{{CONFIG_JSON}}"));
    }

    #[test]
    fn render_script_passes_content_as_a_js_string_literal() {
        assert_eq!(
            render_script(3, "graph LR\n  A[\"</script>\"]"),
            r#"renderMermaid(3, "graph LR\n  A[\"</script>\"]")"#
        );
    }

    #[test]
    fn escape_for_script_escapes_angle_brackets() {
        assert_eq!(
            escape_for_script("</script><script>"),
            "\\u003c/script\\u003e\\u003cscript\\u003e"
        );
    }

    #[test]
    fn validate_config_json_accepts_none_and_objects() {
        assert!(validate_config_json(None).is_ok());
        assert!(validate_config_json(Some("{}")).is_ok());
        assert!(validate_config_json(Some(r#"{"theme":"dark"}"#)).is_ok());
    }

    #[test]
    fn validate_config_json_rejects_non_objects_and_invalid_json() {
        assert!(matches!(
            validate_config_json(Some("not json")),
            Err(Error::Config(_))
        ));
        assert!(matches!(
            validate_config_json(Some("[1, 2, 3]")),
            Err(Error::Config(_))
        ));
        assert!(matches!(
            validate_config_json(Some(r#""a string""#)),
            Err(Error::Config(_))
        ));
    }

    #[test]
    fn build_html_defaults_to_empty_config() {
        let html = build_html(None);
        assert!(html.contains("...{}"));
    }

    #[test]
    fn build_html_with_config_json() {
        let html = build_html(Some(r#"{"flowchart":{"curve":"basis"}}"#));
        assert!(html.contains(r#"...{"flowchart":{"curve":"basis"}}"#));
    }

    #[test]
    fn build_html_escapes_closing_script_tags_in_config_json() {
        // Embedding `</script>` must not break out of the script block.
        let html = build_html(Some(r#"{"theme":"</script>"}"#));
        assert!(!html.contains("</script>\""));
        assert!(html.contains("\\u003c/script\\u003e"));
    }
}
