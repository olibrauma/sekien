//! The pure core of [`super::render_stream`]: a state machine that turns input
//! events into [`Action`]s. See the module-level docs of [`super`].

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

/// IPC messages from the WebView. Three variants, fixed by `render.html`:
///
/// - `{"type":"ready"}`: mermaid.initialize() complete
/// - `{"type":"svg","id":N,"svg":"..."}`: block N rendered successfully
/// - `{"type":"error","id":N,"error":"..."}`: block N failed to parse
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub(super) enum IpcMessage {
    Ready,
    Svg { id: usize, svg: String },
    Error { id: usize, error: String },
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

/// An effect that [`super::render_stream`]'s impure shell should perform, as decided by
/// [`Collector`].
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

struct PendingBlock {
    id: usize,
    content: String,
}

/// Pure state machine driving [`super::render_stream`]. See the module-level docs for
/// the overall flow.
pub(super) struct Collector {
    queue: VecDeque<PendingBlock>,
    /// Whether the input iterator is exhausted.
    end_received: bool,
    pipeline: Pipeline,
}

impl Collector {
    pub(super) fn new() -> Self {
        Self {
            queue: VecDeque::new(),
            end_received: false,
            pipeline: Pipeline::NotReady,
        }
    }

    pub(super) fn on_block(&mut self, id: usize, content: String) -> Vec<Action> {
        self.queue.push_back(PendingBlock { id, content });
        self.try_dispatch_next()
    }

    pub(super) fn on_input_end(&mut self) -> Vec<Action> {
        self.end_received = true;
        self.try_dispatch_next()
    }

    pub(super) fn on_ipc(&mut self, msg: IpcMessage) -> Vec<Action> {
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
        let mut actions = vec![Action::Emit(outcome)];
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

    fn emit_svg(s: &str) -> Action {
        Action::Emit(RenderOutcome::Svg(s.to_string()))
    }

    fn emit_err(s: &str) -> Action {
        Action::Emit(RenderOutcome::Error(s.to_string()))
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
            vec![emit_svg("<svg/>"), dispatch(2, "b")]
        );
    }

    #[test]
    fn collector_error_result_is_emitted() {
        let mut c = Collector::new();
        c.on_block(1, "bogus".into());
        c.on_ipc(ready());

        assert_eq!(
            c.on_ipc(error(1, "Lexical error")),
            vec![emit_err("Lexical error")]
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
            vec![emit_svg("<svg/>"), Action::Done]
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
            vec![emit_svg("<svg/>"), dispatch(2, "b")]
        );
    }
}
