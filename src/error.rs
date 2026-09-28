//! The crate's error type. A leaf module: everything else may depend on it.

/// Fatal failure of [`crate::render_stream`] itself (not a per-diagram render
/// error, which is reported via [`crate::RenderOutcome::Error`]).
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
