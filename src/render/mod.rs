//! Streaming Mermaid → SVG renderer using wry (WebView) and tao (event loop).
//!
//! ## Architecture
//!
//! [`render_stream`] is split into a pure core and an impure shell:
//!
//! - [`Collector`] (in `collector.rs`) is a pure state machine: given an input
//!   event (a new diagram, end-of-input, or an IPC message from the WebView), it
//!   returns the [`Action`]s that should happen next. It does not touch the
//!   WebView, the event loop, or any I/O, so it can be unit tested directly.
//!   The page it talks to is built by the equally pure `html.rs`.
//! - [`render_stream`] is the thin impure shell: it owns the WebView/event loop,
//!   feeds events into the [`Collector`], and executes the [`Action`]s it returns
//!   (dispatching a render, calling `on_result`, or exiting the loop).
//!
//! ```text
//! [feeder thread]             [event loop]                      [WebView / mermaid.js]
//!   |                              |                                  |
//!   |-- Block(1, ...) ------------>|-- Collector::on_block -------->|
//!   |                              |     -> Action::Dispatch(1) --->|-- evaluate_script
//!   |-- Block(2, ...) ------------>|-- Collector::on_block (queued) |
//!   |-- InputEnd ------------------>|-- Collector::on_input_end      |
//!   |                              |<-- IPC: svg/error 1 ------------|
//!   |                              |-- Collector::on_ipc ---------->|
//!   |                              |     -> Action::Emit(...) ---- on_result(...)
//!   |                              |     -> Action::Dispatch(2) --->|-- evaluate_script
//!   |                              |<-- IPC: svg/error 2 ------------|
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
use collector::{Action, Collector, IpcMessage};
use tao::{
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

fn create_window(event_loop: &EventLoopWindowTarget<LoopEvent>) -> Result<Window> {
    // macOS/Windows: place the window off-screen so it is not visible.
    // Linux: position doesn't matter inside Xvfb, but 1x1 triggers a GDK assertion,
    // so use 100x100.
    let size = if cfg!(target_os = "linux") { 100 } else { 1 };
    let builder = WindowBuilder::new()
        .with_transparent(true)
        .with_decorations(false)
        .with_always_on_top(false)
        .with_visible(true)
        .with_inner_size(tao::dpi::LogicalSize::new(size, size));
    #[cfg(not(target_os = "linux"))]
    let builder = builder.with_position(tao::dpi::LogicalPosition::new(-10000, -10000));
    let window = builder
        .build(event_loop)
        .map_err(|e| Error::Internal(format!("failed to create window: {e}")))?;
    #[cfg(not(target_os = "linux"))]
    window.set_outer_position(tao::dpi::LogicalPosition::new(-10000, -10000));
    Ok(window)
}

fn create_webview(
    window: &Window,
    html: String,
    proxy: EventLoopProxy<LoopEvent>,
) -> Result<WebView> {
    WebViewBuilder::new()
        .with_background_color((0, 0, 0, 0))
        .with_transparent(true)
        .with_html(html)
        .with_ipc_handler(move |req| {
            let _ = proxy.send_event(LoopEvent::Ipc(req.into_body()));
        })
        .build(window)
        .map_err(|e| Error::Internal(format!("failed to create webview: {e}")))
}

fn dispatch_render(id: usize, content: &str, wv: &WebView) -> Result<()> {
    // serde_json produces a valid JS string literal (escaping `"`, `\`,
    // control chars, U+2028/U+2029). evaluate_script bypasses the HTML parser,
    // so `</script>` does not need the extra escaping that build_html requires.
    let content_literal = serde_json::to_string(content).expect("serialize Mermaid block content");
    let js = format!("renderMermaid({id}, {content_literal})");
    wv.evaluate_script(&js)
        .map_err(|e| Error::Internal(format!("failed to dispatch render({id}) to webview: {e}")))
}

/// Events delivered to the event loop via [`EventLoopProxy`].
enum LoopEvent {
    /// A diagram from the input, with its 1-origin position.
    Block(usize, String),
    /// The input iterator is exhausted; no more `Block`s will arrive.
    InputEnd,
    /// A raw IPC message from the WebView (JSON, parsed into [`IpcMessage`]).
    Ipc(String),
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

    let mut event_loop = EventLoopBuilder::<LoopEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();

    let window = create_window(&event_loop)?;
    let webview = create_webview(&window, html::build_html(config_json), proxy.clone())?;

    {
        let proxy = proxy.clone();
        std::thread::spawn(move || {
            for (i, content) in diagrams.into_iter().enumerate() {
                if proxy.send_event(LoopEvent::Block(i + 1, content)).is_err() {
                    return;
                }
            }
            let _ = proxy.send_event(LoopEvent::InputEnd);
        });
    }

    let mut collector = Collector::new();
    let mut fatal: Option<Error> = None;

    event_loop.run_return(|event, _, control_flow| {
        *control_flow = ControlFlow::Wait;

        let actions = match event {
            Event::UserEvent(LoopEvent::Block(id, content)) => collector.on_block(id, content),
            Event::UserEvent(LoopEvent::InputEnd) => collector.on_input_end(),
            Event::UserEvent(LoopEvent::Ipc(raw)) => match serde_json::from_str::<IpcMessage>(&raw)
            {
                Ok(msg) => collector.on_ipc(msg),
                Err(e) => vec![Action::Fatal(Error::Internal(format!(
                    "malformed IPC: {e} (raw: {raw})"
                )))],
            },
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                *control_flow = ControlFlow::Exit;
                vec![]
            }
            _ => vec![],
        };

        for action in actions {
            match action {
                Action::Dispatch { id, content } => {
                    if let Err(e) = dispatch_render(id, &content, &webview) {
                        fatal = Some(e);
                        *control_flow = ControlFlow::Exit;
                    }
                }
                Action::Emit(outcome) => on_result(outcome),
                Action::Done => *control_flow = ControlFlow::Exit,
                Action::Fatal(e) => {
                    fatal = Some(e);
                    *control_flow = ControlFlow::Exit;
                }
            }
        }
    });

    match fatal {
        Some(e) => Err(e),
        None => Ok(()),
    }
}
