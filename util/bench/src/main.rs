//! sekien benchmark harness。
//!
//! 起動:
//!   cargo run --release --manifest-path bench/Cargo.toml
//!
//! 各 diagram (bench/diagrams/*.mmd) を warmup 3 + 計測 10 回回し、wall time
//! (`Instant`) と max RSS (`ps` / `pgrep` を別 thread で 50ms 間隔 sampling) の
//! median を出して markdown table で stdout に流す。
//!
//! 前提:
//! - sekien バイナリが `../target/release/sekien` に release build 済み
//!   (起動時に存在チェックして無ければ exit 1)
//! - 比較対象として `mmdc` が PATH にあれば自動で並べる。未インストールなら
//!   sekien 単体で計測する
//!
//! 依存: std のみ。RSS sampling は `ps -p PID -o rss=` と `pgrep -P PID` を
//! spawn する形で、macOS (BSD ps) と Linux (GNU ps / procps pgrep) の双方で動く。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

const WARMUP: usize = 3;
const RUNS: usize = 10;
/// 1 sample あたり ps を 1 回だけ叩く設計 (read_process_tree) に揃えたので、
/// 50ms → 10ms に下げて Xvfb (~200ms 寿命) の捕捉余裕を ~20 回に増やす。
const SAMPLE_INTERVAL: Duration = Duration::from_millis(10);

const DIAGRAMS: &[&str] = &["flowchart.mmd", "gitgraph.mmd", "sequence.mmd"];

struct BenchResult {
    median_duration: Duration,
    median_rss_kb: u64,
}

fn bench_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn default_sekien_path() -> PathBuf {
    bench_dir()
        .parent()
        .expect("bench/ should have a parent (util/)")
        .parent()
        .expect("util/ should have a parent (sekien crate root)")
        .join("target/release/sekien")
}

fn diagram_path(name: &str) -> PathBuf {
    bench_dir().join("diagrams").join(name)
}

fn puppeteer_config_path() -> PathBuf {
    bench_dir().join("puppeteer-config.json")
}

fn mmdc_available() -> bool {
    Command::new("mmdc")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// 1 回の `ps` snapshot。
///
/// `rss` は pid → RSS(KB)、`children` は ppid → 子 pid 群の reverse index。
/// 1 sample = ps 1 回 + parse 時に両 index を同時構築 + tree walk。reverse
/// index を per-sample で再構築しないため、SAMPLE_INTERVAL (10ms) に耐える。
struct ProcessTree {
    rss: HashMap<u32, u64>,
    children: HashMap<u32, Vec<u32>>,
}

/// `ps -ax -o pid,ppid,rss` を 1 回叩いて [`ProcessTree`] を作る。
///
/// 旧設計は `ps -p PID` + `pgrep -P PID` の再帰呼び出しで、子・孫が居る場合 1 sample
/// あたり最大数個のプロセスを spawn していた。1 sample = ps 1 回に統一する。
///
/// BSD ps (macOS) / GNU ps (Linux) の双方で `-ax -o pid=,ppid=,rss=` は同じ形式
/// (空白区切りの 3 列、ヘッダ無し) を返す。
fn read_process_tree() -> ProcessTree {
    let raw = run_ps().unwrap_or_default();
    let mut rss = HashMap::with_capacity(512);
    let mut children: HashMap<u32, Vec<u32>> = HashMap::with_capacity(512);
    for (pid, ppid, kb) in raw.lines().filter_map(parse_ps_line) {
        rss.insert(pid, kb);
        children.entry(ppid).or_default().push(pid);
    }
    ProcessTree { rss, children }
}

fn run_ps() -> Option<String> {
    let output = Command::new("ps")
        .args(["-ax", "-o", "pid=,ppid=,rss="])
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    String::from_utf8(output.stdout).ok()
}

fn parse_ps_line(line: &str) -> Option<(u32, u32, u64)> {
    let mut parts = line.split_whitespace();
    Some((
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
        parts.next()?.parse().ok()?,
    ))
}

/// `root_pid` とその子孫プロセス全部の RSS (KB) を合計する。
///
/// sekien は Linux で内部 Xvfb を子プロセスとして spawn するので、親 pid だけの
/// RSS では実際のメモリ使用量を取り逃がす。`tree.children` を使って root_pid
/// から DFS して合計する (O(descendants) per sample)。
fn tree_rss_kb(root_pid: u32, tree: &ProcessTree) -> u64 {
    let mut total = 0u64;
    let mut stack = vec![root_pid];
    while let Some(pid) = stack.pop() {
        if let Some(&kb) = tree.rss.get(&pid) {
            total += kb;
        }
        if let Some(kids) = tree.children.get(&pid) {
            stack.extend(kids);
        }
    }
    total
}

/// child を 1 回起動し、wall time と max RSS (子孫含む) を返す。
///
/// RSS sampling は別 thread で `SAMPLE_INTERVAL` 間隔。停止 signal は
/// `recv_timeout` の Ok(()) で受け取り、loop を抜けて max を返す。
fn measure_one_run(cmd: &mut Command) -> (Duration, u64) {
    let start = Instant::now();
    let mut child = cmd
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn child process");
    let pid = child.id();

    let (stop_tx, stop_rx) = mpsc::channel::<()>();
    let sampler = thread::spawn(move || {
        let mut max_rss = 0u64;
        loop {
            max_rss = max_rss.max(tree_rss_kb(pid, &read_process_tree()));
            if stop_rx.recv_timeout(SAMPLE_INTERVAL).is_ok() {
                break;
            }
        }
        max_rss
    });

    let _ = child.wait();
    let duration = start.elapsed();
    let _ = stop_tx.send(());
    let max_rss_kb = sampler.join().unwrap_or(0);
    (duration, max_rss_kb)
}

fn bench_target<F: Fn() -> Command>(label: &str, build_cmd: F) -> BenchResult {
    eprint!("    {label}: warmup");
    for _ in 0..WARMUP {
        let mut cmd = build_cmd();
        let _ = measure_one_run(&mut cmd);
        eprint!(".");
    }
    eprint!(" runs");
    let (durations, rsses): (Vec<Duration>, Vec<u64>) = (0..RUNS).map(|_| {
        let result = measure_one_run(&mut build_cmd());
        eprint!(".");
        result
    }).unzip();
    eprintln!();
    BenchResult {
        median_duration: median_duration(&durations),
        median_rss_kb: median_u64(&rsses),
    }
}

fn median_duration(samples: &[Duration]) -> Duration {
    let mut sorted: Vec<u128> = samples.iter().map(|d| d.as_nanos()).collect();
    sorted.sort();
    Duration::from_nanos(sorted[sorted.len() / 2] as u64)
}

fn median_u64(samples: &[u64]) -> u64 {
    let mut sorted = samples.to_vec();
    sorted.sort();
    sorted[sorted.len() / 2]
}

fn fmt_dur(d: Duration) -> String {
    let secs = d.as_secs_f64();
    if secs >= 1.0 {
        format!("{secs:.2} s")
    } else {
        format!("{} ms", d.as_millis())
    }
}

fn fmt_rss_kb(kb: u64) -> String {
    format!("{:.0} MB", kb as f64 / 1024.0)
}

fn make_sekien_cmd(sekien: &Path, diagram: &Path) -> Command {
    let mut cmd = Command::new(sekien);
    cmd.arg(diagram);
    cmd
}

fn make_mmdc_cmd(diagram: &Path, puppeteer_cfg: &Path, out: &Path) -> Command {
    let mut cmd = Command::new("mmdc");
    cmd.arg("-p").arg(puppeteer_cfg);
    cmd.arg("-i").arg(diagram);
    cmd.arg("-o").arg(out);
    cmd
}

fn main() {
    let sekien = default_sekien_path();
    if !sekien.exists() {
        eprintln!("Error: sekien binary not found at {sekien:?}");
        eprintln!("hint: cargo build --release --manifest-path ../Cargo.toml");
        std::process::exit(1);
    }

    let has_mmdc = mmdc_available();
    if !has_mmdc {
        eprintln!("note: mmdc not found in PATH; benchmarking sekien only.");
    }

    // mmdc の出力先 (sekien は stdout なのでファイル不要)
    let tmpdir = std::env::temp_dir().join(format!("sekien-bench-{}", std::process::id()));
    std::fs::create_dir_all(&tmpdir).expect("create tmpdir");
    let puppeteer_cfg = puppeteer_config_path();

    let mut rows: Vec<(String, BenchResult, Option<BenchResult>)> = Vec::new();
    for diagram_name in DIAGRAMS {
        eprintln!("Benchmarking {diagram_name} ...");
        let diagram = diagram_path(diagram_name);
        let sekien_result = bench_target("sekien", || make_sekien_cmd(&sekien, &diagram));
        let mmdc_result = if has_mmdc {
            let out = tmpdir.join(format!("{diagram_name}.svg"));
            Some(bench_target("mmdc  ", || {
                make_mmdc_cmd(&diagram, &puppeteer_cfg, &out)
            }))
        } else {
            None
        };
        rows.push((diagram_name.to_string(), sekien_result, mmdc_result));
    }
    let _ = std::fs::remove_dir_all(&tmpdir);

    println!();
    println!("# sekien benchmark results");
    println!(
        "_(warmup {WARMUP} runs, measurement {RUNS} runs, median over measurement runs)_",
    );
    println!();
    if has_mmdc {
        println!("| diagram | sekien (time) | mmdc (time) | sekien (RSS) | mmdc (RSS) |");
        println!("|---|---|---|---|---|");
        for (diagram, s, m) in &rows {
            let m = m.as_ref().expect("mmdc result present when has_mmdc");
            println!(
                "| {} | {} | {} | {} | {} |",
                diagram,
                fmt_dur(s.median_duration),
                fmt_dur(m.median_duration),
                fmt_rss_kb(s.median_rss_kb),
                fmt_rss_kb(m.median_rss_kb),
            );
        }
    } else {
        println!("| diagram | sekien (time) | sekien (RSS) |");
        println!("|---|---|---|");
        for (diagram, s, _) in &rows {
            println!(
                "| {} | {} | {} |",
                diagram,
                fmt_dur(s.median_duration),
                fmt_rss_kb(s.median_rss_kb),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synth_tree(edges: &[(u32, u32, u64)]) -> ProcessTree {
        // (pid, ppid, rss_kb) のリストから ProcessTree を作る test helper。
        // read_process_tree と同じ構造を構築するので、本番経路と等価。
        let mut rss = HashMap::new();
        let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
        for &(pid, ppid, kb) in edges {
            rss.insert(pid, kb);
            children.entry(ppid).or_default().push(pid);
        }
        ProcessTree { rss, children }
    }

    #[test]
    fn process_tree_includes_self() {
        let tree = read_process_tree();
        assert!(!tree.rss.is_empty(), "ps -ax returned no processes");
        let self_pid = std::process::id();
        assert!(tree.rss.contains_key(&self_pid), "self pid {self_pid} not in ps output");
        let kb = tree_rss_kb(self_pid, &tree);
        assert!(kb > 0, "expected non-zero RSS for self, got {kb}");
    }

    #[test]
    fn tree_rss_kb_sums_descendants() {
        // 合成 tree: 1 → 2 → 3、4 は別 root。1 から sum すると 1+2+3 のみ。
        let tree = synth_tree(&[(1, 0, 100), (2, 1, 200), (3, 2, 400), (4, 0, 800)]);
        assert_eq!(tree_rss_kb(1, &tree), 100 + 200 + 400);
        assert_eq!(tree_rss_kb(2, &tree), 200 + 400);
        assert_eq!(tree_rss_kb(3, &tree), 400);
        assert_eq!(tree_rss_kb(4, &tree), 800);
    }

    #[test]
    fn tree_rss_kb_unknown_root_is_zero() {
        let tree = synth_tree(&[]);
        assert_eq!(tree_rss_kb(999, &tree), 0);
    }

    #[test]
    fn parse_ps_line_typical() {
        assert_eq!(parse_ps_line("123 456 789"), Some((123, 456, 789)));
    }

    #[test]
    fn parse_ps_line_extra_whitespace() {
        assert_eq!(parse_ps_line("  123   456   789  "), Some((123, 456, 789)));
    }

    #[test]
    fn parse_ps_line_too_few_fields() {
        assert!(parse_ps_line("123 456").is_none());
    }

    #[test]
    fn parse_ps_line_non_numeric() {
        assert!(parse_ps_line("abc 456 789").is_none());
    }
}
