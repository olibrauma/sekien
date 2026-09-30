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
//! ## Known issue: the process environment is modified
//!
//! [`ensure_display`] sets five environment variables with `set_var`, which
//! is unsound once other threads exist (another thread's `getenv`, e.g. in
//! GLib or glibc, can read memory freed by `setenv`). Threads do exist at that
//! point: the Xvfb stdout reader below, the CLI's stdin reader, and whatever
//! the host of the library has spawned. This is why the crate stays on edition
//! 2021: edition 2024 makes `set_var` `unsafe`, and no honest `SAFETY`
//! comment can be written for these calls.
//!
//! Why each variable (as of WebKitGTK 2.50/2.54, tao 0.37):
//!
//! | Variable | Read by | Without it |
//! |---|---|---|
//! | `DISPLAY` | GTK; tao's x11 device thread (`XOpenDisplay(NULL)`); WebKitGTK's child processes, which inherit the environment | WebKitGTK 2.50 (e.g. Ubuntu 22.04): `WebKitWebProcess` fails with "cannot open display" and rendering aborts. tao segfaults if `$DISPLAY` is unset (tao bug, cf. tauri-apps/tao#1347). |
//! | `NO_AT_BRIDGE` | GTK's AT-SPI bridge | `dbind-WARNING` on stderr where there is no accessibility bus (e.g. CI), which breaks the CLI's clean stderr |
//! | `LIBGL_ALWAYS_SOFTWARE` | Mesa | Mesa's DRI3 warning on stderr (Xvfb has no DRI3) |
//! | `GDK_BACKEND` | GDK | GDK may prefer `$WAYLAND_DISPLAY` over Xvfb |
//! | `WEBKIT_DISABLE_COMPOSITING_MODE` | WebKitGTK | GPU compositing, which Xvfb cannot provide |
//!
//! The first three were observed; the last two are the documented intent.
//! Only an environment variable can reach child processes, and none of these
//! has an API alternative that does.
//!
//! Alternatives considered:
//!
//! - **API-only setup** (`gtk_init_check` with `--display :N`,
//!   `gdk_set_allowed_backends`, WebKitGTK settings, tao/wry without `x11`):
//!   worked with WebKitGTK 2.54 but not 2.50 (child processes still need
//!   `$DISPLAY`), still printed the AT-SPI warning, and made programs that
//!   enable tao's `x11` feature elsewhere crash without a display. Rejected
//!   for now; the implementation is kept in `util/archive/env-free-linux/`.
//! - **The CLI sets the variables** at the start of `main`, before any thread
//!   exists (sound), and the library requires the host to provide `$DISPLAY`
//!   (e.g. `xvfb-run`). Sound, but burdens library users. Not adopted yet.
//! - **Set them only while single-threaded** (`/proc/self/task`): sound, but
//!   does nothing for hosts that already run threads (e.g. tokio).
//!
//! Decision: keep setting the variables (behaviour unchanged from 0.4.2) and
//! document the side effect and its precondition on
//! [`crate::render_stream`].
//!
//! When to revisit: once the minimum supported WebKitGTK no longer needs
//! `$DISPLAY` in its child processes (2.54 does not, 2.50 does), the API-only
//! setup becomes the leading option; it also unblocks edition 2024. The
//! remaining issues listed above (AT-SPI warning, tao's `x11` feature) would
//! still need an answer.
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
