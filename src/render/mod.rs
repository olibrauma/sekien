//! Streaming Mermaid → SVG renderer using wry (WebView) and tao (event loop).
//!
//! ## Architecture
//!
//! [`render_stream`] is split into a pure core and an impure shell:
//!
//! - [`Collector`] (in `collector.rs`) is a pure state machine: given an
//!   [`Input`] (a new diagram, end-of-input, an IPC message from the WebView,
//!   or the window closing), it returns the [`Action`]s that should happen
//!   next. It assigns each diagram its id and parses IPC, and does not touch
//!   the WebView, the event loop, or any I/O, so it can be unit tested directly.
//! - `html.rs` is equally pure: it builds the page and the script that asks
//!   the page to render a diagram.
//! - [`render_stream`] is the thin impure shell: it owns the window, WebView
//!   and event loop, feeds [`Input`]s into the [`Collector`], and executes the
//!   [`Action`]s it returns (evaluating a render script, calling `on_result`,
//!   or exiting the loop).
//!
//! ```text
//! [feeder thread]             [event loop]                      [WebView / mermaid.js]
//!   |                              |                                  |
//!   |-- Block(...) --------------->|-- Collector::handle ------------>|
//!   |                              |     -> Action::Dispatch(1) ----->|-- evaluate_script
//!   |-- Block(...) --------------->|-- Collector::handle (queued)     |
//!   |-- End ---------------------->|-- Collector::handle              |
//!   |                              |<-- Ipc: svg/error 1 -------------|
//!   |                              |-- Collector::handle              |
//!   |                              |     -> Action::Emit(...) ---- on_result(...)
//!   |                              |     -> Action::Dispatch(2) ----->|-- evaluate_script
//!   |                              |<-- Ipc: svg/error 2 -------------|
//!   |                              |     -> Action::Emit(...) ---- on_result(...)
//!   |                              |     -> Action::Done -> loop exits (run_return)
//! ```
//!
//! `render_stream` dispatches at most one render at a time (mermaid.render is not
//! parallelisable), in input order. `on_result` is therefore called exactly once
//! per diagram, in the order of `diagrams`.

mod collector;
mod html;

pub use collector::RenderOutcome;

use crate::error::{Error, Result};
use collector::{Action, Collector, Input};
use tao::{
    dpi::{LogicalPosition, LogicalSize},
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy, EventLoopWindowTarget},
    platform::run_return::EventLoopExtRunReturn,
    window::{Window, WindowBuilder},
};
use wry::{WebView, WebViewBuilder};

#[cfg(target_os = "linux")]
use crate::linux_display;

/// Version string extracted from `mermaid.min.js` by `build.rs` at compile time.
pub const MERMAID_VERSION: &str = env!("MERMAID_VERSION");

fn create_window(event_loop: &EventLoopWindowTarget<Input>) -> Result<Window> {
    // Placed off-screen so that it is never visible (on Linux it lives on a
    // private Xvfb display anyway). 1x1 triggers a GDK assertion on Linux.
    let size = if cfg!(target_os = "linux") { 100 } else { 1 };
    let off_screen = LogicalPosition::new(-10000, -10000);
    let window = WindowBuilder::new()
        .with_transparent(true)
        .with_decorations(false)
        .with_always_on_top(false)
        .with_visible(true)
        .with_inner_size(LogicalSize::new(size, size))
        .with_position(off_screen)
        .build(event_loop)
        .map_err(|e| Error::Internal(format!("failed to create window: {e}")))?;
    // Set again after creation: window managers may ignore or adjust the
    // initial position.
    window.set_outer_position(off_screen);
    Ok(window)
}

fn create_webview(window: &Window, html: String, proxy: EventLoopProxy<Input>) -> Result<WebView> {
    WebViewBuilder::new()
        .with_background_color((0, 0, 0, 0))
        .with_transparent(true)
        .with_html(html)
        .with_ipc_handler(move |req| {
            let _ = proxy.send_event(Input::Ipc(req.into_body()));
        })
        .build(window)
        .map_err(|e| Error::Internal(format!("failed to create webview: {e}")))
}

/// Renders each diagram in `diagrams` to SVG, calling `on_result(outcome)` for
/// each one as it completes. Results are reported in the same order as the
/// input (rendering is strictly sequential).
///
/// Returns `Err` only for fatal failures of sekien itself: an invalid
/// `config_json` (checked up front), or display initialisation, WebView
/// creation, and malformed IPC (which can only occur once rendering has
/// started). Errors raised by `on_result` are the caller's responsibility —
/// `on_result` may, for example, call [`std::process::exit`] directly.
///
/// `config_json` is a JSON object string spread into mermaid.initialize()
/// (e.g. `{"theme":"dark","fontFamily":"Arial"}`), or `None` for defaults.
/// If it doesn't parse as a JSON object, returns `Err(Error::Config(_))`
/// immediately.
///
/// # Main thread only
///
/// `render_stream` creates and runs a `tao` event loop, which panics if not
/// called from the process's main thread. It blocks the calling thread until
/// rendering is complete. Callers that need to do other work concurrently
/// (e.g. while diagrams are being fed in from `diagrams`'s iterator, which
/// runs on its own thread) must do that work on a thread other than main —
/// `render_stream` itself must run on main.
pub fn render_stream(
    diagrams: impl IntoIterator<Item = String> + Send + 'static,
    config_json: Option<&str>,
    mut on_result: impl FnMut(RenderOutcome),
) -> Result<()> {
    html::validate_config_json(config_json)?;

    #[cfg(target_os = "linux")]
    linux_display::ensure_display()
        .map_err(|e| Error::Internal(format!("failed to initialize display: {e:#}")))?;

    let mut event_loop = EventLoopBuilder::<Input>::with_user_event().build();
    let window = create_window(&event_loop)?;
    let webview = create_webview(
        &window,
        html::build_html(config_json),
        event_loop.create_proxy(),
    )?;

    let feeder = event_loop.create_proxy();
    std::thread::spawn(move || {
        for content in diagrams {
            if feeder.send_event(Input::Block(content)).is_err() {
                return;
            }
        }
        let _ = feeder.send_event(Input::End);
    });

    let mut collector = Collector::new();
    // Set once the loop should exit: `Ok` when done, `Err` on a fatal error.
    let mut exit: Option<Result<()>> = None;

    event_loop.run_return(|event, _, control_flow| {
        let input = match event {
            Event::UserEvent(input) => Some(input),
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => Some(Input::WindowClosed),
            _ => None,
        };
        if let (None, Some(input)) = (&exit, input) {
            for action in collector.handle(input) {
                match action {
                    Action::Dispatch { id, content } => {
                        if let Err(e) = webview.evaluate_script(&html::render_script(id, &content))
                        {
                            exit = Some(Err(Error::Internal(format!(
                                "failed to dispatch render({id}) to webview: {e}"
                            ))));
                        }
                    }
                    Action::Emit(outcome) => on_result(outcome),
                    Action::Done => exit = Some(Ok(())),
                    Action::Fatal(e) => exit = Some(Err(e)),
                }
                if exit.is_some() {
                    break;
                }
            }
        }
        *control_flow = if exit.is_some() {
            ControlFlow::Exit
        } else {
            ControlFlow::Wait
        };
    });

    exit.unwrap_or(Ok(()))
}
