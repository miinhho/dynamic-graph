//! Text-to-graph ingestion via LLM.
//!
//! [`TextIngestor`] takes a block of natural-language text and asks an LLM
//! to extract named entities of the requested kinds. The result is a
//! `Vec<`[`ExtractedNode`]`>` that maps directly onto a
//! `Simulation::ingest_cooccurrence` call.
//!
//! ## Example
//!
//! ```no_run
//! use graph_llm::{AnthropicClient, TextIngestor};
//!
//! let client = AnthropicClient::from_env().unwrap();
//! let ingestor = TextIngestor::new(&client);
//!
//! let text = "Alice and Bob co-authored a paper with Carol at MIT.";
//! let nodes = ingestor.extract(text, &["PERSON", "ORG"]).unwrap();
//! // → [ExtractedNode { name: "Alice", kind: "PERSON", .. }, ...]
//! ```

use std::collections::HashMap;

use crate::client::LlmClient;
use crate::error::LlmError;

// ─── Output type ──────────────────────────────────────────────────────────────

/// A single entity extracted from text by the LLM.
#[derive(Debug, Clone, PartialEq)]
pub struct ExtractedNode {
    /// Human-readable name of the entity (e.g. `"Alice"`, `"MIT"`).
    pub name: String,
    /// The locus-kind label the caller requested (e.g. `"PERSON"`, `"ORG"`).
    pub kind: String,
    /// Domain properties extracted by the LLM (free-form key/value strings).
    pub properties: HashMap<String, String>,
}

// ─── Ingestor ─────────────────────────────────────────────────────────────────

/// Extracts graph nodes from natural-language text using an LLM.
///
/// Borrow an [`LlmClient`] reference to construct one. The ingestor is
/// stateless — you can call [`extract`](TextIngestor::extract) multiple times
/// on different texts with the same client.
pub struct TextIngestor<'c> {
    client: &'c dyn LlmClient,
}

impl<'c> TextIngestor<'c> {
    /// Create a new ingestor backed by `client`.
    pub fn new(client: &'c dyn LlmClient) -> Self {
        Self { client }
    }

    /// Extract entities of the given `kinds` from `text`.
    ///
    /// The LLM is instructed to respond with a JSON object matching:
    /// ```json
    /// { "nodes": [{ "name": "...", "kind": "...", "properties": { "key": "val" } }] }
    /// ```
    ///
    /// Non-string property values in the LLM response are silently skipped.
    pub fn extract(&self, text: &str, kinds: &[&str]) -> Result<Vec<ExtractedNode>, LlmError> {
        let kinds_list = kinds.join(", ");
        let system = format!(
            "You are a knowledge graph entity extractor. \
             Extract all named entities from the given text that belong to one of these kinds: {kinds_list}.\n\
             Respond with ONLY a JSON object in this exact format — no explanation, no markdown fences:\n\
             {{\"nodes\": [{{\"name\": \"entity name\", \"kind\": \"ENTITY_KIND\", \"properties\": {{\"key\": \"value\"}}}}]}}\n\
             - Use only the kinds listed above.\n\
             - Include only entities explicitly mentioned in the text.\n\
             - Properties are optional; include relevant domain attributes (e.g. role, location)."
        );

        let response = self.client.complete(&system, text)?;
        let json_str = extract_json_object(&response)?;

        let parsed: serde_json::Value = serde_json::from_str(&json_str)
            .map_err(|e| LlmError::ParseError(format!("invalid JSON: {e}")))?;

        let nodes = parsed["nodes"]
            .as_array()
            .ok_or_else(|| LlmError::ParseError("missing top-level 'nodes' array".to_owned()))?;

        nodes
            .iter()
            .map(|n| {
                let name = n["name"]
                    .as_str()
                    .ok_or_else(|| LlmError::ParseError("node missing 'name' field".to_owned()))?
                    .trim()
                    .to_owned();
                let kind = n["kind"]
                    .as_str()
                    .ok_or_else(|| LlmError::ParseError("node missing 'kind' field".to_owned()))?
                    .trim()
                    .to_owned();
                let properties = n["properties"]
                    .as_object()
                    .map(|obj| {
                        obj.iter()
                            .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_owned())))
                            .collect()
                    })
                    .unwrap_or_default();

                Ok(ExtractedNode { name, kind, properties })
            })
            .collect()
    }
}

// ─── Helpers ──────────────────────────────────────────────────────────────────

/// Find the outermost `{...}` in `s` and return that slice.
///
/// Handles cases where the model wraps its answer in prose or code fences.
pub(crate) fn extract_json_object(s: &str) -> Result<String, LlmError> {
    let start = s
        .find('{')
        .ok_or_else(|| LlmError::ParseError("no JSON object in LLM response".to_owned()))?;
    let end = s
        .rfind('}')
        .ok_or_else(|| LlmError::ParseError("no closing brace in LLM response".to_owned()))?;
    if end < start {
        return Err(LlmError::ParseError(
            "closing brace before opening brace".to_owned(),
        ));
    }
    Ok(s[start..=end].to_owned())
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::MockLlmClient;

    const MOCK_JSON: &str = r#"
    {
        "nodes": [
            {"name": "Alice", "kind": "PERSON", "properties": {"role": "author"}},
            {"name": "MIT",   "kind": "ORG",    "properties": {}}
        ]
    }
    "#;

    #[test]
    fn extracts_nodes_from_mock_json() {
        let client = MockLlmClient::new(MOCK_JSON);
        let ingestor = TextIngestor::new(&client);
        let nodes = ingestor
            .extract("Alice wrote a paper at MIT.", &["PERSON", "ORG"])
            .unwrap();

        assert_eq!(nodes.len(), 2);
        assert_eq!(nodes[0].name, "Alice");
        assert_eq!(nodes[0].kind, "PERSON");
        assert_eq!(nodes[0].properties.get("role").map(|s| s.as_str()), Some("author"));
        assert_eq!(nodes[1].name, "MIT");
        assert_eq!(nodes[1].kind, "ORG");
    }

    #[test]
    fn tolerates_json_wrapped_in_prose() {
        let wrapped = "Sure! Here is the result:\n```json\n{\"nodes\":[]}\n```";
        let client = MockLlmClient::new(wrapped);
        let ingestor = TextIngestor::new(&client);
        let nodes = ingestor.extract("no entities here", &["PERSON"]).unwrap();
        assert!(nodes.is_empty());
    }

    #[test]
    fn parse_error_on_missing_nodes_key() {
        let bad = r#"{"result": []}"#;
        let client = MockLlmClient::new(bad);
        let ingestor = TextIngestor::new(&client);
        let err = ingestor.extract("text", &["PERSON"]).unwrap_err();
        assert!(matches!(err, LlmError::ParseError(_)));
    }

    #[test]
    fn extract_json_object_strips_surrounding_text() {
        let s = "some text {\"key\": 1} more text";
        assert_eq!(extract_json_object(s).unwrap(), r#"{"key": 1}"#);
    }
}
