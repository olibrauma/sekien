use sha2::{Digest, Sha256};
use std::env;
use std::fs;
use std::path::PathBuf;

/// Expected SHA256 of `assets/mermaid.min.js`.
/// Guards against accidental or malicious modifications.
/// Rewritten by util/update-mermaid.sh.
const EXPECTED_MERMAID_SHA: &str =
    "28fca7ae6ebc7ed7bb63bde63136a74bfef14f296a57e403657eeb8b32836073";

fn main() {
    let manifest_dir = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR");
    let mermaid_path = PathBuf::from(&manifest_dir).join("assets/mermaid.min.js");

    println!("cargo:rerun-if-changed=assets/mermaid.min.js");
    println!("cargo:rerun-if-changed=assets/mermaid.version");
    println!("cargo:rerun-if-changed=assets/render.html");
    println!("cargo:rerun-if-changed=build.rs");

    let bytes = fs::read(&mermaid_path).unwrap_or_else(|e| panic!("read {mermaid_path:?}: {e}"));

    // Integrity check (SHA256).
    // Strip \r before hashing so the check is platform-independent: git may
    // convert LF to CRLF on Windows checkout without .gitattributes, which
    // would otherwise change the hash.
    let lf_bytes: Vec<u8> = bytes.iter().copied().filter(|&b| b != b'\r').collect();
    let actual_sha = format!("{:x}", Sha256::digest(&lf_bytes));
    if actual_sha != EXPECTED_MERMAID_SHA {
        panic!(
            "\n\n\
            [INTEGRITY ERROR] assets/mermaid.min.js does not match the expected SHA256 hash!\n\
            Expected: {EXPECTED_MERMAID_SHA}\n\
            Actual:   {actual_sha}\n\n\
            To update mermaid.js, use util/update-mermaid.sh <version>, which also\n\
            updates EXPECTED_MERMAID_SHA.\n\n"
        );
    }

    // The version is recorded by util/update-mermaid.sh from the npm
    // package.json; the minified bundle has no stable, structured version
    // marker to parse. The SHA256 check above pins the bundle bytes, and the
    // check below catches a version file left stale after an update.
    let version_path = PathBuf::from(&manifest_dir).join("assets/mermaid.version");
    let version = fs::read_to_string(&version_path)
        .unwrap_or_else(|e| panic!("read {version_path:?}: {e}"))
        .trim()
        .to_string();
    let content = String::from_utf8_lossy(&bytes);
    if !content.contains(&format!("\"{version}\"")) {
        panic!(
            "\n\n\
            [VERSION ERROR] assets/mermaid.version says {version:?}, but that version\n\
            string does not appear in assets/mermaid.min.js.\n\n\
            Update mermaid.js with util/update-mermaid.sh <version>.\n\n"
        );
    }

    println!("cargo:rustc-env=MERMAID_VERSION={version}");
}
