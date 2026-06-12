//! Library API for sekien: renders Mermaid diagrams to SVG using an OS-native WebView.
//!
//! The CLI (`src/main.rs`) and this library are both thin layers over
//! [`render_stream`], the single entry point for rendering.

mod render;

#[cfg(target_os = "linux")]
mod linux_display;

pub use render::{render_stream, Error, RenderConfig, RenderOutcome, Result, MERMAID_VERSION};
