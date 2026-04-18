//! Causal narration: turn structured query results into human-readable prose.
//!
//! Both functions in this module accept **pre-computed** results from
//! `graph-query` so callers can inspect the structured data before (or
//! instead of) requesting a narrative.
//!
//! ## Counterfactual narration
//!
//! ```ignore
//! use graph_llm::{AnthropicClient, narrate_counterfactual};
//! use graph_query::{relationships_absent_without, NameMap};
//!
//! // 1. Query — structured result
//! let absent_ids  = relationships_absent_without(&world, &root_changes);
//! let names       = NameMap::from_world(&world);
//! let name_pairs: Vec<(String, String)> = absent_ids.iter().map(|&id| {
//!     let rel = world.relationships().get(id).unwrap();
//!     let (a, b) = rel.endpoint_ids();
//!     (names.name(a), names.name(b))
//! }).collect();
//!
//! // 2. Narrate — LLM prose
//! let client = AnthropicClient::from_env().unwrap();
//! let text = narrate_counterfactual(&client, &name_pairs).unwrap();
//! println!("{text}");
//! ```
//!
//! ## Entity-deviation narration
//!
//! ```ignore
//! use graph_llm::{AnthropicClient, narrate_entity_deviations};
//! use graph_query::{entity_deviations_since, NameMap};
//!
//! let diffs  = entity_deviations_since(&world, baseline);
//! let names  = NameMap::from_world(&world);
//! let client = AnthropicClient::from_env().unwrap();
//! let text   = narrate_entity_deviations(&client, &diffs, &names).unwrap();
//! println!("{text}");
//! ```

use graph_query::{EntityDiff, NameMap};

use crate::client::LlmClient;
use crate::error::LlmError;

// ─── Counterfactual narration ─────────────────────────────────────────────────

/// Ask the LLM to explain which relationships would vanish without the given
/// root stimuli.
///
/// `removed_relationships` is a list of `(name_a, name_b)` pairs — the
/// human-readable endpoint names of each relationship that would be absent.
/// Build this from the output of [`graph_query::relationships_absent_without`]
/// plus a [`NameMap`] (or any other source of locus names).
///
/// Returns a short narrative suitable for displaying to end users.
/// If `removed_relationships` is empty, returns a canned "no impact" message
/// without calling the LLM.
pub fn narrate_counterfactual(
    client: &dyn LlmClient,
    removed_relationships: &[(String, String)],
) -> Result<String, LlmError> {
    if removed_relationships.is_empty() {
        return Ok("No relationships would be affected by removing these stimuli.".to_owned());
    }

    let rel_list = removed_relationships
        .iter()
        .map(|(a, b)| format!("  - {a} ↔ {b}"))
        .collect::<Vec<_>>()
        .join("\n");

    let system = "\
        You are analyzing a dynamic knowledge graph simulation. \
        Given a list of relationships that would not exist without a specific stimulus, \
        write a concise, domain-neutral explanation (2–4 sentences) of what this means \
        causally: what the stimulus was responsible for creating and what its absence \
        would imply for the overall graph structure.";

    let user = format!(
        "If the triggering stimulus had never fired, the following relationships \
         would not exist in the graph:\n\n{rel_list}\n\n\
         Explain the causal significance of this stimulus."
    );

    client.complete(system, &user)
}

// ─── Entity-deviation narration ───────────────────────────────────────────────

/// Ask the LLM to summarise entity-level changes since a baseline batch.
///
/// `diffs` is the output of [`graph_query::entity_deviations_since`]; `names`
/// is a [`NameMap`] built from the same world for resolving locus IDs to
/// human-readable labels.
///
/// Returns a short narrative. Returns a canned message without calling the
/// LLM when `diffs` is empty.
pub fn narrate_entity_deviations(
    client: &dyn LlmClient,
    diffs: &[EntityDiff],
    names: &NameMap,
) -> Result<String, LlmError> {
    if diffs.is_empty() {
        return Ok("No entity changes have been detected since the baseline.".to_owned());
    }

    let summary = format_entity_diffs(diffs, names);

    let system = "\
        You are analyzing structural changes in a dynamic knowledge graph. \
        Summarise the entity-level changes in plain language. Focus on the most \
        significant transitions (new entities, dormancy, large membership shifts, \
        large coherence changes). Be concise (3–5 sentences).";

    let user = format!(
        "The following entity changes were detected since the baseline batch:\n\n{summary}"
    );

    client.complete(system, &user)
}

// ─── Formatting helpers ───────────────────────────────────────────────────────

fn format_entity_diffs(diffs: &[EntityDiff], names: &NameMap) -> String {
    diffs
        .iter()
        .map(|d| {
            let mut parts: Vec<String> = Vec::new();

            if d.born_after_baseline {
                parts.push("newly formed".to_owned());
            }
            if d.went_dormant {
                parts.push("went dormant".to_owned());
            }
            if d.revived {
                parts.push("revived from dormancy".to_owned());
            }
            if !d.members_added.is_empty() {
                let added: Vec<String> = d.members_added.iter().map(|&id| names.name(id)).collect();
                parts.push(format!("added members: {}", added.join(", ")));
            }
            if !d.members_removed.is_empty() {
                let removed: Vec<String> =
                    d.members_removed.iter().map(|&id| names.name(id)).collect();
                parts.push(format!("removed members: {}", removed.join(", ")));
            }
            if d.coherence_delta.abs() > 0.05 {
                parts.push(format!(
                    "coherence {:.2} → {:.2} ({:+.2})",
                    d.coherence_at_baseline, d.coherence_now, d.coherence_delta
                ));
            }
            if d.member_count_delta != 0 {
                parts.push(format!("member count {:+}", d.member_count_delta));
            }

            let desc = if parts.is_empty() {
                "minor changes".to_owned()
            } else {
                parts.join("; ")
            };

            format!("Entity#{}: {}", d.entity_id.0, desc)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// ─── Prediction scoring ───────────────────────────────────────────────────────

/// Ask the LLM to evaluate a prediction made by the simulation engine.
///
/// `task_description` explains what was predicted (e.g. "faction split on
/// Karate Club"), `ground_truth` is the known-correct answer, `prediction` is
/// what the engine produced, and `metrics` summarises quantitative accuracy
/// (precision, recall, accuracy percentage, etc.).
///
/// Returns a concise qualitative evaluation with an overall score.
pub fn score_prediction(
    client: &dyn LlmClient,
    task_description: &str,
    ground_truth: &str,
    prediction: &str,
    metrics: &str,
) -> Result<String, LlmError> {
    let system = "\
        You are an expert evaluator of graph learning and network analysis systems. \
        Evaluate the engine prediction against the ground truth using this rubric:\n\
        - correctness (0-4): does the prediction capture the qualitatively correct structure?\n\
        - precision (0-2): are false positives low?\n\
        - recall (0-2): are false negatives low?\n\
        - insight (0-2): does the prediction surface non-obvious structure?\n\
        Produce a JSON object with keys: correctness, precision, recall, insight, total (sum), \
        rationale (one sentence per criterion). \
        Then, after the JSON block, add one paragraph of plain-language interpretation \
        (what the engine got right, what it missed, and why). \
        Total score range: 0–10.";

    let user = format!(
        "Task: {task_description}\n\n\
         Ground truth:\n{ground_truth}\n\n\
         Engine prediction:\n{prediction}\n\n\
         Quantitative metrics:\n{metrics}\n\n\
         Evaluate the prediction."
    );

    client.complete(system, &user)
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::MockLlmClient;

    #[test]
    fn empty_removed_rels_returns_canned_message() {
        let client = MockLlmClient::new("should not be called");
        let result = narrate_counterfactual(&client, &[]).unwrap();
        assert!(result.contains("No relationships"), "{result}");
    }

    #[test]
    fn counterfactual_passes_names_to_llm() {
        let client = MockLlmClient::new("Alice and Bob were co-activated.");
        let pairs = vec![
            ("Alice".to_owned(), "Bob".to_owned()),
            ("Carol".to_owned(), "Dave".to_owned()),
        ];
        let result = narrate_counterfactual(&client, &pairs).unwrap();
        assert_eq!(result, "Alice and Bob were co-activated.");
    }

    #[test]
    fn empty_diffs_returns_canned_message() {
        let client = MockLlmClient::new("should not be called");
        let names = NameMap::default();
        let result = narrate_entity_deviations(&client, &[], &names).unwrap();
        assert!(result.contains("No entity"), "{result}");
    }
}
