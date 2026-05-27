//! sekien benchmark harness。
//!
//! 起動:
//!   cargo run --release --manifest-path bench/Cargo.toml
//!
//! 各 diagram (bench/diagrams/*.mmd) を warmup 3 + 計測 10 回回し、wall time
//! (`Instant`) と max RSS (/proc/pid/statm または ps) の median を出して 
//! markdown table で stdout に流す。

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

const WARMUP: usize = 3;
const RUNS: usize = 10;
/// 10ms 間隔でサンプリングし、短寿命な Xvfb も捕捉する。
const SAMPLE_INTERVAL: Duration = Duration::from_millis(10);

const DIAGRAMS: &[&str] = &["flowchart.mmd", "gitgraph.mmd", "sequence.mmd"];

struct BenchResult {
    median_duration: Duration,
    median_rss_bytes: u64,
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

#[cfg(target_os = "linux")]
fn get_rss_bytes(pid: u32) -> u64 {
    // statm: size resident shared text lib data dirty
    let Ok(content) = std::fs::read_to_string(format!("/proc/{pid}/statm")) else { return 0; };
    let pages: u64 = content.split_whitespace().nth(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    pages * 4096 // Linux default page size
}

#[cfg(target_os = "linux")]
fn get_process_group_rss(root_pid: u32) -> u64 {
    // 指定したプロセスとその子孫全ての RSS を合算。
    // procfs を走査して PPID を辿る。
    let mut total = 0;
    let mut ppid_map: HashMap<u32, Vec<u32>> = HashMap::new();
    let mut rss_map: HashMap<u32, u64> = HashMap::new();

    if let Ok(entries) = std::fs::read_dir("/proc") {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(pid) = name.to_str().and_then(|s| s.parse::<u32>().ok()) else { continue; };
            
            if let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) {
                let parts: Vec<&str> = stat.split_whitespace().collect();
                if parts.len() > 3 {
                    let ppid: u32 = parts[3].parse().unwrap_or(0);
                    ppid_map.entry(ppid).or_default().push(pid);
                    rss_map.insert(pid, get_rss_bytes(pid));
                }
            }
        }
    }

    let mut stack = vec![root_pid];
    while let Some(pid) = stack.pop() {
        total += rss_map.get(&pid).cloned().unwrap_or(0);
        if let Some(children) = ppid_map.get(&pid) {
            stack.extend(children);
        }
    }
    total
}

#[cfg(not(target_os = "linux"))]
fn get_process_group_rss(root_pid: u32) -> u64 {
    // macOS 等では ps コマンドに頼る
    let output = Command::new("ps")
        .args(["-ax", "-o", "pid=,ppid=,rss="])
        .output()
        .ok();
    
    let mut total = 0;
    if let Some(o) = output {
        let raw = String::from_utf8_lossy(&o.stdout);
        let mut ppid_map: HashMap<u32, Vec<u32>> = HashMap::new();
        let mut rss_map: HashMap<u32, u64> = HashMap::new();

        for line in raw.lines() {
            let parts: Vec<u32> = line.split_whitespace().filter_map(|s| s.parse().ok()).collect();
            if parts.len() == 3 {
                let pid = parts[0];
                let ppid = parts[1];
                let rss_kb = parts[2] as u64;
                ppid_map.entry(ppid).or_default().push(pid);
                rss_map.insert(pid, rss_kb * 1024);
            }
        }

        let mut stack = vec![root_pid];
        while let Some(pid) = stack.pop() {
            total += rss_map.get(&pid).cloned().unwrap_or(0);
            if let Some(children) = ppid_map.get(&pid) {
                stack.extend(children);
            }
        }
    }
    total
}

#[derive(Clone, Copy)]
enum MeasureMode {
    TimeOnly,
    RssOnly,
}

/// child を 1 回起動し、指定されたモードで計測を行う。
fn measure_one_run(cmd: &mut Command, mode: MeasureMode) -> u64 {
    let start = Instant::now();
    let mut child = cmd
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn child process");
    let pid = child.id();

    let (stop_tx, stop_rx) = mpsc::channel::<()>();
    let sampler = if matches!(mode, MeasureMode::RssOnly) {
        Some(thread::spawn(move || {
            let mut max_rss = 0u64;
            loop {
                max_rss = max_rss.max(get_process_group_rss(pid));
                if stop_rx.recv_timeout(SAMPLE_INTERVAL).is_ok() {
                    break;
                }
            }
            max_rss
        }))
    } else {
        None
    };

    let _ = child.wait();
    let duration = start.elapsed();
    let _ = stop_tx.send(());

    match mode {
        MeasureMode::TimeOnly => duration.as_nanos() as u64,
        MeasureMode::RssOnly => sampler.and_then(|s| s.join().ok()).unwrap_or(0),
    }
}

fn bench_target<F: Fn() -> Command>(label: &str, build_cmd: F) -> BenchResult {
    eprint!("    {label}: warmup");
    for _ in 0..WARMUP {
        let _ = measure_one_run(&mut build_cmd(), MeasureMode::TimeOnly);
        eprint!(".");
    }
    
    eprint!(" time_runs");
    let durations: Vec<u64> = (0..RUNS).map(|_| {
        let res = measure_one_run(&mut build_cmd(), MeasureMode::TimeOnly);
        eprint!(".");
        res
    }).collect();

    eprint!(" rss_runs");
    let rsses: Vec<u64> = (0..RUNS).map(|_| {
        let res = measure_one_run(&mut build_cmd(), MeasureMode::RssOnly);
        eprint!(".");
        res
    }).collect();
    
    eprintln!();
    BenchResult {
        median_duration: Duration::from_nanos(median_u64(&durations)),
        median_rss_bytes: median_u64(&rsses),
    }
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

fn fmt_rss_bytes(bytes: u64) -> String {
    format!("{:.0} MB", bytes as f64 / 1024.0 / 1024.0)
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
    println!("_(warmup {WARMUP} runs, measurement {RUNS} runs, median over measurement runs)_");
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
                fmt_rss_bytes(s.median_rss_bytes),
                fmt_rss_bytes(m.median_rss_bytes),
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
                fmt_rss_bytes(s.median_rss_bytes),
            );
        }
    }
}
