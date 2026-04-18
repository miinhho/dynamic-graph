//! Graph-grounded question answering.
//!
//! [`answer_with_graph`] answers a natural-language question using the
//! knowledge graph as an information source. Unlike dumping a full subgraph
//! into the prompt, it does a **targeted retrieval**: only the top-K most
//! active relationships touching entities mentioned in the question are
//! injected as context.
//!
//! ## How it works
//!
//! 1. Scan the [`NameMap`] for loci whose names appear in the question
//!    (case-insensitive substring match).
//! 2. Expand one hop: collect all loci reachable from the matched set.
//! 3. Gather every relationship that touches the expanded set.
//! 4. Sort by `activity` descending, keep the top `top_k`.
//! 5. Format as a compact "Known facts:" block and call the LLM.
//!
//! The context injected into the prompt is at most `top_k` lines —
//! typically a handful of tokens, not a subgraph dump.
//!
//! ## Example
//!
//! ```ignore
//! use graph_llm::{OllamaClient, answer_with_graph};
//! use graph_query::NameMap;
//!
//! let client = OllamaClient::new("llama3:8b");
//! let names  = NameMap::from_world(&world);
//! let answer = answer_with_graph(&client, "Who is Alice connected to?", &world, &names, 5)?;
//! println!("{answer}");
//! ```

use graph_core::{Endpoints, LocusId};
use graph_query::{NameMap, reachable_from};
use graph_world::World;
use rustc_hash::FxHashSet;

use crate::client::LlmClient;
use crate::error::LlmError;

// ─── Public API ───────────────────────────────────────────────────────────────

/// Answer `question` using the knowledge graph as a targeted information source.
///
/// Entities mentioned in the question are matched against `names` (case-
/// insensitive substring). Their immediate neighbours are included for context.
/// Only the `top_k` most active relationships are injected into the prompt.
///
/// If no entities match, the question is forwarded to the LLM without graph
/// context.
pub fn answer_with_graph(
    client: &dyn LlmClient,
    question: &str,
    world: &World,
    names: &NameMap,
    top_k: usize,
) -> Result<String, LlmError> {
    // 1. Find loci whose names appear in the question.
    let q_lower = question.to_lowercase();
    let matched: Vec<LocusId> = world
        .loci()
        .iter()
        .filter(|l| {
            names
                .get(l.id)
                .map(|n| q_lower.contains(&n.to_lowercase()))
                .unwrap_or(false)
        })
        .map(|l| l.id)
        .collect();

    // 2. Expand one hop.
    let mut relevant: FxHashSet<LocusId> = matched.iter().copied().collect();
    for &root in &matched {
        for neighbor in reachable_from(world, root, 1) {
            relevant.insert(neighbor);
        }
    }

    // 3. Collect relationships touching the relevant set, sort by activity.
    let mut facts: Vec<(f32, String)> = world
        .relationships()
        .iter()
        .filter(|rel| {
            let (a, b) = endpoints(rel);
            relevant.contains(&a) || relevant.contains(&b)
        })
        .map(|rel| {
            let (a, b) = endpoints(rel);
            let line = format!(
                "  {} ↔ {}  (strength: {:.2})",
                names.name(a),
                names.name(b),
                rel.activity(),
            );
            (rel.activity(), line)
        })
        .collect();

    facts.sort_by(|x, y| y.0.partial_cmp(&x.0).unwrap_or(std::cmp::Ordering::Equal));
    facts.truncate(top_k);

    // 4. Build prompt.
    let system = "You are a helpful assistant with access to a knowledge graph. \
                  Use the provided graph facts to answer the question concisely and factually. \
                  If the facts don't cover the question, say so.";

    let user = if facts.is_empty() {
        format!("Question: {question}")
    } else {
        let context = facts
            .iter()
            .map(|(_, line)| line.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        format!("Known graph facts:\n{context}\n\nQuestion: {question}")
    };

    client.complete(system, &user)
}

// ─── Helper ───────────────────────────────────────────────────────────────────

fn endpoints(rel: &graph_core::Relationship) -> (LocusId, LocusId) {
    match rel.endpoints {
        Endpoints::Symmetric { a, b } => (a, b),
        Endpoints::Directed { from, to } => (from, to),
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::MockLlmClient;
    use graph_query::NameMap;

    #[test]
    fn no_match_forwards_question_without_context() {
        let client = MockLlmClient::new("I don't know.");
        let world = graph_world::World::new();
        let names = NameMap::default();
        let result = answer_with_graph(&client, "Who is Zara?", &world, &names, 5).unwrap();
        assert_eq!(result, "I don't know.");
    }
}
