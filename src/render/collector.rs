//! The pure core of [`super::render_stream`]: a state machine that turns
//! [`Input`]s into [`Action`]s. See the module-level docs of [`super`].

use crate::error::Error;
use serde::Deserialize;
use std::collections::VecDeque;

/// Result of rendering a single diagram.
#[derive(Debug, PartialEq, Eq)]
pub enum RenderOutcome {
    /// Rendered successfully; the SVG markup.
    Svg(String),
    /// Mermaid failed to parse/render the diagram; the error message.
    Error(String),
}

/// An event fed into [`Collector`]. The event loop receives these through its
/// proxy (all but `WindowClosed`, which comes from the window itself).
pub(super) enum Input {
    /// The next diagram from the input.
    Block(String),
    /// The input iterator is exhausted; no more `Block`s will arrive.
    End,
    /// A raw IPC message from the WebView (JSON; see [`IpcMessage`]).
    Ipc(String),
    /// The window was closed.
    WindowClosed,
}

/// IPC messages from the WebView. Four variants, fixed by `render.html`:
///
/// - `{"type":"ready"}`: mermaid.initialize() complete
/// - `{"type":"svg","id":N,"svg":"..."}`: block N rendered successfully
/// - `{"type":"error","id":N,"error":"..."}`: block N failed to parse
/// - `{"type":"fatal","error":"..."}`: mermaid could not be loaded or
///   initialized, so "ready" will never come
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum IpcMessage {
    Ready,
    Svg { id: usize, svg: String },
    Error { id: usize, error: String },
    Fatal { error: String },
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

/// An effect that [`super::render_stream`]'s impure shell should perform, as
/// decided by [`Collector`].
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Action {
    /// Dispatch diagram `id` (`content`) to the WebView.
    Dispatch { id: usize, content: String },
    /// Report the outcome of the in-flight diagram to the caller. Outcomes are
    /// emitted in input order, so no id is attached: ids exist only to match
    /// IPC results to dispatched renders, and never leave the renderer.
    Emit(RenderOutcome),
    /// All diagrams processed; the event loop should exit.
    Done,
    /// Fatal failure; the event loop should exit and return this error.
    Fatal(Error),
}

/// Pure state machine driving [`super::render_stream`].
pub(super) struct Collector {
    /// Blocks waiting to be dispatched, with their ids.
    queue: VecDeque<(usize, String)>,
    /// Number of blocks received so far; ids are 1-origin positions.
    received: usize,
    /// Whether the input iterator is exhausted.
    end_received: bool,
    pipeline: Pipeline,
}

impl Collector {
    pub(super) fn new() -> Self {
        Self {
            queue: VecDeque::new(),
            received: 0,
            end_received: false,
            pipeline: Pipeline::NotReady,
        }
    }

    pub(super) fn handle(&mut self, input: Input) -> Vec<Action> {
        match input {
            Input::Block(content) => {
                self.received += 1;
                self.queue.push_back((self.received, content));
                self.try_dispatch_next()
            }
            Input::End => {
                self.end_received = true;
                self.try_dispatch_next()
            }
            Input::Ipc(raw) => match serde_json::from_str(&raw) {
                Ok(msg) => self.on_ipc(msg),
                Err(e) => fatal(format!("malformed IPC: {e} (raw: {raw})")),
            },
            Input::WindowClosed => fatal("window closed before rendering completed".into()),
        }
    }

    fn on_ipc(&mut self, msg: IpcMessage) -> Vec<Action> {
        match msg {
            IpcMessage::Ready => self.on_ready(),
            IpcMessage::Svg { id, svg } => self.on_render_done(id, RenderOutcome::Svg(svg)),
            IpcMessage::Error { id, error } => self.on_render_done(id, RenderOutcome::Error(error)),
            IpcMessage::Fatal { error } => fatal(format!("page failed to load: {error}")),
        }
    }

    fn on_ready(&mut self) -> Vec<Action> {
        if self.pipeline != Pipeline::NotReady {
            return self.unexpected_ipc("received ready");
        }
        self.pipeline = Pipeline::Idle;
        self.try_dispatch_next()
    }

    fn on_render_done(&mut self, id: usize, outcome: RenderOutcome) -> Vec<Action> {
        if self.pipeline != Pipeline::Awaiting(id) {
            return self.unexpected_ipc(&format!("received result for id {id}"));
        }
        self.pipeline = Pipeline::Idle;
        let mut actions = vec![Action::Emit(outcome)];
        actions.extend(self.try_dispatch_next());
        actions
    }

    fn unexpected_ipc(&self, what: &str) -> Vec<Action> {
        fatal(format!(
            "malformed IPC: {what} but pipeline state is {:?}",
            self.pipeline
        ))
    }

    /// Dispatches the next queued block if conditions are met, or signals `Done`
    /// if the queue is empty and the input is exhausted.
    fn try_dispatch_next(&mut self) -> Vec<Action> {
        if self.pipeline != Pipeline::Idle {
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

fn fatal(message: String) -> Vec<Action> {
    vec![Action::Fatal(Error::Internal(message))]
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(matches!(
            parse_ipc(r#"{"type":"fatal","error":"TypeError"}"#).unwrap(),
            IpcMessage::Fatal { ref error } if error == "TypeError"
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
            r#"{"type":"fatal"}"#,
        ] {
            assert!(parse_ipc(raw).is_err(), "expected error for {raw}");
        }
    }

    fn block(content: &str) -> Input {
        Input::Block(content.to_string())
    }

    fn ready() -> Input {
        Input::Ipc(r#"{"type":"ready"}"#.to_string())
    }

    fn svg(id: usize, s: &str) -> Input {
        Input::Ipc(serde_json::json!({"type": "svg", "id": id, "svg": s}).to_string())
    }

    fn error(id: usize, s: &str) -> Input {
        Input::Ipc(serde_json::json!({"type": "error", "id": id, "error": s}).to_string())
    }

    fn dispatch(id: usize, content: &str) -> Action {
        Action::Dispatch {
            id,
            content: content.to_string(),
        }
    }

    fn emit_svg(s: &str) -> Action {
        Action::Emit(RenderOutcome::Svg(s.to_string()))
    }

    fn emit_err(s: &str) -> Action {
        Action::Emit(RenderOutcome::Error(s.to_string()))
    }

    fn is_fatal(actions: &[Action]) -> bool {
        matches!(actions, [Action::Fatal(Error::Internal(_))])
    }

    #[test]
    fn collector_not_ready_queues_without_dispatch() {
        let mut c = Collector::new();
        assert_eq!(c.handle(block("a")), vec![]);
    }

    #[test]
    fn collector_ready_dispatches_queued_block() {
        let mut c = Collector::new();
        c.handle(block("a"));
        assert_eq!(c.handle(ready()), vec![dispatch(1, "a")]);
    }

    #[test]
    fn collector_svg_result_emits_and_dispatches_next() {
        let mut c = Collector::new();
        c.handle(block("a"));
        c.handle(block("b"));
        c.handle(ready()); // dispatches 1

        assert_eq!(
            c.handle(svg(1, "<svg/>")),
            vec![emit_svg("<svg/>"), dispatch(2, "b")]
        );
    }

    #[test]
    fn collector_error_result_is_emitted() {
        let mut c = Collector::new();
        c.handle(block("bogus"));
        c.handle(ready());

        assert_eq!(
            c.handle(error(1, "Lexical error")),
            vec![emit_err("Lexical error")]
        );
    }

    #[test]
    fn collector_done_after_input_end_and_drain() {
        let mut c = Collector::new();
        c.handle(block("a"));
        c.handle(ready()); // dispatches 1
        c.handle(Input::End);
        assert_eq!(
            c.handle(svg(1, "<svg/>")),
            vec![emit_svg("<svg/>"), Action::Done]
        );
    }

    #[test]
    fn collector_input_end_before_ready_then_empty() {
        let mut c = Collector::new();
        c.handle(Input::End);
        assert_eq!(c.handle(ready()), vec![Action::Done]);
    }

    #[test]
    fn collector_unexpected_ipc_result_is_fatal() {
        // A result arrives before any block was dispatched (pipeline NotReady).
        assert!(is_fatal(&Collector::new().handle(svg(1, "<svg/>"))));

        // A result arrives for a different id than the one in flight.
        let mut c = Collector::new();
        c.handle(block("a"));
        c.handle(ready()); // awaiting 1
        assert!(is_fatal(&c.handle(svg(2, "<svg/>"))));
    }

    #[test]
    fn collector_second_ready_is_fatal() {
        // While idle.
        let mut c = Collector::new();
        c.handle(ready());
        assert!(is_fatal(&c.handle(ready())));

        // While a render is in flight: must not dispatch a second one.
        let mut c = Collector::new();
        c.handle(block("a"));
        c.handle(block("b"));
        c.handle(ready()); // awaiting 1
        assert!(is_fatal(&c.handle(ready())));
    }

    #[test]
    fn collector_malformed_ipc_is_fatal() {
        assert!(is_fatal(
            &Collector::new().handle(Input::Ipc("not json".into()))
        ));
    }

    #[test]
    fn collector_page_load_failure_is_fatal_in_any_state() {
        let fatal_msg = || Input::Ipc(r#"{"type":"fatal","error":"TypeError: x"}"#.into());
        // Before ready: the usual case (the import failed).
        let mut c = Collector::new();
        c.handle(block("a"));
        let actions = c.handle(fatal_msg());
        assert!(
            matches!(actions.as_slice(), [Action::Fatal(Error::Internal(m))] if m.contains("TypeError: x")),
            "{actions:?}"
        );
        // Idle, and with a render in flight.
        let mut c = Collector::new();
        c.handle(ready());
        assert!(is_fatal(&c.handle(fatal_msg())));
        let mut c = Collector::new();
        c.handle(block("a"));
        c.handle(ready());
        assert!(is_fatal(&c.handle(fatal_msg())));
    }

    #[test]
    fn collector_window_closed_is_fatal() {
        assert!(is_fatal(&Collector::new().handle(Input::WindowClosed)));
    }

    #[test]
    fn collector_blocks_arriving_after_ready_are_dispatched_in_order() {
        let mut c = Collector::new();
        c.handle(ready()); // Idle, queue empty -> no action yet
        assert_eq!(c.handle(block("a")), vec![dispatch(1, "a")]);
        // Second block queues behind the in-flight render.
        assert_eq!(c.handle(block("b")), vec![]);
        assert_eq!(
            c.handle(svg(1, "<svg/>")),
            vec![emit_svg("<svg/>"), dispatch(2, "b")]
        );
    }
}
