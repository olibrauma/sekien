//! Streaming Mermaid → SVG renderer using wry (WebView) and tao (event loop).
//!
//! ## Architecture
//!
//! [`render_stream`] is split into a pure core and an impure shell:
//!
//! - [`Collector`] is a pure state machine: given an input event (a new diagram,
//!   end-of-input, or an IPC message from the WebView), it returns the [`Action`]s
//!   that should happen next. It does not touch the WebView, the event loop, or
//!   any I/O, so it can be unit tested directly.
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
//!   |                              |     -> Action::Emit(1, ...) -- on_result(1, ...)
//!   |                              |     -> Action::Dispatch(2) --->|-- evaluate_script
//!   |                              |<-- IPC: svg/error 2 ------------|
//!   |                              |     -> Action::Emit(2, ...) -- on_result(2, ...)
//!   |                              |     -> Action::Done -> loop exits (run_return)
//! ```
//!
//! `render_stream` dispatches at most one render at a time (mermaid.render is not
//! parallelisable), in input order. `on_result` is therefore called in input order
//! too: `on_result`'s first argument is the 1-origin position of the diagram in
//! `diagrams`.

use serde::Deserialize;
use std::collections::VecDeque;
use tao::{
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy, EventLoopWindowTarget},
    platform::run_return::EventLoopExtRunReturn,
    window::{Window, WindowBuilder},
};
use wry::{WebView, WebViewBuilder};

#[cfg(target_os = "linux")]
use crate::linux_display;

const MERMAID_JS: &str = include_str!("../assets/mermaid.min.js");
const HTML_TEMPLATE: &str = include_str!("../assets/render.html");

/// Version string extracted from `mermaid.min.js` by `build.rs` at compile time.
pub const MERMAID_VERSION: &str = env!("MERMAID_VERSION");

/// Result of rendering a single diagram.
#[derive(Debug, PartialEq, Eq)]
pub enum RenderOutcome {
    /// Rendered successfully; the SVG markup.
    Svg(String),
    /// Mermaid failed to parse/render the diagram; the error message.
    Error(String),
}

/// Fatal failure of [`render_stream`] itself (not a per-diagram render error,
/// which is reported via [`RenderOutcome::Error`]).
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum Error {
    /// `config_json` did not parse as a JSON object. Returned before any
    /// display/window/WebView initialisation is attempted.
    #[error("invalid config_json: {0}")]
    Config(String),
    /// Internal failure (display init, window/WebView creation, or malformed
    /// IPC). The message describes the specific cause.
    #[error("{0}")]
    Internal(String),
}

pub type Result<T> = std::result::Result<T, Error>;

/// IPC messages from the WebView. Three variants, fixed by `build_html`:
///
/// - `{"type":"ready"}`: mermaid.initialize() complete
/// - `{"type":"svg","id":N,"svg":"..."}`: block N rendered successfully
/// - `{"type":"error","id":N,"error":"..."}`: block N failed to parse
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum IpcMessage {
    Ready,
    Svg { id: usize, svg: String },
    Error { id: usize, error: String },
}

/// Escapes `<` and `>` to Unicode escapes for safe embedding inside `<script>`.
fn escape_for_script(s: &str) -> String {
    s.replace('<', "\\u003c").replace('>', "\\u003e")
}

/// Validates that `config_json` (if present) parses as a JSON object.
///
/// A `config_json` that isn't a valid JS object literal breaks
/// `mermaid.initialize()` in the WebView before it can signal readiness,
/// which would otherwise hang [`render_stream`] forever instead of
/// returning an error.
fn validate_config_json(config_json: Option<&str>) -> Result<()> {
    if let Some(s) = config_json {
        let value: serde_json::Value =
            serde_json::from_str(s).map_err(|e| Error::Config(e.to_string()))?;
        if !value.is_object() {
            return Err(Error::Config(format!("expected a JSON object, got: {s}")));
        }
    }
    Ok(())
}

fn build_html(config_json: Option<&str>) -> String {
    let config_json = config_json.map_or_else(|| "{}".to_string(), escape_for_script);

    HTML_TEMPLATE
        .replace("{{MERMAID_JS}}", MERMAID_JS)
        .replace("{{CONFIG_JSON}}", &config_json)
}

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
    /// A raw IPC message from the WebView (JSON, parsed by [`Collector::on_ipc`]).
    Ipc(String),
}

/// Three-state machine gating dispatch to the WebView.
///
/// - `NotReady`: mermaid.initialize() not yet complete
/// - `Idle`: WebView ready, no render in flight
/// - `Awaiting(N)`: rendering block N
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pipeline {
    NotReady,
    Idle,
    Awaiting(usize),
}

/// An effect that [`render_stream`]'s impure shell should perform, as decided by
/// [`Collector`].
#[derive(Debug, PartialEq, Eq)]
enum Action {
    /// Dispatch diagram `id` (`content`) to the WebView.
    Dispatch { id: usize, content: String },
    /// Report the outcome for diagram `id` to the caller.
    Emit { id: usize, outcome: RenderOutcome },
    /// All diagrams processed; the event loop should exit.
    Done,
    /// Fatal failure; the event loop should exit and return this error.
    Fatal(Error),
}

struct PendingBlock {
    id: usize,
    content: String,
}

/// Pure state machine driving [`render_stream`]. See the module-level docs for
/// the overall flow.
struct Collector {
    queue: VecDeque<PendingBlock>,
    /// Whether the input iterator is exhausted.
    end_received: bool,
    pipeline: Pipeline,
}

impl Collector {
    fn new() -> Self {
        Self {
            queue: VecDeque::new(),
            end_received: false,
            pipeline: Pipeline::NotReady,
        }
    }

    fn on_block(&mut self, id: usize, content: String) -> Vec<Action> {
        self.queue.push_back(PendingBlock { id, content });
        self.try_dispatch_next()
    }

    fn on_input_end(&mut self) -> Vec<Action> {
        self.end_received = true;
        self.try_dispatch_next()
    }

    fn on_ipc(&mut self, msg: IpcMessage) -> Vec<Action> {
        match msg {
            IpcMessage::Ready => {
                self.pipeline = Pipeline::Idle;
                self.try_dispatch_next()
            }
            IpcMessage::Svg { id, svg } => self.on_render_done(id, RenderOutcome::Svg(svg)),
            IpcMessage::Error { id, error } => self.on_render_done(id, RenderOutcome::Error(error)),
        }
    }

    fn on_render_done(&mut self, id: usize, outcome: RenderOutcome) -> Vec<Action> {
        if !matches!(self.pipeline, Pipeline::Awaiting(n) if n == id) {
            return vec![Action::Fatal(Error::Internal(format!(
                "malformed IPC: received result for id {id} but pipeline state is {:?}",
                self.pipeline
            )))];
        }
        self.pipeline = Pipeline::Idle;
        let mut actions = vec![Action::Emit { id, outcome }];
        actions.extend(self.try_dispatch_next());
        actions
    }

    /// Dispatches the next queued block if conditions are met, or signals `Done`
    /// if the queue is empty and the input is exhausted.
    fn try_dispatch_next(&mut self) -> Vec<Action> {
        if !matches!(self.pipeline, Pipeline::Idle) {
            return vec![];
        }
        if let Some(PendingBlock { id, content }) = self.queue.pop_front() {
            self.pipeline = Pipeline::Awaiting(id);
            vec![Action::Dispatch { id, content }]
        } else if self.end_received {
            vec![Action::Done]
        } else {
            vec![]
        }
    }
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
    validate_config_json(config_json)?;

    #[cfg(target_os = "linux")]
    linux_display::ensure_display().map_err(|e| Error::Internal(format!("failed to initialize display: {e:#}")))?;

    let mut event_loop = EventLoopBuilder::<LoopEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();

    let window = create_window(&event_loop)?;
    let webview = create_webview(&window, build_html(config_json), proxy.clone())?;

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
                Err(e) => vec![Action::Fatal(Error::Internal(format!("malformed IPC: {e} (raw: {raw})")))],
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
                Action::Emit { id: _, outcome } => on_result(outcome),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_for_script_escapes_angle_brackets() {
        assert_eq!(
            escape_for_script("</script><script>"),
            "\\u003c/script\\u003e\\u003cscript\\u003e"
        );
    }

    #[test]
    fn validate_config_json_accepts_none_and_objects() {
        assert!(validate_config_json(None).is_ok());
        assert!(validate_config_json(Some("{}")).is_ok());
        assert!(validate_config_json(Some(r#"{"theme":"dark"}"#)).is_ok());
    }

    #[test]
    fn validate_config_json_rejects_non_objects_and_invalid_json() {
        assert!(matches!(
            validate_config_json(Some("not json")),
            Err(Error::Config(_))
        ));
        assert!(matches!(
            validate_config_json(Some("[1, 2, 3]")),
            Err(Error::Config(_))
        ));
        assert!(matches!(
            validate_config_json(Some(r#""a string""#)),
            Err(Error::Config(_))
        ));
    }

    #[test]
    fn build_html_defaults_to_empty_config() {
        let html = build_html(None);
        assert!(html.contains("...{}"));
    }

    #[test]
    fn build_html_with_config_json() {
        let html = build_html(Some(r#"{"flowchart":{"curve":"basis"}}"#));
        assert!(html.contains(r#"...{"flowchart":{"curve":"basis"}}"#));
    }

    #[test]
    fn build_html_escapes_closing_script_tags_in_config_json() {
        // Embedding `</script>` must not break out of the script block.
        let html = build_html(Some(r#"{"theme":"</script>"}"#));
        assert!(!html.contains("</script>\""));
        assert!(html.contains("\\u003c/script\\u003e"));
    }

    fn parse_ipc(s: &str) -> std::result::Result<IpcMessage, serde_json::Error> {
        serde_json::from_str(s)
    }

    #[test]
    fn ipc_message_valid_variants() {
        assert!(matches!(
            parse_ipc(r#"{"type":"ready"}"#).unwrap(),
            IpcMessage::Ready
        ));
        assert!(matches!(
            parse_ipc(r#"{"type":"svg","id":1,"svg":"<svg/>"}"#).unwrap(),
            IpcMessage::Svg { id: 1, ref svg } if svg == "<svg/>"
        ));
        assert!(matches!(
            parse_ipc(r#"{"type":"error","id":2,"error":"Lexical error"}"#).unwrap(),
            IpcMessage::Error { id: 2, ref error } if error == "Lexical error"
        ));
    }

    #[test]
    fn ipc_message_invalid_variants_are_rejected() {
        for raw in [
            r#"{"type":"frobnicate"}"#,
            r#"{"id":1,"svg":"<svg/>"}"#,
            r#"{"type":"svg","svg":"<svg/>"}"#,
            r#"{"type":"svg","id":1}"#,
            r#"{"type":"svg","id":"oops","svg":"<svg/>"}"#,
            r#"{"type":"svg","id":1,"svg":42}"#,
            r#"{"type":"error","id":1}"#,
            r#"{"type":"error","id":1,"error":42}"#,
        ] {
            assert!(parse_ipc(raw).is_err(), "expected error for {raw}");
        }
    }

    // ------ Collector ------

    fn ready() -> IpcMessage {
        IpcMessage::Ready
    }

    fn svg(id: usize, s: &str) -> IpcMessage {
        IpcMessage::Svg {
            id,
            svg: s.to_string(),
        }
    }

    fn error(id: usize, s: &str) -> IpcMessage {
        IpcMessage::Error {
            id,
            error: s.to_string(),
        }
    }

    fn dispatch(id: usize, content: &str) -> Action {
        Action::Dispatch {
            id,
            content: content.to_string(),
        }
    }

    fn emit_svg(id: usize, s: &str) -> Action {
        Action::Emit {
            id,
            outcome: RenderOutcome::Svg(s.to_string()),
        }
    }

    fn emit_err(id: usize, s: &str) -> Action {
        Action::Emit {
            id,
            outcome: RenderOutcome::Error(s.to_string()),
        }
    }

    #[test]
    fn collector_not_ready_queues_without_dispatch() {
        let mut c = Collector::new();
        assert_eq!(c.on_block(1, "a".into()), vec![]);
    }

    #[test]
    fn collector_ready_dispatches_queued_block() {
        let mut c = Collector::new();
        c.on_block(1, "a".into());
        assert_eq!(c.on_ipc(ready()), vec![dispatch(1, "a")]);
    }

    #[test]
    fn collector_svg_result_emits_and_dispatches_next() {
        let mut c = Collector::new();
        c.on_block(1, "a".into());
        c.on_block(2, "b".into());
        c.on_ipc(ready()); // dispatches 1

        assert_eq!(
            c.on_ipc(svg(1, "<svg/>")),
            vec![emit_svg(1, "<svg/>"), dispatch(2, "b")]
        );
    }

    #[test]
    fn collector_error_result_is_emitted() {
        let mut c = Collector::new();
        c.on_block(1, "bogus".into());
        c.on_ipc(ready());

        assert_eq!(
            c.on_ipc(error(1, "Lexical error")),
            vec![emit_err(1, "Lexical error")]
        );
    }

    #[test]
    fn collector_done_after_input_end_and_drain() {
        let mut c = Collector::new();
        c.on_block(1, "a".into());
        c.on_ipc(ready()); // dispatches 1
        c.on_input_end();
        assert_eq!(
            c.on_ipc(svg(1, "<svg/>")),
            vec![emit_svg(1, "<svg/>"), Action::Done]
        );
    }

    #[test]
    fn collector_input_end_before_ready_then_empty() {
        let mut c = Collector::new();
        c.on_input_end();
        assert_eq!(c.on_ipc(ready()), vec![Action::Done]);
    }

    #[test]
    fn collector_unexpected_ipc_result_is_fatal() {
        // A result arrives before any block was dispatched (pipeline NotReady).
        assert!(matches!(
            Collector::new().on_ipc(svg(1, "<svg/>")).as_slice(),
            [Action::Fatal(Error::Internal(_))]
        ));

        // A result arrives for a different id than the one in flight.
        let mut c = Collector::new();
        c.on_block(1, "a".into());
        c.on_ipc(ready()); // awaiting 1
        assert!(matches!(
            c.on_ipc(svg(2, "<svg/>")).as_slice(),
            [Action::Fatal(Error::Internal(_))]
        ));
    }

    #[test]
    fn collector_blocks_arriving_after_ready_are_dispatched_in_order() {
        let mut c = Collector::new();
        c.on_ipc(ready()); // Idle, queue empty -> no action yet
        assert_eq!(c.on_block(1, "a".into()), vec![dispatch(1, "a")]);
        // Second block queues behind the in-flight render.
        assert_eq!(c.on_block(2, "b".into()), vec![]);
        assert_eq!(
            c.on_ipc(svg(1, "<svg/>")),
            vec![emit_svg(1, "<svg/>"), dispatch(2, "b")]
        );
    }
}
