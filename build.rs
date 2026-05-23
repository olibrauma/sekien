use std::env;
use std::fs;
use std::path::PathBuf;

fn main() {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    let mermaid_path = PathBuf::from(&manifest_dir).join("src/assets/mermaid.min.js");

    println!("cargo:rerun-if-changed=src/assets/mermaid.min.js");
    println!("cargo:rerun-if-changed=src/assets/render.html");
    println!("cargo:rerun-if-changed=build.rs");

    let content = fs::read_to_string(&mermaid_path)
        .unwrap_or_else(|e| panic!("read {mermaid_path:?}: {e}"));

    let version = extract_version(&content)
        .unwrap_or_else(|| panic!("could not find `version:\"X.Y.Z\"` in mermaid.min.js"));

    println!("cargo:rustc-env=MERMAID_VERSION={version}");
}

fn extract_version(content: &str) -> Option<String> {
    let needle = "version:\"";
    let start = content.find(needle)?;
    let after = &content[start + needle.len()..];
    let end = after.find('"')?;
    Some(after[..end].to_string())
}
