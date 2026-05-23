//! sekien CLI 統合テスト。
//!
//! 通常は `cargo test` で実行 (Cargo が用意する debug build バイナリを
//! `env!("CARGO_BIN_EXE_sekien")` 経由で使う)。release build を検証したい
//! ときは `SEKIEN_TEST_BIN` env で上書きできる:
//!
//! ```bash
//! cargo build --release
//! SEKIEN_TEST_BIN=$(pwd)/target/release/sekien cargo test --test integration
//! ```

use std::ffi::OsString;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};

fn sekien() -> OsString {
    std::env::var_os("SEKIEN_TEST_BIN").unwrap_or_else(|| env!("CARGO_BIN_EXE_sekien").into())
}

fn cmd() -> Command {
    Command::new(sekien())
}

fn run_stdin(input: &[u8]) -> Output {
    let mut child = cmd()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn sekien");
    child
        .stdin
        .as_mut()
        .expect("piped stdin")
        .write_all(input)
        .expect("write stdin");
    child.wait_with_output().expect("wait sekien")
}

fn run_args(args: &[&str]) -> Output {
    cmd().args(args).output().expect("run sekien")
}

/// テスト用 tempfile を `${TMPDIR}/sekien_test_<pid>_<n>.mmd` に書き出す。
/// `Drop` で削除されるので test の早期 panic でも残らない。
struct TempMmd(PathBuf);

impl TempMmd {
    fn new(content: &str) -> Self {
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        let pid = std::process::id();
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let path = std::env::temp_dir().join(format!("sekien_test_{pid}_{n}.mmd"));
        std::fs::write(&path, content).expect("write tempfile");
        Self(path)
    }
    fn path(&self) -> &PathBuf {
        &self.0
    }
}

impl Drop for TempMmd {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

fn count_subseq(haystack: &[u8], needle: &[u8]) -> usize {
    if needle.is_empty() || haystack.len() < needle.len() {
        return 0;
    }
    let mut count = 0;
    let mut i = 0;
    while i + needle.len() <= haystack.len() {
        if &haystack[i..i + needle.len()] == needle {
            count += 1;
            i += needle.len();
        } else {
            i += 1;
        }
    }
    count
}

fn find_subseq(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn assert_has_svg(stdout: &[u8]) {
    assert!(
        count_subseq(stdout, b"<svg") > 0,
        "stdout does not contain <svg: {:?}",
        String::from_utf8_lossy(stdout)
    );
}

#[test]
fn mmd_file_arg_produces_svg() {
    let tmp = TempMmd::new("graph LR\n  A --> B\n");
    let out = cmd().arg(tmp.path()).output().expect("run sekien");
    assert!(out.status.success(), "exit: {:?}", out.status);
    assert_has_svg(&out.stdout);
    assert!(
        count_subseq(&out.stdout, b"tspan") > 0,
        "stdout missing tspan"
    );
}

#[test]
fn stdin_single_block_produces_svg() {
    let out = run_stdin(b"graph LR\n  A --> B\n");
    assert!(out.status.success());
    assert_has_svg(&out.stdout);
}

#[test]
fn stdin_multi_block_produces_separated_svgs() {
    let out = run_stdin(b"graph LR\n  A --> B\0graph TD\n  X --> Y");
    assert!(out.status.success());
    let svg_count = count_subseq(&out.stdout, b"<svg");
    let nul_count = out.stdout.iter().filter(|&&b| b == 0).count();
    assert_eq!(svg_count, 2, "expected 2 SVGs, got {svg_count}");
    assert_eq!(nul_count, 1, "expected 1 NUL separator, got {nul_count}");
}

#[test]
fn multi_block_order_is_preserved() {
    let input = b"graph LR\n  Apple --> Banana\0graph LR\n  Cat --> Dog\0graph LR\n  Eagle --> Falcon";
    let out = run_stdin(input);
    assert!(out.status.success());
    let p1 = find_subseq(&out.stdout, b"Apple").expect("Apple not found in stdout");
    let p2 = find_subseq(&out.stdout, b"Cat").expect("Cat not found in stdout");
    let p3 = find_subseq(&out.stdout, b"Eagle").expect("Eagle not found in stdout");
    assert!(p1 < p2, "order broken: Apple@{p1} >= Cat@{p2}");
    assert!(p2 < p3, "order broken: Cat@{p2} >= Eagle@{p3}");
}

#[test]
fn invalid_mermaid_emits_stderr_line_and_exits_zero() {
    let out = run_stdin(b"thisIsNotAValidMermaidDiagramType");
    assert!(out.status.success(), "expected exit 0, got {:?}", out.status);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("mermaid block 1"),
        "stderr missing 'mermaid block 1' line: {stderr}"
    );
}

#[test]
fn version_flag_mentions_sekien_and_mermaid() {
    let out = run_args(&["--version"]);
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("sekien"), "no 'sekien' in --version: {stdout}");
    assert!(stdout.contains("mermaid"), "no 'mermaid' in --version: {stdout}");
}

#[test]
fn help_flag_exits_zero() {
    let out = run_args(&["--help"]);
    assert!(out.status.success());
}

#[test]
fn font_flag_produces_svg() {
    let tmp = TempMmd::new("graph LR\n  A --> B\n");
    let out = cmd()
        .args(["--font", "Arial"])
        .arg(tmp.path())
        .output()
        .expect("run sekien");
    assert!(out.status.success());
    assert_has_svg(&out.stdout);
}

#[test]
fn theme_flag_produces_svg() {
    let tmp = TempMmd::new("graph LR\n  A --> B\n");
    let out = cmd()
        .args(["--theme", "dark"])
        .arg(tmp.path())
        .output()
        .expect("run sekien");
    assert!(out.status.success());
    assert_has_svg(&out.stdout);
}

#[test]
fn look_flag_produces_svg() {
    let tmp = TempMmd::new("graph LR\n  A --> B\n");
    let out = cmd()
        .args(["--look", "classic"])
        .arg(tmp.path())
        .output()
        .expect("run sekien");
    assert!(out.status.success());
    assert_has_svg(&out.stdout);
}

#[test]
fn sekien_font_env_produces_svg() {
    let tmp = TempMmd::new("graph LR\n  A --> B\n");
    let out = cmd()
        .env("SEKIEN_FONT", "Arial")
        .arg(tmp.path())
        .output()
        .expect("run sekien");
    assert!(out.status.success());
    assert_has_svg(&out.stdout);
}

#[test]
fn multiple_files_is_error() {
    let p1 = TempMmd::new("graph LR\n  A --> B\n");
    let p2 = TempMmd::new("graph TD\n  X --> Y\n");
    let out = cmd()
        .arg(p1.path())
        .arg(p2.path())
        .output()
        .expect("run sekien");
    assert!(
        !out.status.success(),
        "expected non-zero exit for multiple files, got {:?}",
        out.status
    );
}

#[test]
fn continue_on_error_three_blocks_middle_fails() {
    let input = b"graph LR\n  A --> B\0totallyBogusDiagram\0graph TD\n  X --> Y";
    let out = run_stdin(input);
    assert!(out.status.success(), "expected exit 0, got {:?}", out.status);
    let svg_count = count_subseq(&out.stdout, b"<svg");
    let nul_count = out.stdout.iter().filter(|&&b| b == 0).count();
    assert_eq!(svg_count, 2, "expected 2 SVGs, got {svg_count}");
    assert_eq!(nul_count, 1, "expected 1 NUL separator, got {nul_count}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("mermaid block 2"),
        "stderr missing 'mermaid block 2': {stderr}"
    );
}

#[test]
fn all_blocks_fail_yields_empty_stdout_and_three_stderr_lines() {
    let input = b"bogusOne\0bogusTwo\0bogusThree";
    let out = run_stdin(input);
    assert!(out.status.success(), "expected exit 0, got {:?}", out.status);
    assert!(
        out.stdout.is_empty(),
        "stdout should be empty, got {} bytes",
        out.stdout.len()
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    let lines = stderr.lines().filter(|l| l.contains("mermaid block")).count();
    assert_eq!(lines, 3, "expected 3 error lines, got {lines}");
}

#[test]
fn trailing_whitespace_after_nul_is_ignored() {
    let out = run_stdin(b"graph LR\n  A --> B\0\n");
    assert!(out.status.success());
    let svg_count = count_subseq(&out.stdout, b"<svg");
    assert_eq!(svg_count, 1, "expected 1 SVG, got {svg_count}");
    assert!(
        out.stderr.is_empty(),
        "stderr should be empty, got: {:?}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn trailing_nul_is_ignored() {
    let out = run_stdin(b"graph LR\n  A --> B\0");
    assert!(out.status.success());
    let svg_count = count_subseq(&out.stdout, b"<svg");
    assert_eq!(svg_count, 1, "expected 1 SVG, got {svg_count}");
    assert!(
        out.stderr.is_empty(),
        "stderr should be empty, got: {:?}",
        String::from_utf8_lossy(&out.stderr)
    );
}
