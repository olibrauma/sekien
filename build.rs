use sha2::{Digest, Sha256};
use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

/// Expected manifest hash of `assets/mermaid/` (see [`manifest_sha`]).
/// Guards against accidental or malicious modifications.
/// Rewritten by util/update-mermaid.sh.
const EXPECTED_MERMAID_SHA: &str =
    "71d17c484ce177e43637aa04de8d3f914cc428701614cb37ff0e23bbdff2b979";

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let mermaid_dir = manifest_dir.join("assets/mermaid");

    println!("cargo:rerun-if-changed=assets/mermaid");
    println!("cargo:rerun-if-changed=assets/mermaid.version");
    println!("cargo:rerun-if-changed=assets/render.html");
    println!("cargo:rerun-if-changed=build.rs");

    // mermaid's ESM build: relative paths (with `/`) in byte order.
    let mut paths = Vec::new();
    collect_mjs(&mermaid_dir, &mermaid_dir, &mut paths);
    paths.sort();
    assert!(
        paths.iter().any(|p| p == "mermaid.esm.min.mjs"),
        "assets/mermaid/mermaid.esm.min.mjs is missing; run util/update-mermaid.sh <version>"
    );
    let files: Vec<(String, Vec<u8>)> = paths
        .into_iter()
        .map(|p| {
            let path = mermaid_dir.join(&p);
            println!("cargo:rerun-if-changed={}", path.display());
            let bytes = fs::read(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"));
            (p, bytes)
        })
        .collect();

    // Integrity check.
    let actual_sha = manifest_sha(&files);
    if actual_sha != EXPECTED_MERMAID_SHA {
        panic!(
            "\n\n\
            [INTEGRITY ERROR] assets/mermaid/ does not match the expected manifest hash!\n\
            Expected: {EXPECTED_MERMAID_SHA}\n\
            Actual:   {actual_sha}\n\n\
            To update mermaid.js, use util/update-mermaid.sh <version>, which also\n\
            updates EXPECTED_MERMAID_SHA.\n\n"
        );
    }

    // The version is recorded by util/update-mermaid.sh from the npm
    // package.json; the minified bundle has no stable, structured version
    // marker to parse. The hash above pins the bytes, and the check below
    // catches a version file left stale after an update.
    let version_path = manifest_dir.join("assets/mermaid.version");
    let version = fs::read_to_string(&version_path)
        .unwrap_or_else(|e| panic!("read {version_path:?}: {e}"))
        .trim()
        .to_string();
    let needle = format!("\"{version}\"");
    if !files
        .iter()
        .any(|(_, b)| String::from_utf8_lossy(b).contains(&needle))
    {
        panic!(
            "\n\n\
            [VERSION ERROR] assets/mermaid.version says {version:?}, but that version\n\
            string does not appear in assets/mermaid/.\n\n\
            Update mermaid.js with util/update-mermaid.sh <version>.\n\n"
        );
    }
    println!("cargo:rustc-env=MERMAID_VERSION={version}");

    // Embed the files: a table sorted by path, for binary search.
    let mut table = String::from("&[\n");
    for (p, _) in &files {
        let abs = mermaid_dir.join(p);
        writeln!(
            table,
            "    ({p:?}, include_bytes!({:?})),",
            abs.display().to_string()
        )
        .unwrap();
    }
    table.push(']');
    let out = PathBuf::from(env::var("OUT_DIR").expect("OUT_DIR")).join("mermaid_files.rs");
    fs::write(&out, table).unwrap_or_else(|e| panic!("write {out:?}: {e}"));
}

/// Collects the `.mjs` files under `dir` as paths relative to `root`, with `/`.
fn collect_mjs(root: &Path, dir: &Path, out: &mut Vec<String>) {
    let entries = fs::read_dir(dir).unwrap_or_else(|e| panic!("read {dir:?}: {e}"));
    for entry in entries {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            collect_mjs(root, &path, out);
        } else if path.extension().is_some_and(|e| e == "mjs") {
            let rel = path.strip_prefix(root).expect("under root");
            let parts: Vec<_> = rel.iter().map(|c| c.to_string_lossy()).collect();
            out.push(parts.join("/"));
        }
    }
}

/// Manifest hash, as computed by util/update-mermaid.sh: for each file in
/// byte order of its path, "<sha256 of the content with \r stripped>  <path>\n",
/// then the sha256 of all those lines. `\r` is stripped so that the check does
/// not depend on line endings in a Windows checkout.
fn manifest_sha(files: &[(String, Vec<u8>)]) -> String {
    let mut lines = String::new();
    for (p, bytes) in files {
        let lf: Vec<u8> = bytes.iter().copied().filter(|&b| b != b'\r').collect();
        writeln!(lines, "{:x}  {p}", Sha256::digest(&lf)).unwrap();
    }
    format!("{:x}", Sha256::digest(lines.as_bytes()))
}
