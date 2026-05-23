//! sekien バイナリと sekien-api の契約を検証する integration test。
//!
//! sekien バイナリの実体を spawn するので、`SEKIEN_TEST_BIN` 環境変数で path を
//! 指定する。未指定のときは各 test は早期 return して skip される (`cargo test`
//! は pass 扱い、`(skip)` メッセージを出力)。
//!
//! ```bash
//! cargo build --manifest-path ../../Cargo.toml --release
//! SEKIEN_TEST_BIN=$(pwd)/../../target/release/sekien cargo test --test e2e
//! ```
//!
//! sekien バイナリ自体の挙動を検証するものではなく、`render_blocks` /
//! `mermaid_version` が sekien との protocol contract を守っているか
//! (順序保持、per-block 失敗の位置、空 block 混在、stderr parse 等) を
//! 固定化する。

use std::ffi::OsString;
use sekien_api::{mermaid_version, render_blocks, BlockOutcome, RenderConfig, SekienApiError};

/// `SEKIEN_TEST_BIN` env を読み、未指定なら `None` を返す。
/// caller (各 test) は `None` の場合早期 return することで skip する。
fn sekien_bin() -> Option<OsString> {
    std::env::var_os("SEKIEN_TEST_BIN")
}

/// 各 test の冒頭で呼ぶ skip helper。
/// SEKIEN_TEST_BIN 未指定なら `(skip)` を stdout に出して early return する。
macro_rules! sekien_or_skip {
    () => {
        match sekien_bin() {
            Some(p) => p,
            None => {
                println!("(skip) set SEKIEN_TEST_BIN to enable this test");
                return;
            }
        }
    };
}

#[test]
fn round_trip_single_block() {
    let sekien = sekien_or_skip!();
    let outcomes = render_blocks(
        &sekien,
        vec!["graph LR\n  A --> B".to_string()],
        &RenderConfig::default(),
    )
    .expect("render_blocks should succeed");
    assert_eq!(outcomes.len(), 1);
    match &outcomes[0] {
        BlockOutcome::Rendered(svg) => assert!(svg.contains("<svg"), "expected SVG content"),
        BlockOutcome::Failed(msg) => panic!("expected Rendered, got Failed: {msg}"),
    }
}

#[test]
fn render_blocks_preserves_order() {
    let sekien = sekien_or_skip!();
    let outcomes = render_blocks(
        &sekien,
        vec![
            "graph LR\n  Apple --> Banana".to_string(),
            "graph LR\n  Cat --> Dog".to_string(),
            "graph LR\n  Eagle --> Falcon".to_string(),
        ],
        &RenderConfig::default(),
    )
    .expect("render_blocks should succeed");
    assert_eq!(outcomes.len(), 3);
    for (i, (a, b)) in [("Apple", "Banana"), ("Cat", "Dog"), ("Eagle", "Falcon")]
        .iter()
        .enumerate()
    {
        match &outcomes[i] {
            BlockOutcome::Rendered(svg) => {
                assert!(svg.contains(a), "block {i} SVG missing {a:?}");
                assert!(svg.contains(b), "block {i} SVG missing {b:?}");
            }
            BlockOutcome::Failed(msg) => panic!("block {i} unexpectedly failed: {msg}"),
        }
    }
}

#[test]
fn partial_failure_preserves_position() {
    let sekien = sekien_or_skip!();
    let outcomes = render_blocks(
        &sekien,
        vec![
            "graph LR\n  A --> B".to_string(),
            "totallyNotAMermaidDiagram".to_string(),
            "graph TD\n  X --> Y".to_string(),
        ],
        &RenderConfig::default(),
    )
    .expect("per-block failure should not Err the whole call");
    assert_eq!(outcomes.len(), 3);
    assert!(matches!(outcomes[0], BlockOutcome::Rendered(_)), "block 1: Rendered");
    assert!(matches!(outcomes[1], BlockOutcome::Failed(_)), "block 2: Failed");
    assert!(matches!(outcomes[2], BlockOutcome::Rendered(_)), "block 3: Rendered");
}

#[test]
fn empty_block_in_input_does_not_shift_positions() {
    // sekien protocol の trailing \0 1 個 drop に正しく乗っているかを固定化する。
    // 過去の bug: ["m1", "", "m2"] を渡すと末尾 \0 を書かない実装が "m2\0" を欠落させ、
    // sekien 側で 2 blocks としか解釈されず ProtocolViolation で Err していた。
    // 修正後は 3 blocks として認識し、空 block は mermaid 側で Failed になる。
    let sekien = sekien_or_skip!();
    let outcomes = render_blocks(
        &sekien,
        vec![
            "graph LR\n  A --> B".to_string(),
            String::new(),
            "graph TD\n  X --> Y".to_string(),
        ],
        &RenderConfig::default(),
    )
    .expect("blocks containing empty string should not Err");
    assert_eq!(
        outcomes.len(),
        3,
        "outcomes.len() must equal blocks.len() even when empty block is included"
    );
    assert!(matches!(outcomes[0], BlockOutcome::Rendered(_)), "block 1: Rendered");
    assert!(matches!(outcomes[1], BlockOutcome::Failed(_)), "block 2: empty → Failed");
    assert!(matches!(outcomes[2], BlockOutcome::Rendered(_)), "block 3: Rendered");
}

#[test]
fn mermaid_version_returns_semver_like_string() {
    let sekien = sekien_or_skip!();
    let v = mermaid_version(&sekien).expect("mermaid_version should succeed");
    // `X.Y.Z` または `X.Y.Z-<prerelease>` の形を期待
    let core = v.split('-').next().expect("split always yields at least one item");
    let parts: Vec<&str> = core.split('.').collect();
    assert_eq!(parts.len(), 3, "expected semver X.Y.Z, got {v:?}");
    for p in &parts {
        assert!(p.parse::<u32>().is_ok(), "non-numeric semver component in {v:?}");
    }
}

#[test]
fn spawn_failure_returns_not_found() {
    // この test は sekien バイナリの実体を必要としないので、SEKIEN_TEST_BIN 未指定でも走る
    let err = render_blocks(
        "/nonexistent/sekien-binary-xyz-please-do-not-create-this",
        vec!["graph LR\n  A --> B".to_string()],
        &RenderConfig::default(),
    )
    .expect_err("should fail to spawn nonexistent binary");
    match err {
        SekienApiError::Spawn { source, .. } => {
            assert_eq!(source.kind(), std::io::ErrorKind::NotFound);
        }
        other => panic!("expected Spawn variant, got: {other:?}"),
    }
}
