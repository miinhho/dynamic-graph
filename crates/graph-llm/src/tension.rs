//! Schema-tension narration: translate `BoundaryAction` proposals into prose.
//!
//! After [`graph_boundary::prescribe_updates`] produces a list of
//! [`BoundaryAction`]s, call [`narrate_prescriptions`] to get a plain-language
//! explanation of what the analysis found and what the user should do about it.
//!
//! ## Example
//!
//! ```ignore
//! use graph_boundary::{analyze_boundary, prescribe_updates, PrescriptionConfig};
//! use graph_llm::{AnthropicClient, narrate_prescriptions};
//! use graph_query::NameMap;
//!
//! let report  = analyze_boundary(&dynamic, &schema, None);
//! let actions = prescribe_updates(&report, &schema, &dynamic, &PrescriptionConfig::default());
//! let names   = NameMap::from_world(&dynamic);
//!
//! let client  = AnthropicClient::from_env().unwrap();
//! let prose   = narrate_prescriptions(&client, &actions, &schema, &names).unwrap();
//! println!("{prose}");
//! ```

use graph_boundary::{BoundaryAction, RetractReason};
use graph_query::NameMap;
use graph_schema::SchemaWorld;

use crate::client::LlmClient;
use crate::error::LlmError;

// ─── Public API ───────────────────────────────────────────────────────────────

/// Ask the LLM to narrate a list of schema-update proposals.
///
/// `actions` is the output of [`graph_boundary::prescribe_updates`].
/// `schema` is needed to resolve `RetractFact` fact IDs to subject/object
/// locus IDs (so they can be named via `names`).
///
/// Returns a concise, actionable explanation suitable for displaying to a
/// domain expert. Returns a canned message without calling the LLM when
/// `actions` is empty.
pub fn narrate_prescriptions(
    client: &dyn LlmClient,
    actions: &[BoundaryAction],
    schema: &SchemaWorld,
    names: &NameMap,
) -> Result<String, LlmError> {
    if actions.is_empty() {
        return Ok(
            "The schema is well-aligned with observed behaviour — no updates are recommended."
                .to_owned(),
        );
    }

    let summary = format_actions(actions, schema, names);

    let system = "\
        You are a schema advisor for a dynamic knowledge graph. \
        The system has detected a gap between the declared schema and observed behaviour. \
        Translate the proposed schema updates into plain-language recommendations. \
        Explain what each proposal means in domain terms and why the system is suggesting it. \
        Be concise (one short paragraph) and actionable.";

    let user = format!(
        "Boundary analysis produced the following schema update proposals:\n\n{summary}\n\n\
         Explain what these recommendations mean and what the user should do."
    );

    client.complete(system, &user)
}

// ─── Formatting helpers ───────────────────────────────────────────────────────

fn format_actions(actions: &[BoundaryAction], schema: &SchemaWorld, names: &NameMap) -> String {
    actions
        .iter()
        .map(|action| match action {
            BoundaryAction::RetractFact { fact_id, reason } => {
                let fact_desc = schema
                    .facts
                    .active_facts()
                    .find(|f| f.id == *fact_id)
                    .map(|f| {
                        format!(
                            "{} -[{}]→ {}",
                            names.name(f.subject),
                            f.predicate,
                            names.name(f.object)
                        )
                    })
                    .unwrap_or_else(|| format!("fact#{}", fact_id.0));

                let age_note = match reason {
                    RetractReason::LongRunningGhost { age_versions } => {
                        format!(" (declared but behaviourally absent for {age_versions} schema versions)")
                    }
                };

                format!("RETRACT: {fact_desc}{age_note}")
            }

            BoundaryAction::AssertFact {
                subject,
                predicate,
                object,
                ..
            } => {
                format!(
                    "ASSERT: {} -[{}]→ {}",
                    names.name(*subject),
                    predicate,
                    names.name(*object)
                )
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::MockLlmClient;
    use graph_schema::SchemaWorld;

    #[test]
    fn empty_actions_returns_canned_message() {
        let client = MockLlmClient::new("should not be called");
        let schema = SchemaWorld::new();
        let names = NameMap::default();
        let result = narrate_prescriptions(&client, &[], &schema, &names).unwrap();
        assert!(result.contains("no updates are recommended"), "{result}");
    }

    #[test]
    fn assert_fact_action_formats_correctly() {
        use graph_boundary::BoundaryAction;
        use graph_core::LocusId;
        use graph_core::RelationshipId;
        use graph_schema::DeclaredRelKind;

        let client = MockLlmClient::new("schema advice here");
        let schema = SchemaWorld::new();
        let names = NameMap::from_pairs([(LocusId(1), "Alice"), (LocusId(2), "Bob")]);

        let actions = vec![BoundaryAction::AssertFact {
            subject: LocusId(1),
            predicate: DeclaredRelKind::new("collaborates_with"),
            object: LocusId(2),
            shadow_rel: RelationshipId(0),
        }];

        let result = narrate_prescriptions(&client, &actions, &schema, &names).unwrap();
        assert_eq!(result, "schema advice here");
    }
}
