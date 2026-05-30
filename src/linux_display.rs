//! Resolves the display backend on Linux before WebView initialisation.
//!
//! - Forces `GDK_BACKEND=x11`. Off-screen window placement is not possible on
//!   Wayland, so X11 (including Xwayland) is used unconditionally.
//! - Always launches an internal Xvfb and overwrites `$DISPLAY`, regardless of
//!   any existing display. Rendering via Xwayland would cause a visible window
//!   flash; Xvfb has no screen and never flashes.
//! - Xvfb is launched with `-terminate`, so it exits automatically when
//!   sekien exits (when the last X client disconnects).
//!
//! ## Xvfb readiness detection
//!
//! Xvfb launched with `-displayfd <fd>` writes the chosen display number to
//! that fd once the X server is ready to accept clients. Polling for the socket
//! file alone is insufficient — GTK may attempt to connect before the server
//! is fully ready. Waiting for this signal is the correct approach
//! (xvfb-run uses the same technique).

use anyhow::{anyhow, Context, Result};
use std::io::{BufRead, BufReader, Read};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

/// Resolves the display backend. Call before GTK initialisation.
///
/// Side effects: sets `GDK_BACKEND=x11`, launches Xvfb, and overwrites
/// `$DISPLAY` (any pre-existing display is ignored).
pub fn ensure_display() -> Result<()> {
    std::env::set_var("GDK_BACKEND", "x11");
    // Disable GPU compositing and force software rendering to suppress libEGL warnings.
    std::env::set_var("WEBKIT_DISABLE_COMPOSITING_MODE", "1");
    std::env::set_var("LIBGL_ALWAYS_SOFTWARE", "1");
    // Disable AT-SPI accessibility bus connection attempts.
    // Without this, GTK emits a dbind-WARNING to stderr in headless environments (e.g. CI).
    std::env::set_var("NO_AT_BRIDGE", "1");

    spawn_xvfb().context("failed to start Xvfb (install xvfb)")?;
    Ok(())
}

fn spawn_xvfb() -> Result<()> {
    let mut child = Command::new("Xvfb")
        .arg("-displayfd")
        .arg("1")
        .arg("-screen")
        .arg("0")
        .arg("100x100x24")
        .arg("-nolisten")
        .arg("tcp")
        .arg("-terminate")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("spawn Xvfb")?;

    let stdout = child.stdout.take().expect("piped stdout");

    // Read stdout on a separate thread so we can apply a timeout to the blocking read_line.
    let (tx, rx) = mpsc::channel::<Result<u32>>();
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();
        let res = match reader.read_line(&mut line) {
            Ok(0) => Err(anyhow!(
                "Xvfb closed stdout before reporting display number"
            )),
            Ok(_) => line
                .trim()
                .parse::<u32>()
                .context("parse display number from Xvfb"),
            Err(e) => Err(e.into()),
        };
        let _ = tx.send(res);
        // Drain any remaining output so Xvfb does not block on a full pipe.
        let mut buf = Vec::new();
        let _ = reader.read_to_end(&mut buf);
    });

    let display_num = rx
        .recv_timeout(Duration::from_secs(10))
        .unwrap_or_else(|_| Err(anyhow!("Xvfb did not become ready in time")))
        .inspect_err(|_| {
            let _ = child.kill();
        })?;
    std::env::set_var("DISPLAY", format!(":{display_num}"));
    Ok(())
}
