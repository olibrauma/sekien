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

/// Rendering options, passed to mermaid.initialize().
#[derive(Clone, Default)]
pub struct RenderConfig {
    pub font_family: Option<String>,
    pub theme: Option<String>,
    pub look: Option<String>,
    /// Normalised JSON object string, spread into mermaid.initialize().
    pub config_json: Option<String>,
}

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
    #[error("failed to initialize display: {0}")]
    Display(String),
    #[error("failed to create window: {0}")]
    Window(String),
    #[error("failed to create webview: {0}")]
    WebView(String),
    #[error("malformed IPC from webview: {0}")]
    Ipc(String),
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

fn js_string_in_html(s: &str) -> String {
    escape_for_script(&serde_json::to_string(s).expect("serialize config field"))
}

fn build_html(config: &RenderConfig) -> String {
    let extra_config: String = [
        ("theme", &config.theme),
        ("fontFamily", &config.font_family),
        ("look", &config.look),
    ]
    .iter()
    .filter_map(|(k, v)| {
        v.as_deref()
            .map(|v| format!("  {k}: {},\n", js_string_in_html(v)))
    })
    .collect();

    let config_json = config
        .config_json
        .as_deref()
        .map_or_else(|| "{}".to_string(), escape_for_script);

    HTML_TEMPLATE
        .replace("{{MERMAID_JS}}", MERMAID_JS)
        .replace("{{CONFIG_JSON}}", &config_json)
        .replace("{{EXTRA_CONFIG}}", &extra_config)
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
        .map_err(|e| Error::Window(e.to_string()))?;
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
        .map_err(|e| Error::WebView(e.to_string()))
}

fn dispatch_render(id: usize, content: &str, wv: &WebView) -> Result<()> {
    // serde_json produces a valid JS string literal (escaping `"`, `\`,
    // control chars, U+2028/U+2029). evaluate_script bypasses the HTML parser,
    // so `</script>` does not need the extra escaping that build_html requires.
    let content_literal = serde_json::to_string(content).expect("serialize Mermaid block content");
    let js = format!("renderMermaid({id}, {content_literal})");
    wv.evaluate_script(&js)
        .map_err(|e| Error::WebView(format!("failed to dispatch render({id}) to webview: {e}")))
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

/// Pure state machine driving [`render_stream`]. See the module-level docs for
/// the overall flow.
struct Collector {
    /// Diagrams received but not yet dispatched (1-origin id, content).
    queue: VecDeque<(usize, String)>,
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
        self.queue.push_back((id, content));
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
            IpcMessage::Error { id, error } => {
                self.on_render_done(id, RenderOutcome::Error(error))
            }
        }
    }

    fn on_render_done(&mut self, id: usize, outcome: RenderOutcome) -> Vec<Action> {
        if !matches!(self.pipeline, Pipeline::Awaiting(n) if n == id) {
            return vec![Action::Fatal(Error::Ipc(format!(
                "received result for id {id} but pipeline state is {:?}",
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
        if let Some((id, content)) = self.queue.pop_front() {
            self.pipeline = Pipeline::Awaiting(id);
            vec![Action::Dispatch { id, content }]
        } else if self.end_received {
            vec![Action::Done]
        } else {
            vec![]
        }
    }
}

/// Renders each diagram in `diagrams` to SVG, calling `on_result(id, outcome)` for
/// each one as it completes. `id` is the 1-origin position of the diagram in
/// `diagrams`'s iteration order; results are reported in that same order
/// (rendering is strictly sequential).
///
/// Returns `Err` only for fatal failures of sekien itself (display
/// initialisation, WebView creation, malformed IPC). Errors raised by
/// `on_result` are the caller's responsibility — `on_result` may, for example,
/// call [`std::process::exit`] directly.
pub fn render_stream(
    diagrams: impl IntoIterator<Item = String> + Send + 'static,
    config: &RenderConfig,
    mut on_result: impl FnMut(usize, RenderOutcome) + Send + 'static,
) -> Result<()> {
    #[cfg(target_os = "linux")]
    linux_display::ensure_display().map_err(|e| Error::Display(format!("{e:#}")))?;

    let mut event_loop = EventLoopBuilder::<LoopEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();

    let window = create_window(&event_loop)?;
    let webview = create_webview(&window, build_html(config), proxy.clone())?;

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
                Err(e) => vec![Action::Fatal(Error::Ipc(format!("{e} (raw: {raw})")))],
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
                Action::Emit { id, outcome } => on_result(id, outcome),
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
    fn js_string_in_html_escapes_quotes_and_backslashes() {
        assert_eq!(js_string_in_html("Arial"), "\"Arial\"");
        assert_eq!(js_string_in_html("Font\"Name"), "\"Font\\\"Name\"");
        assert_eq!(js_string_in_html("Font\\Name"), "\"Font\\\\Name\"");
        assert_eq!(
            js_string_in_html("'; alert('xss'); '"),
            "\"'; alert('xss'); '\""
        );
    }

    fn cfg(font_family: Option<&str>, theme: Option<&str>) -> RenderConfig {
        RenderConfig {
            font_family: font_family.map(|s| s.to_string()),
            theme: theme.map(|s| s.to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn build_html_defaults_have_no_extra_config() {
        let html = build_html(&cfg(None, None));
        assert!(!html.contains("  fontFamily:"));
        assert!(!html.contains("  theme:"));
        assert!(html.contains("...{}"));
    }

    #[test]
    fn build_html_includes_font_and_theme() {
        let html = build_html(&cfg(Some("Arial"), Some("dark")));
        assert!(html.contains("fontFamily: \"Arial\""));
        assert!(html.contains("theme: \"dark\""));
    }

    #[test]
    fn build_html_with_config_json() {
        let html = build_html(&RenderConfig {
            config_json: Some(r#"{"flowchart":{"curve":"basis"}}"#.to_string()),
            ..Default::default()
        });
        assert!(html.contains(r#"...{"flowchart":{"curve":"basis"}}"#));
    }

    #[test]
    fn build_html_escapes_closing_script_tags_in_fields() {
        // `</script>` を埋め込まれても script ブロックから抜けられないこと
        let html = build_html(&RenderConfig {
            theme: Some("</script>".to_string()),
            config_json: Some(r#"{"fontFamily":"a</script>b"}"#.to_string()),
            ..Default::default()
        });
        assert!(!html.contains("theme: \"</script>\""));
        assert!(!html.contains(r#""a</script>b""#));
        assert_eq!(html.matches("\\u003c/script\\u003e").count(), 2);
    }

    #[test]
    fn build_html_config_json_cli_flag_comes_after_spread() {
        // spread より後に個別フラグが来ることで CLI フラグが優先されることを確認
        let html = build_html(&RenderConfig {
            theme: Some("forest".to_string()),
            config_json: Some(r#"{"theme":"dark"}"#.to_string()),
            ..Default::default()
        });
        let spread_pos = html.find("...{").unwrap();
        let theme_pos = html.find("theme: \"forest\"").unwrap();
        assert!(
            spread_pos < theme_pos,
            "spread must appear before CLI flag override"
        );
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
            [Action::Fatal(Error::Ipc(_))]
        ));

        // A result arrives for a different id than the one in flight.
        let mut c = Collector::new();
        c.on_block(1, "a".into());
        c.on_ipc(ready()); // awaiting 1
        assert!(matches!(
            c.on_ipc(svg(2, "<svg/>")).as_slice(),
            [Action::Fatal(Error::Ipc(_))]
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
