// ARCHIVED — NOT COMPILED. sekien's Linux setup without modifying the process
// environment (no `std::env::set_var`), kept as a starting point for when it
// becomes viable. Why it is not used: "Known issue" in src/linux_display.rs.
//
// Status (tested 2026-09 on the 0.4.3 code base, before the ES module page):
// - WebKitGTK 2.54 (Fedora 44): all E2E tests passed; output byte-identical.
// - WebKitGTK 2.50 (Ubuntu 22.04, CI): rendering aborted. WebKitWebProcess
//   inherits the environment and opens `$DISPLAY` itself ("cannot open
//   display"), which only an environment variable can reach. GTK's AT-SPI
//   bridge also printed a `dbind-WARNING` on stderr (no NO_AT_BRIDGE).
// - If another crate in the build enables tao's `x11` feature and `$DISPLAY`
//   is unset, tao segfaults: its device thread passes a NULL display to
//   XDefaultRootWindow (tao's device.rs; cf. tauri-apps/tao#1347).
// - Library users see Mesa's DRI3 warning on stderr unless they set
//   LIBGL_ALWAYS_SOFTWARE=1 before starting threads, as the CLI did.
//
// Wiring (replacing src/linux_display.rs, as this file's `mod linux`):
// - Cargo.toml: `wry = { version = "0.57", default-features = false,
//   features = ["os-webview"] }`, `tao = { version = "0.37",
//   default-features = false, features = ["rwh_06"] }` (no `x11`), and
//   `[target.'cfg(target_os = "linux")'.dependencies]` gtk = "0.18",
//   webkit2gtk = { version = "2.0", features = ["v2_16"] }, libc = "0.2" (the
//   versions tao/wry use).
// - render_stream: `linux::init()` before creating the event loop.
// - create_webview: on Linux, `linux::build_webview(builder, window, ...)`
//   instead of `builder.build(window)`. Since 0.5.1 the page is loaded from
//   the `sekien://` URL, not an HTML string: keep registering the custom
//   protocol on the builder, and load the URL (`load_url`) where this file
//   calls `load_html`, after changing the settings.
// - CLI `main`: set LIBGL_ALWAYS_SOFTWARE=1 before any thread exists (under
//   edition 2024, in `unsafe` with a SAFETY comment saying so).
// - tests/env_untouched.rs (`harness = false`, see the file next to this one)
//   asserts the environment is unchanged after render_stream.

//! Linux workarounds, kept in this one file so that they can be removed
//! together once they are no longer needed. [`init`] is to be called before
//! the event loop is created, and [`build_webview`] instead of wry's `build`.
//! This module depends on nothing else in the crate.
//!
//! Rendering happens on a private Xvfb display, and the process environment is
//! never modified (`std::env::set_var` is unsound once other threads exist,
//! and callers typically have spawned threads by then):
//!
//! - GTK is restricted to the X11 backend with `gdk_set_allowed_backends`
//!   (instead of `GDK_BACKEND=x11`). Off-screen window placement is not
//!   possible on Wayland, so X11 is used unconditionally.
//! - An internal Xvfb is always launched, regardless of any existing display,
//!   and GTK is pointed at it with `--display :N` in the `gtk_init_check` argv
//!   (instead of `$DISPLAY`). Rendering via Xwayland would cause a visible
//!   window flash; Xvfb has no screen and never flashes.
//! - Xvfb is launched with `-terminate`, so it exits automatically when
//!   sekien exits (when the last X client disconnects).
//! - tao/wry are built without their `x11` features (see `Cargo.toml`): with
//!   them, tao opens `$DISPLAY` itself instead of using the display GTK was
//!   initialised on. The WebView is therefore attached to the window's GTK
//!   container with `build_gtk`.
//! - GPU compositing is disabled via WebKitGTK settings (instead of
//!   `WEBKIT_DISABLE_COMPOSITING_MODE`): Xvfb has no GPU.
//!
//! sekien initialises GTK itself, the same way `gtk::init()` does, so that
//! tao's own `gtk::init()` call becomes a no-op. If GTK was already
//! initialised by the caller, initialisation (and Xvfb) is skipped and the
//! caller's display is used. (GTK's own `gtk_init_check` unsets
//! `DESKTOP_STARTUP_ID`, as it does in any GTK application.)
//!
//! Mesa prints a DRI3 warning to stderr when WebKitGTK initialises EGL on
//! Xvfb. Only an environment variable (`LIBGL_ALWAYS_SOFTWARE=1`) avoids it,
//! so avoiding it is left to the owner of the process, who can set it before
//! any threads exist.
//!
//! ## Keeping in sync with tao/wry
//!
//! The `gtk` and `webkit2gtk` crates used here must be the same versions tao
//! and wry depend on. Otherwise the GTK types are duplicated and the build
//! fails. Update them in `Cargo.toml` together whenever tao/wry are updated.
//!
//! ## Xvfb readiness detection
//!
//! Xvfb launched with `-displayfd <fd>` writes the chosen display number to
//! that fd once the X server is ready to accept clients. Polling for the socket
//! file alone is insufficient — GTK may attempt to connect before the server
//! is fully ready. Waiting for this signal is the correct approach
//! (xvfb-run uses the same technique).

use anyhow::{Context, Result, anyhow, bail};
use gtk::glib;
use std::ffi::{CString, c_char, c_int};
use std::io::{BufRead, BufReader, Read};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;
use tao::platform::unix::WindowExtUnix;
use tao::window::Window;
use webkit2gtk::{HardwareAccelerationPolicy, SettingsExt, WebViewExt};
use wry::{WebView, WebViewBuilder, WebViewBuilderExtUnix, WebViewExtUnix};

/// Launches Xvfb and initialises GTK on it. Must be called on the main thread
/// before the tao event loop is created.
pub fn init() -> Result<()> {
    if !is_main_thread() {
        bail!("the event loop must be created on the main thread");
    }
    if gtk::is_initialized() {
        return Ok(());
    }
    init_display().context("failed to initialize display")
}

fn init_display() -> Result<()> {
    let display = spawn_xvfb().context("failed to start Xvfb (install xvfb)")?;
    init_gtk(&display)
}

/// Builds `builder` into `window`'s GTK container and loads `html`.
///
/// WebKitGTK settings must be changed before the page is loaded, but wry
/// loads `with_html` content inside `build_gtk()`. So build without HTML,
/// configure, then load.
pub fn build_webview(builder: WebViewBuilder, window: &Window, html: &str) -> Result<WebView> {
    let vbox = window
        .default_vbox()
        .context("failed to get the window's GTK container")?;
    let webview = builder.build_gtk(vbox)?;
    if let Some(settings) = WebViewExt::settings(&webview.webview()) {
        settings.set_hardware_acceleration_policy(HardwareAccelerationPolicy::Never);
    }
    webview.load_html(html)?;
    Ok(webview)
}

/// Same check tao uses to decide whether it is running on the main thread.
fn is_main_thread() -> bool {
    // SAFETY: both syscalls have no preconditions.
    unsafe { libc::syscall(libc::SYS_gettid) == libc::getpid() as libc::c_long }
}

/// Initialises GTK on `display` (e.g. `":1"`), following the same steps as
/// `gtk::init()`.
fn init_gtk(display: &str) -> Result<()> {
    gtk::gdk::set_allowed_backends("x11");

    let args = [
        CString::new("sekien")?,
        CString::new("--display")?,
        CString::new(display)?,
    ];
    let mut argv: Vec<*mut c_char> = args.iter().map(|a| a.as_ptr().cast_mut()).collect();
    argv.push(std::ptr::null_mut());
    let mut argc = args.len() as c_int;
    let mut argv_ptr = argv.as_mut_ptr();

    // SAFETY: argc/argv describe a NULL-terminated array of valid C strings
    // that outlive the call; GTK only reorders/removes entries in `argv`.
    let ok = unsafe { gtk::ffi::gtk_init_check(&mut argc, &mut argv_ptr) };
    if ok == glib::ffi::GFALSE {
        bail!("failed to initialize GTK on Xvfb display {display}");
    }

    // SAFETY: GTK has been initialised above. gtk::init() also acquires (and
    // leaks) the default main context; see gtk-rs/gtk-rs-core#186.
    let acquired =
        unsafe { glib::ffi::g_main_context_acquire(glib::ffi::g_main_context_default()) };
    if acquired == glib::ffi::GFALSE {
        bail!("failed to acquire the default GLib main context");
    }

    // SAFETY: we initialised GTK ourselves just above, on this thread, which
    // is_main_thread() has confirmed is the process's main thread.
    unsafe { gtk::set_initialized() };
    Ok(())
}

/// Launches Xvfb and returns its display name (e.g. `":1"`) once it is ready.
fn spawn_xvfb() -> Result<String> {
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
    let (tx, rx) = mpsc::channel::<Result<String>>();
    std::thread::spawn(move || {
        let mut reader = BufReader::new(stdout);
        let mut line = String::new();
        let res = match reader.read_line(&mut line) {
            Ok(0) => Err(anyhow!(
                "Xvfb closed stdout before reporting display number"
            )),
            Ok(_) => display_name(&line),
            Err(e) => Err(e.into()),
        };
        let _ = tx.send(res);
        // Drain any remaining output so Xvfb does not block on a full pipe.
        let mut buf = Vec::new();
        let _ = reader.read_to_end(&mut buf);
    });

    rx.recv_timeout(Duration::from_secs(10))
        .unwrap_or_else(|_| Err(anyhow!("Xvfb did not become ready in time")))
        .inspect_err(|_| {
            let _ = child.kill();
        })
}

/// Turns the line Xvfb writes to its `-displayfd` (the display number) into a
/// display name, e.g. `"1\n"` into `":1"`.
fn display_name(line: &str) -> Result<String> {
    let number: u32 = line
        .trim()
        .parse()
        .context("parse display number from Xvfb")?;
    Ok(format!(":{number}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_name_from_displayfd_line() {
        assert_eq!(display_name("1\n").unwrap(), ":1");
        assert_eq!(display_name("42").unwrap(), ":42");
        assert!(display_name("").is_err());
        assert!(display_name(":1\n").is_err());
    }
}
