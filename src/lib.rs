//! Library API for sekien: renders Mermaid diagrams to SVG using an OS-native WebView.
//!
//! The CLI (`src/main.rs`) and this library are both thin layers over
//! [`render_stream`], the single entry point for rendering.
//!
//! [`render_stream`] must be called from the process's main thread (it runs a
//! `tao` event loop, which panics on any other thread) and blocks the calling
//! thread until done. Run any concurrent work on other threads.
//!
//! ```no_run
//! use sekien::{render_stream, RenderOutcome};
//!
//! let diagrams = vec!["graph LR\n  A --> B".to_string()];
//! render_stream(diagrams, None, |outcome| {
//!     match outcome {
//!         RenderOutcome::Svg(svg) => println!("{svg}"),
//!         RenderOutcome::Error(e) => eprintln!("error: {e}"),
//!     }
//! })
//! .unwrap();
//! ```

mod render;

#[cfg(target_os = "linux")]
mod linux_display;

pub use render::{render_stream, Error, RenderOutcome, Result, MERMAID_VERSION};
