use sha2::{Digest, Sha256};
use std::env;
use std::fs;
use std::path::PathBuf;

/// `src/assets/mermaid.min.js` の期待される SHA256 ハッシュ値。
/// 手動更新時に改ざんや事故を防ぐための完全性検証。
const EXPECTED_MERMAID_SHA: &str =
    "217b66ef4279c33c141b4afe22effad10a91c02558dc70917be2c0981e78ed87";

fn main() {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    let mermaid_path = PathBuf::from(&manifest_dir).join("src/assets/mermaid.min.js");

    println!("cargo:rerun-if-changed=src/assets/mermaid.min.js");
    println!("cargo:rerun-if-changed=src/assets/render.html");
    println!("cargo:rerun-if-changed=build.rs");

    let bytes = fs::read(&mermaid_path).unwrap_or_else(|e| panic!("read {mermaid_path:?}: {e}"));

    // 完全性検証 (SHA256)
    let actual_sha = format!("{:x}", Sha256::digest(&bytes));
    if actual_sha != EXPECTED_MERMAID_SHA {
        panic!(
            "\n\n\
            [INTEGRITY ERROR] src/assets/mermaid.min.js does not match the expected SHA256 hash!\n\
            Expected: {EXPECTED_MERMAID_SHA}\n\
            Actual:   {actual_sha}\n\n\
            If you intended to update mermaid.min.js, please update EXPECTED_MERMAID_SHA in build.rs.\n\n"
        );
    }

    let content = String::from_utf8_lossy(&bytes);
    let version = extract_version(&content)
        .unwrap_or_else(|| panic!("could not find version:\"X.Y.Z\" in mermaid.min.js"));

    println!("cargo:rustc-env=MERMAID_VERSION={version}");
}

fn extract_version(content: &str) -> Option<String> {
    let needle = "version:\"";
    let start = content.find(needle)?;
    let after = &content[start + needle.len()..];
    let end = after.find('"')?;
    Some(after[..end].to_string())
}
