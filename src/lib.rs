//! Library API for sekien: renders Mermaid diagrams to SVG using an OS-native WebView.
//!
//! The CLI (`src/main.rs`) and this library are both thin layers over
//! [`render_stream`], the single entry point for rendering.
//!
//! ```no_run
//! use sekien::{render_stream, RenderConfig, RenderOutcome};
//!
//! let diagrams = vec!["graph LR\n  A --> B".to_string()];
//! render_stream(diagrams, &RenderConfig::default(), |id, outcome| {
//!     match outcome {
//!         RenderOutcome::Svg(svg) => println!("diagram {id}: {svg}"),
//!         RenderOutcome::Error(e) => eprintln!("diagram {id} error: {e}"),
//!     }
//! })
//! .unwrap();
//! ```

mod render;

#[cfg(target_os = "linux")]
mod linux_display;

pub use render::{render_stream, Error, RenderConfig, RenderOutcome, Result, MERMAID_VERSION};
