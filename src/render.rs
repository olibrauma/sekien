//! Streaming Mermaid → SVG renderer using wry (WebView) and tao (event loop).
//! Internal to the sekien crate.
//!
//! ## Architecture
//!
//! sekien behaves like a long-lived streaming process, similar to cat. It reads
//! the input splitting on `\0` delimiters, and dispatches each block to the
//! WebView in arrival order.
//!
//! ```text
//! [reader thread]            [event loop / main]                [WebView / mermaid.js]
//!   |                              |                                  |
//!   |-- Block (1 item) ----------->|                                  |
//!   |                              |-- Launch WebView (once) ------->|
//!   |                              |<-- IPC: ready -------------------|
//!   |                              |-- evaluate_script: render(N) -->|
//!   |                              |<-- IPC: svg N or error N --------|
//!   |                              |-- write SVG to stdout (flush)   /
//!   |                              |   write error to stderr          |
//!   |-- InputEnd (EOF) ----------->|                                  |
//!   |                              |-- process::exit(0)
//! ```
//!
//! Blocks arriving faster than the WebView renders are queued internally.
//! A per-block render failure does not stop the event loop (continue-on-error).
//! After EOF and queue drain: exit 0. Only sekien's own failures (display init,
//! malformed IPC, stdout write failure) trigger exit 1.

use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::VecDeque;
use std::io::{self, BufRead, BufReader, Read, Write};
use tao::{
    event::{Event, WindowEvent},
    event_loop::{ControlFlow, EventLoopBuilder, EventLoopProxy, EventLoopWindowTarget},
    window::{Window, WindowBuilder},
};
use wry::{WebView, WebViewBuilder};

#[cfg(target_os = "linux")]
use crate::linux_display;

const MERMAID_JS: &str = include_str!("../assets/mermaid.min.js");
const HTML_TEMPLATE: &str = include_str!("../assets/render.html");

/// Version string extracted from `mermaid.min.js` by `build.rs` at compile time.
/// Baked into the binary so `--version` stays in sync with the bundled JS.
pub const MERMAID_VERSION: &str = env!("MERMAID_VERSION");

#[derive(Clone, Default)]
pub struct RenderConfig {
    pub font_family: Option<String>,
    pub theme: Option<String>,
    pub look: Option<String>,
    pub show_meta: bool,
    /// Normalised JSON object string from `--config`, spread into mermaid.initialize().
    pub config_json: Option<String>,
}

/// User events handled by the event loop.
/// Produced by the reader thread (Block / InputEnd / InputError) and the WebView (Ipc).
enum LoopEvent {
    Block(String),
    InputEnd,
    InputError(String),
    Ipc(String),
}

/// IPC messages from the WebView. Three variants, fixed by `build_html`:
///
/// - `{"type":"ready"}`: mermaid.initialize() complete
/// - `{"type":"svg","id":N,"svg":"..."}`: block N rendered successfully
/// - `{"type":"error","id":N,"error":"..."}`: block N failed to parse
///
/// The `type` field is the serde discriminant; missing fields, type mismatches,
/// and unknown variants are all rejected by serde with `Err`.
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum IpcMessage {
    Ready,
    Svg { id: usize, svg: String },
    Error { id: usize, error: String },
}

/// Escapes `<` and `>` to Unicode escapes for safe embedding inside `<script>`.
///
/// serde_json does not escape these characters, so an unescaped `</script>`
/// would terminate the script tag early. Used for both string literals and JSON objects.
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

    // Spread the --config JSON first; CLI flags (EXTRA_CONFIG) follow and take precedence.
    // Fall back to `{}` (no-op spread) when --config is not set.
    let config_json = config
        .config_json
        .as_deref()
        .map_or_else(|| "{}".to_string(), escape_for_script);

    // Substitute placeholders in the HTML template.
    // None of mermaid.min.js / extra_config / config_json contain the placeholder
    // strings, so naive String::replace is collision-free (no template engine needed).
    HTML_TEMPLATE
        .replace("{{MERMAID_JS}}", MERMAID_JS)
        .replace("{{CONFIG_JSON}}", &config_json)
        .replace("{{EXTRA_CONFIG}}", &extra_config)
}

fn create_window(event_loop: &EventLoopWindowTarget<LoopEvent>) -> anyhow::Result<Window> {
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
        .map_err(|e| anyhow::anyhow!("failed to create window: {e}"))?;
    #[cfg(not(target_os = "linux"))]
    window.set_outer_position(tao::dpi::LogicalPosition::new(-10000, -10000));
    Ok(window)
}

fn create_webview(
    window: &Window,
    html: String,
    proxy: EventLoopProxy<LoopEvent>,
) -> anyhow::Result<WebView> {
    WebViewBuilder::new()
        .with_background_color((0, 0, 0, 0))
        .with_transparent(true)
        .with_html(html)
        .with_ipc_handler(move |req| {
            let _ = proxy.send_event(LoopEvent::Ipc(req.into_body()));
        })
        .build(window)
        .map_err(|e| anyhow::anyhow!("failed to create webview: {e}"))
}

/// Emits `buf` as a single `Block`. On invalid UTF-8, emits `InputError` and
/// returns `false` to signal the caller to stop reading.
fn emit_block<F: FnMut(LoopEvent)>(buf: &mut Vec<u8>, on_event: &mut F) -> bool {
    String::from_utf8(std::mem::take(buf))
        .map(|s| {
            on_event(LoopEvent::Block(s));
            true
        })
        .unwrap_or_else(|e| {
            on_event(LoopEvent::InputError(format!(
                "input is not valid UTF-8: {e}"
            )));
            false
        })
}

/// Reads the input stream, splits on `\0`, and calls `on_event` for each block.
///
/// On `\0`: emits the current buffer (even if empty) as `Block`.
/// On EOF: emits the buffer only if non-empty (drops a single trailing `\0`).
/// Always emits exactly one `InputEnd` at the end.
/// On I/O or UTF-8 error: emits `InputError` and returns immediately
/// (`InputEnd` is NOT emitted in this case).
fn read_blocks<R: Read>(reader: R, mut on_event: impl FnMut(LoopEvent)) {
    let mut reader = BufReader::new(reader);
    let mut buf: Vec<u8> = Vec::new();
    loop {
        match reader.read_until(0, &mut buf) {
            Ok(0) => break,
            Ok(_) => {
                let is_nul = buf.last() == Some(&0);
                if is_nul {
                    buf.pop();
                }
                let should_emit = is_nul || !String::from_utf8_lossy(&buf).trim().is_empty();
                if should_emit && !emit_block(&mut buf, &mut on_event) {
                    return;
                }
                buf.clear();
            }
            Err(e) => {
                on_event(LoopEvent::InputError(format!("failed to read input: {e}")));
                return;
            }
        }
    }
    on_event(LoopEvent::InputEnd);
}

/// Three-state machine gating dispatch to the WebView.
///
/// - `NotReady`: mermaid.initialize() not yet complete
/// - `Idle`: WebView ready, no render in flight
/// - `Awaiting(N)`: rendering block N
///
/// Transitions: `NotReady` → (ready IPC) → `Idle` → (dispatch) → `Awaiting(N)`
/// → (svg/error IPC) → `Idle`. Invalid combinations (e.g. ready but awaiting)
/// are ruled out at the type level.
#[derive(Debug)]
enum Pipeline {
    NotReady,
    Idle,
    Awaiting(usize),
}

/// Result of processing one event. `Err` indicates a fatal failure.
type StepResult = anyhow::Result<Continuation>;

#[derive(Debug, PartialEq, Eq)]
enum Continuation {
    /// Keep the event loop running.
    Continue,
    /// All blocks processed through EOF; caller should exit 0.
    Done,
}

/// Output destination for an IPC result: `Svg` → stdout, `Error` → stderr.
#[derive(Clone, Copy)]
enum Channel {
    Stdout,
    Stderr,
}

impl Channel {
    fn kind(self) -> &'static str {
        match self {
            Channel::Stdout => "svg",
            Channel::Stderr => "error",
        }
    }
}

/// Streaming progress state. Dispatch is gated by [`Pipeline`]
/// (at most one block in flight, since mermaid.render is not parallelisable).
struct StreamState {
    /// 1-origin counter for the next block received from the reader thread.
    next_index: usize,
    /// Blocks waiting to be dispatched: (block id, content).
    queue: VecDeque<(usize, String)>,
    /// Whether EOF has been received from the reader thread.
    end_received: bool,
    /// Whether any SVG has been written to stdout (used to decide the `\0` separator).
    wrote_any_svg: bool,
    /// Whether any error has been written to stderr (used to decide the `\0` separator).
    wrote_any_error: bool,
    /// WebView readiness and whether a render is currently in flight.
    pipeline: Pipeline,
    config: RenderConfig,
}

impl StreamState {
    fn new(config: RenderConfig) -> Self {
        Self {
            next_index: 1,
            queue: VecDeque::new(),
            end_received: false,
            wrote_any_svg: false,
            wrote_any_error: false,
            pipeline: Pipeline::NotReady,
            config,
        }
    }

    /// Processes one event.
    ///
    /// Returns:
    /// - `Ok(Continue)`: keep the event loop running
    /// - `Ok(Done)`: all blocks processed; caller should exit 0
    /// - `Err(msg)`: fatal failure (InputError / malformed IPC / write failure);
    ///   caller prints `msg` to stderr and exits 1
    fn handle(&mut self, ev: LoopEvent, wv: &WebView) -> StepResult {
        match ev {
            LoopEvent::Block(content) => {
                let id = self.next_index;
                self.next_index += 1;
                self.queue.push_back((id, content));
                self.try_dispatch_next(wv)
            }
            LoopEvent::InputEnd => {
                self.end_received = true;
                self.try_dispatch_next(wv)
            }
            LoopEvent::InputError(e) => Err(anyhow::anyhow!(e)),
            LoopEvent::Ipc(msg) => self.on_ipc(&msg, wv),
        }
    }

    /// Dispatches the next queued block if conditions are met.
    /// Return value semantics are the same as [`StreamState::handle`].
    fn try_dispatch_next(&mut self, wv: &WebView) -> StepResult {
        if !matches!(self.pipeline, Pipeline::Idle) {
            return Ok(Continuation::Continue);
        }
        if let Some((id, content)) = self.queue.pop_front() {
            dispatch_render(id, &content, wv)?;
            self.pipeline = Pipeline::Awaiting(id);
            return Ok(Continuation::Continue);
        }
        // Queue empty and Idle — no render in flight.
        Ok(if self.end_received {
            Continuation::Done
        } else {
            Continuation::Continue
        })
    }

    /// Verifies that the pipeline is in state `Awaiting(id)`.
    fn check_awaiting(&self, id: usize, kind: &str) -> Result<()> {
        if matches!(self.pipeline, Pipeline::Awaiting(n) if n == id) {
            Ok(())
        } else {
            Err(ipc_protocol_error(&format!(
                "'{kind}' id {id} does not match pipeline state {:?}",
                self.pipeline
            )))
        }
    }

    /// Builds the output string: optional metadata comment + content + `\n`.
    fn format_output(&self, id: usize, content: &str) -> String {
        let prefix = if self.config.show_meta {
            format_block_comment(id)
        } else {
            String::new()
        };
        format!("{prefix}{content}\n")
    }

    fn on_ipc(&mut self, msg: &str, wv: &WebView) -> StepResult {
        let parsed: IpcMessage = serde_json::from_str(msg)
            .map_err(|e| ipc_protocol_error(&format!("{e} (raw: {msg})")))?;

        // Ready is control-flow only. Extract destination and content for Svg/Error.
        let (id, content, ch) = match parsed {
            IpcMessage::Ready => {
                self.pipeline = Pipeline::Idle;
                return self.try_dispatch_next(wv);
            }
            IpcMessage::Svg { id, svg } => (id, svg, Channel::Stdout),
            IpcMessage::Error { id, error } => (id, error, Channel::Stderr),
        };

        self.check_awaiting(id, ch.kind())?;
        let output = self.format_output(id, &content);
        match ch {
            Channel::Stdout => {
                write_output(io::stdout().lock(), &output, self.wrote_any_svg)
                    .context("failed to write SVG to stdout")?;
                self.wrote_any_svg = true;
            }
            Channel::Stderr => {
                write_output(io::stderr().lock(), &output, self.wrote_any_error)
                    .context("failed to write error to stderr")?;
                self.wrote_any_error = true;
            }
        }
        self.pipeline = Pipeline::Idle;
        self.try_dispatch_next(wv)
    }
}

fn format_block_comment(id: usize) -> String {
    format!("<!-- {{\"id\": {id}}} -->\n")
}

fn dispatch_render(id: usize, content: &str, wv: &WebView) -> anyhow::Result<()> {
    // serde_json produces a valid JS string literal (escaping `"`, `\`,
    // control chars, U+2028/U+2029). evaluate_script bypasses the HTML parser,
    // so `</script>` does not need the extra escaping that build_html requires.
    let content_literal = serde_json::to_string(content).expect("serialize Mermaid block content");
    let js = format!("renderMermaid({id}, {content_literal})");
    wv.evaluate_script(&js)
        .map_err(|e| anyhow::anyhow!("failed to dispatch render({id}) to webview: {e}"))
}

fn write_output(mut out: impl Write, content: &str, write_separator: bool) -> io::Result<()> {
    if write_separator {
        out.write_all(&[0])?;
    }
    out.write_all(content.as_bytes())?;
    out.flush()
}

/// Formats an error for unexpected IPC messages.
///
/// The IPC protocol is fully controlled by this crate (the WebView script is
/// fixed by `build_html`), so reaching this function indicates an internal bug
/// or a breaking change in wry. Silently exit 0 would present as "empty SVG"
/// to the user and be hard to debug, so this is surfaced as a fatal error.
fn ipc_protocol_error(detail: &str) -> anyhow::Error {
    anyhow::anyhow!("malformed IPC from webview: {detail}")
}

/// Reads the input stream and converts Mermaid blocks to SVG until EOF.
///
/// Successful SVGs are written to stdout separated by `\0`. Per-block render
/// failures are written to stderr and processing continues (exit 0).
/// Only setup failures return `Result::Err` (displayed by `main` as exit 1).
pub fn run_stream<R: Read + Send + 'static>(reader: R, config: &RenderConfig) -> Result<()> {
    #[cfg(target_os = "linux")]
    linux_display::ensure_display()?;

    let event_loop = EventLoopBuilder::<LoopEvent>::with_user_event().build();
    let proxy = event_loop.create_proxy();

    // Pre-warm the WebView: start loading the HTML before the reader thread
    // starts and before any data arrives, minimising latency to the first render.
    let window = create_window(&event_loop).unwrap_or_else(|e| exit_fatal(e));
    let webview = create_webview(&window, build_html(config), proxy.clone())
        .unwrap_or_else(|e| exit_fatal(e));

    std::thread::spawn(move || {
        read_blocks(reader, |ev| {
            let _ = proxy.send_event(ev);
        })
    });

    let mut state = StreamState::new(config.clone());
    event_loop.run(move |event, _event_loop, control_flow| {
        *control_flow = ControlFlow::Wait;
        let _keep_window_alive = &window;
        match event {
            Event::UserEvent(ev) => dispatch_or_exit(state.handle(ev, &webview)),
            Event::WindowEvent {
                event: WindowEvent::CloseRequested,
                ..
            } => {
                *control_flow = ControlFlow::Exit;
            }
            _ => {}
        }
    });
}

/// Interprets a `StepResult`: returns normally on `Continue`,
/// exits 0 on `Done`, exits 1 on `Err`.
fn dispatch_or_exit(result: StepResult) {
    match result {
        Ok(Continuation::Continue) => {}
        Ok(Continuation::Done) => std::process::exit(0),
        Err(e) => exit_fatal(e),
    }
}

/// The sole exit(1) path for fatal failures. Called only from the event loop closure.
fn exit_fatal(e: anyhow::Error) -> ! {
    eprintln!("Error: {e:?}");
    std::process::exit(1);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(font_family: Option<&str>, theme: Option<&str>) -> RenderConfig {
        RenderConfig {
            font_family: font_family.map(|s| s.to_string()),
            theme: theme.map(|s| s.to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn build_html_no_options() {
        let html = build_html(&cfg(None, None));
        assert!(!html.contains("  fontFamily:"));
        assert!(!html.contains("  theme:"));
    }

    #[test]
    fn build_html_with_font() {
        let html = build_html(&cfg(Some("Arial"), None));
        assert!(html.contains("fontFamily: \"Arial\""));
    }

    #[test]
    fn build_html_font_single_quote_is_escaped() {
        let html = build_html(&cfg(Some("'; alert('xss'); '"), None));
        assert!(html.contains("fontFamily: \"'; alert('xss'); '\""));
        assert!(!html.contains("fontFamily: '"));
    }

    #[test]
    fn build_html_font_double_quote_is_escaped() {
        let html = build_html(&cfg(Some("Font\"Name"), None));
        assert!(html.contains("fontFamily: \"Font\\\"Name\""));
    }

    #[test]
    fn build_html_font_backslash_is_escaped() {
        let html = build_html(&cfg(Some("Font\\Name"), None));
        assert!(html.contains("fontFamily: \"Font\\\\Name\""));
    }

    #[test]
    fn build_html_with_theme() {
        let html = build_html(&cfg(None, Some("dark")));
        assert!(html.contains("theme: \"dark\""));
    }

    #[test]
    fn build_html_font_closing_script_tag_is_escaped() {
        // `</script>` を埋め込まれても script ブロックから抜けられないこと
        let html = build_html(&cfg(Some("</script><script>alert(1)//"), None));
        assert!(!html.contains("</script><script>"));
        assert!(html.contains("\\u003c/script\\u003e\\u003cscript\\u003e"));
    }

    #[test]
    fn build_html_theme_closing_script_tag_is_escaped() {
        let html = build_html(&cfg(None, Some("</script>")));
        assert!(!html.contains("theme: \"</script>\""));
        assert!(html.contains("\\u003c/script\\u003e"));
    }

    fn cfg_with_config_json(json: &str) -> RenderConfig {
        RenderConfig {
            config_json: Some(json.to_string()),
            ..Default::default()
        }
    }

    #[test]
    fn build_html_no_config_json_uses_empty_spread() {
        let html = build_html(&cfg(None, None));
        assert!(html.contains("...{}"));
    }

    #[test]
    fn build_html_with_config_json() {
        let html = build_html(&cfg_with_config_json(r#"{"flowchart":{"curve":"basis"}}"#));
        assert!(html.contains(r#"...{"flowchart":{"curve":"basis"}}"#));
    }

    #[test]
    fn build_html_config_json_closing_script_tag_is_escaped() {
        let html = build_html(&cfg_with_config_json(r#"{"fontFamily":"a</script>b"}"#));
        // config_json 内の </script> がエスケープされていること
        assert!(html.contains("\\u003c/script\\u003e"));
        // エスケープ前の文字列が config_json として埋め込まれていないこと
        assert!(!html.contains(r#""a</script>b""#));
    }

    #[test]
    fn build_html_config_json_cli_flag_comes_after_spread() {
        // spread より後に個別フラグが来ることで CLI フラグが優先されることを確認
        let config = RenderConfig {
            theme: Some("forest".to_string()),
            config_json: Some(r#"{"theme":"dark"}"#.to_string()),
            ..Default::default()
        };
        let html = build_html(&config);
        let spread_pos = html.find("...{").unwrap();
        let theme_pos = html.find("theme: \"forest\"").unwrap();
        assert!(
            spread_pos < theme_pos,
            "spread must appear before CLI flag override"
        );
    }

    fn parse_ipc(s: &str) -> Result<IpcMessage, serde_json::Error> {
        serde_json::from_str(s)
    }

    #[test]
    fn ipc_message_ready_ok() {
        let m = parse_ipc(r#"{"type":"ready"}"#).unwrap();
        assert!(matches!(m, IpcMessage::Ready));
    }

    #[test]
    fn ipc_message_svg_ok() {
        let m = parse_ipc(r#"{"type":"svg","id":1,"svg":"<svg/>"}"#).unwrap();
        assert!(matches!(m, IpcMessage::Svg { id: 1, ref svg } if svg == "<svg/>"));
    }

    #[test]
    fn ipc_message_error_ok() {
        let m = parse_ipc(r#"{"type":"error","id":2,"error":"Lexical error"}"#).unwrap();
        assert!(matches!(m, IpcMessage::Error { id: 2, ref error } if error == "Lexical error"));
    }

    #[test]
    fn ipc_message_unknown_type_err() {
        assert!(parse_ipc(r#"{"type":"frobnicate"}"#).is_err());
    }

    #[test]
    fn ipc_message_missing_type_err() {
        assert!(parse_ipc(r#"{"id":1,"svg":"<svg/>"}"#).is_err());
    }

    #[test]
    fn ipc_message_svg_missing_id_err() {
        assert!(parse_ipc(r#"{"type":"svg","svg":"<svg/>"}"#).is_err());
    }

    #[test]
    fn ipc_message_svg_missing_svg_err() {
        assert!(parse_ipc(r#"{"type":"svg","id":1}"#).is_err());
    }

    #[test]
    fn ipc_message_svg_non_numeric_id_err() {
        assert!(parse_ipc(r#"{"type":"svg","id":"oops","svg":"<svg/>"}"#).is_err());
    }

    #[test]
    fn ipc_message_svg_non_string_svg_err() {
        assert!(parse_ipc(r#"{"type":"svg","id":1,"svg":42}"#).is_err());
    }

    #[test]
    fn ipc_message_error_missing_field_err() {
        assert!(parse_ipc(r#"{"type":"error","id":1}"#).is_err());
    }

    #[test]
    fn ipc_message_error_non_string_err() {
        assert!(parse_ipc(r#"{"type":"error","id":1,"error":42}"#).is_err());
    }

    // ------ read_blocks ------

    fn run_reader(bytes: &[u8]) -> (Vec<String>, Option<String>, bool) {
        let mut blocks: Vec<String> = Vec::new();
        let mut error: Option<String> = None;
        let mut ended = false;
        read_blocks(std::io::Cursor::new(bytes.to_vec()), |ev| match ev {
            LoopEvent::Block(s) => blocks.push(s),
            LoopEvent::InputEnd => ended = true,
            LoopEvent::InputError(e) => error = Some(e),
            LoopEvent::Ipc(_) => unreachable!("read_blocks never emits Ipc"),
        });
        (blocks, error, ended)
    }

    #[test]
    fn reader_empty_input() {
        let (blocks, err, ended) = run_reader(b"");
        assert!(blocks.is_empty());
        assert!(err.is_none());
        assert!(ended);
    }

    #[test]
    fn reader_single_block() {
        let (blocks, _, ended) = run_reader(b"graph LR\n  A --> B");
        assert_eq!(blocks, vec!["graph LR\n  A --> B"]);
        assert!(ended);
    }

    #[test]
    fn reader_two_blocks() {
        let (blocks, _, ended) = run_reader(b"m1\0m2");
        assert_eq!(blocks, vec!["m1", "m2"]);
        assert!(ended);
    }

    #[test]
    fn reader_three_blocks() {
        let (blocks, _, _) = run_reader(b"a\0b\0c");
        assert_eq!(blocks, vec!["a", "b", "c"]);
    }

    #[test]
    fn reader_trailing_null_is_dropped() {
        let (blocks, _, _) = run_reader(b"m1\0m2\0");
        assert_eq!(blocks, vec!["m1", "m2"]);
    }

    #[test]
    fn reader_double_trailing_null_yields_one_empty() {
        let (blocks, _, _) = run_reader(b"m1\0m2\0\0");
        assert_eq!(blocks, vec!["m1", "m2", ""]);
    }

    #[test]
    fn reader_invalid_utf8_stops_reader() {
        let (_, err, ended) = run_reader(&[0xff, 0xff]);
        assert!(err.is_some());
        assert!(!ended);
    }

    #[test]
    fn reader_invalid_utf8_after_separator() {
        let (blocks, err, _) = run_reader(&[b'a', 0, 0xff, 0xff]);
        assert_eq!(blocks, vec!["a"]);
        assert!(err.is_some());
    }
}
