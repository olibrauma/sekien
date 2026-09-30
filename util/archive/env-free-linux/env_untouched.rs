// ARCHIVED — NOT COMPILED. Goes with linux.rs in this directory; see its header.
// Register in Cargo.toml as [[test]] name = "env_untouched", harness = false.

//! Checks that `render_stream` does not modify the process environment on
//! Linux (it used to set `DISPLAY`, `GDK_BACKEND`, etc.).
//!
//! `render_stream` must run on the main thread, but libtest runs tests on
//! worker threads, so this is a `harness = false` test with its own `main`.

#[cfg(target_os = "linux")]
fn main() {
    use sekien::{RenderOutcome, render_stream};

    // GTK itself consumes and unsets DESKTOP_STARTUP_ID in gtk_init (as it
    // does in any GTK application); that is not sekien's doing.
    let vars = || -> Vec<_> {
        std::env::vars_os()
            .filter(|(k, _)| k != "DESKTOP_STARTUP_ID")
            .collect()
    };
    let before = vars();

    let mut outcomes = Vec::new();
    render_stream(vec!["graph LR\n  A --> B".to_string()], None, |o| {
        outcomes.push(o)
    })
    .expect("render_stream failed");

    assert!(
        matches!(outcomes.as_slice(), [RenderOutcome::Svg(svg)] if svg.contains("<svg")),
        "unexpected outcomes: {outcomes:?}"
    );

    let after = vars();
    assert_eq!(before, after, "render_stream modified the environment");
    println!("env_untouched: ok");
}

#[cfg(not(target_os = "linux"))]
fn main() {}
