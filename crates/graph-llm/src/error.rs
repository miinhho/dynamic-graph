//! Error type for LLM operations.

use std::fmt;

/// Errors that can occur when calling an LLM backend.
#[derive(Debug)]
pub enum LlmError {
    /// Network or transport-level failure.
    Http(String),
    /// The API returned a non-2xx status code.
    ApiError { status: u16, message: String },
    /// The response could not be parsed as expected.
    ParseError(String),
}

impl fmt::Display for LlmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LlmError::Http(msg) => write!(f, "HTTP error: {msg}"),
            LlmError::ApiError { status, message } => {
                write!(f, "API error {status}: {message}")
            }
            LlmError::ParseError(msg) => write!(f, "parse error: {msg}"),
        }
    }
}

impl std::error::Error for LlmError {}
