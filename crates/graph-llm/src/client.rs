//! LLM client trait and implementations.
//!
//! [`LlmClient`] is the single abstraction the rest of this crate programs
//! against. Provide your own implementation to use any backend (OpenAI, local
//! Ollama, etc.), or enable the `anthropic` feature to get [`AnthropicClient`]
//! out of the box.
//!
//! [`MockLlmClient`] is always available and returns a fixed response string.
//! Use it in tests.

use crate::error::LlmError;

// ─── Trait ────────────────────────────────────────────────────────────────────

/// A minimal LLM interface: system prompt + user message → assistant reply.
///
/// Implementations are expected to be **blocking** (not async). Users who need
/// async can wrap `complete` in `tokio::task::spawn_blocking`.
pub trait LlmClient: Send + Sync {
    /// Send `user` with `system` as the system prompt and return the model's
    /// text reply.
    fn complete(&self, system: &str, user: &str) -> Result<String, LlmError>;
}

// ─── Anthropic implementation ─────────────────────────────────────────────────

/// Blocking HTTP client for the Anthropic Messages API.
///
/// Requires the `anthropic` feature (enabled by default). Pass your API key
/// via `ANTHROPIC_API_KEY` or supply it directly with [`AnthropicClient::new`].
///
/// ```no_run
/// use graph_llm::AnthropicClient;
/// let client = AnthropicClient::from_env().expect("ANTHROPIC_API_KEY not set");
/// ```
#[cfg(feature = "anthropic")]
pub struct AnthropicClient {
    api_key: String,
    model: String,
    http: reqwest::blocking::Client,
}

#[cfg(feature = "anthropic")]
impl AnthropicClient {
    /// Create a client using the default model (`claude-sonnet-4-6`).
    pub fn new(api_key: impl Into<String>) -> Self {
        Self::with_model(api_key, "claude-sonnet-4-6")
    }

    /// Create a client targeting a specific model ID.
    pub fn with_model(api_key: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            model: model.into(),
            http: reqwest::blocking::Client::new(),
        }
    }

    /// Read the API key from the `ANTHROPIC_API_KEY` environment variable.
    ///
    /// Returns `None` if the variable is not set.
    pub fn from_env() -> Option<Self> {
        std::env::var("ANTHROPIC_API_KEY").ok().map(Self::new)
    }
}

#[cfg(feature = "anthropic")]
impl LlmClient for AnthropicClient {
    fn complete(&self, system: &str, user: &str) -> Result<String, LlmError> {
        let body = serde_json::json!({
            "model": self.model,
            "max_tokens": 1024,
            "system": system,
            "messages": [{ "role": "user", "content": user }]
        });

        let response = self
            .http
            .post("https://api.anthropic.com/v1/messages")
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .map_err(|e| LlmError::Http(e.to_string()))?;

        let status = response.status();
        if !status.is_success() {
            let err_body: serde_json::Value = response.json().unwrap_or_default();
            let message = err_body["error"]["message"]
                .as_str()
                .unwrap_or("unknown error")
                .to_owned();
            return Err(LlmError::ApiError {
                status: status.as_u16(),
                message,
            });
        }

        let resp_body: serde_json::Value = response
            .json()
            .map_err(|e| LlmError::ParseError(e.to_string()))?;

        let text = resp_body["content"][0]["text"]
            .as_str()
            .ok_or_else(|| LlmError::ParseError("missing content[0].text in response".to_owned()))?
            .to_owned();

        Ok(text)
    }
}

// ─── Ollama implementation ────────────────────────────────────────────────────

/// Blocking HTTP client for a local [Ollama](https://ollama.com/) server.
///
/// Requires the `ollama` feature. Connects to `http://localhost:11434` by
/// default. Use [`OllamaClient::with_base_url`] to point at a remote host.
///
/// ```no_run
/// # #[cfg(feature = "ollama")]
/// # {
/// use graph_llm::OllamaClient;
/// let client = OllamaClient::new("llama3:8b");
/// # }
/// ```
#[cfg(feature = "ollama")]
pub struct OllamaClient {
    model: String,
    base_url: String,
    http: reqwest::blocking::Client,
}

#[cfg(feature = "ollama")]
impl OllamaClient {
    /// Create a client targeting `model` at `http://localhost:11434`.
    pub fn new(model: impl Into<String>) -> Self {
        Self::with_base_url(model, "http://localhost:11434")
    }

    /// Create a client targeting `model` at a custom base URL.
    pub fn with_base_url(model: impl Into<String>, base_url: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            base_url: base_url.into(),
            http: reqwest::blocking::Client::new(),
        }
    }
}

#[cfg(feature = "ollama")]
impl LlmClient for OllamaClient {
    fn complete(&self, system: &str, user: &str) -> Result<String, LlmError> {
        let url = format!("{}/api/chat", self.base_url);

        let body = serde_json::json!({
            "model": self.model,
            "stream": false,
            "messages": [
                { "role": "system", "content": system },
                { "role": "user",   "content": user   }
            ]
        });

        let response = self
            .http
            .post(&url)
            .json(&body)
            .send()
            .map_err(|e| LlmError::Http(e.to_string()))?;

        let status = response.status();
        if !status.is_success() {
            let err_body: serde_json::Value = response.json().unwrap_or_default();
            let message = err_body["error"]
                .as_str()
                .unwrap_or("unknown error")
                .to_owned();
            return Err(LlmError::ApiError {
                status: status.as_u16(),
                message,
            });
        }

        let resp_body: serde_json::Value = response
            .json()
            .map_err(|e| LlmError::ParseError(e.to_string()))?;

        let text = resp_body["message"]["content"]
            .as_str()
            .ok_or_else(|| LlmError::ParseError("missing message.content in response".to_owned()))?
            .to_owned();

        Ok(text)
    }
}

// ─── Mock implementation ───────────────────────────────────────────────────────

/// An [`LlmClient`] that always returns a fixed response string.
///
/// Useful for unit tests that verify prompt construction or downstream
/// parsing without making real API calls.
///
/// ```
/// use graph_llm::MockLlmClient;
/// use graph_llm::LlmClient;
///
/// let client = MockLlmClient::new("hello");
/// assert_eq!(client.complete("sys", "user").unwrap(), "hello");
/// ```
pub struct MockLlmClient {
    response: String,
}

impl MockLlmClient {
    /// Create a mock that always returns `response`.
    pub fn new(response: impl Into<String>) -> Self {
        Self {
            response: response.into(),
        }
    }
}

impl LlmClient for MockLlmClient {
    fn complete(&self, _system: &str, _user: &str) -> Result<String, LlmError> {
        Ok(self.response.clone())
    }
}

/// An [`LlmClient`] that captures every `(system, user)` pair it is
/// handed, then returns a configurable fixed response. Useful for
/// prompt-structure regression tests — assert the prompt contains the
/// expected fields without depending on any model's output.
///
/// ```
/// use graph_llm::{CapturingLlmClient, LlmClient};
///
/// let client = CapturingLlmClient::new("response");
/// let _ = client.complete("system text", "user text").unwrap();
/// let calls = client.calls();
/// assert_eq!(calls.len(), 1);
/// assert_eq!(calls[0].0, "system text");
/// assert_eq!(calls[0].1, "user text");
/// ```
pub struct CapturingLlmClient {
    response: String,
    calls: std::sync::Mutex<Vec<(String, String)>>,
}

impl CapturingLlmClient {
    /// Create a client that returns `response` on every call.
    pub fn new(response: impl Into<String>) -> Self {
        Self {
            response: response.into(),
            calls: std::sync::Mutex::new(Vec::new()),
        }
    }

    /// Snapshot of every `(system, user)` pair seen so far, oldest
    /// first.
    pub fn calls(&self) -> Vec<(String, String)> {
        self.calls.lock().unwrap().clone()
    }

    /// The most recent `(system, user)` pair, or `None` if the client
    /// has never been called.
    pub fn last(&self) -> Option<(String, String)> {
        self.calls.lock().unwrap().last().cloned()
    }
}

impl LlmClient for CapturingLlmClient {
    fn complete(&self, system: &str, user: &str) -> Result<String, LlmError> {
        self.calls
            .lock()
            .unwrap()
            .push((system.to_owned(), user.to_owned()));
        Ok(self.response.clone())
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_returns_fixed_response() {
        let c = MockLlmClient::new("pong");
        assert_eq!(c.complete("sys", "ping").unwrap(), "pong");
    }

    #[test]
    fn capturing_records_calls_in_order() {
        let c = CapturingLlmClient::new("ok");
        c.complete("s1", "u1").unwrap();
        c.complete("s2", "u2").unwrap();
        let calls = c.calls();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0], ("s1".into(), "u1".into()));
        assert_eq!(calls[1], ("s2".into(), "u2".into()));
        assert_eq!(c.last(), Some(("s2".into(), "u2".into())));
    }
}
