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

use graph_boundary::{BoundaryAction, BoundaryReport, RetractReason};
use graph_core::LocusId;
use graph_query::NameMap;
use graph_schema::SchemaWorld;
use graph_world::World;

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
/// Ask the LLM to narrate a raw [`BoundaryReport`] — the four-quadrant
/// declared-vs-observed state — without first deriving prescriptions.
///
/// Use this when the caller just wants to *understand* the current
/// drift, not yet decide what to do about it.
/// [`narrate_prescriptions`] is the action-oriented counterpart: it
/// receives proposed `BoundaryAction`s and explains what the user
/// should do. `narrate_boundary` returns the diagnostic story.
///
/// `world` supplies endpoint loci for shadow relationships; `names`
/// resolves `LocusId`s to human-readable labels. Returns a canned
/// aligned-world message (without calling the LLM) when the report's
/// tension is `0.0`.
pub fn narrate_boundary(
    client: &dyn LlmClient,
    report: &BoundaryReport,
    world: &World,
    names: &NameMap,
) -> Result<String, LlmError> {
    if report.is_aligned() {
        return Ok(
            "The declared schema and observed behaviour are fully aligned — no drift to narrate."
                .to_owned(),
        );
    }

    let summary = format_boundary_report(report, world, names);

    let system = "\
        You are a knowledge-graph observer. Given a boundary report that \
        compares a declared schema against observed behaviour, describe \
        the state of the world in plain language. Mention which declared \
        structure is behaviourally confirmed, which is declared-but-silent \
        (ghosts), and which active behaviour lacks any declaration \
        (shadows). Be concise (one short paragraph) and concrete — use \
        the names of the entities involved.";

    let user = format!(
        "Boundary report:\n\n{summary}\n\nDescribe the state and highlight \
         the most salient drift between declared and observed structure."
    );

    client.complete(system, &user)
}

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

/// Summarise a boundary report in a prompt-ready block. Lists up to
/// four edges per quadrant so the LLM has concrete examples without the
/// prompt ballooning on huge reports.
fn format_boundary_report(report: &BoundaryReport, world: &World, names: &NameMap) -> String {
    const SAMPLE: usize = 4;
    let mut out = String::new();

    out.push_str(&format!(
        "tension = {:.3} (0.0 = aligned, 1.0 = disjoint)\n",
        report.tension
    ));
    out.push_str(&format!(
        "counts: confirmed={}  ghost={}  shadow={}\n\n",
        report.confirmed.len(),
        report.ghost.len(),
        report.shadow.len()
    ));

    if !report.confirmed.is_empty() {
        out.push_str("CONFIRMED (declared + behaviourally active):\n");
        for edge in report.confirmed.iter().take(SAMPLE) {
            out.push_str(&format!(
                "  {} -[{}]→ {}\n",
                names.name(edge.subject),
                edge.predicate,
                names.name(edge.object),
            ));
        }
        if report.confirmed.len() > SAMPLE {
            out.push_str(&format!("  ... {} more\n", report.confirmed.len() - SAMPLE));
        }
        out.push('\n');
    }
    if !report.ghost.is_empty() {
        out.push_str("GHOST (declared but silent):\n");
        for edge in report.ghost.iter().take(SAMPLE) {
            out.push_str(&format!(
                "  {} -[{}]→ {}\n",
                names.name(edge.subject),
                edge.predicate,
                names.name(edge.object),
            ));
        }
        if report.ghost.len() > SAMPLE {
            out.push_str(&format!("  ... {} more\n", report.ghost.len() - SAMPLE));
        }
        out.push('\n');
    }
    if !report.shadow.is_empty() {
        out.push_str("SHADOW (active but undeclared):\n");
        for rel_id in report.shadow.iter().take(SAMPLE) {
            if let Some(rel) = world.relationships().get(*rel_id) {
                let (a, b): (LocusId, LocusId) = match rel.endpoints {
                    graph_core::Endpoints::Directed { from, to } => (from, to),
                    graph_core::Endpoints::Symmetric { a, b } => (a, b),
                };
                out.push_str(&format!(
                    "  {} ↔ {}  (strength={:.2})\n",
                    names.name(a),
                    names.name(b),
                    rel.strength(),
                ));
            }
        }
        if report.shadow.len() > SAMPLE {
            out.push_str(&format!("  ... {} more\n", report.shadow.len() - SAMPLE));
        }
    }
    out
}

fn format_actions(actions: &[BoundaryAction], schema: &SchemaWorld, names: &NameMap) -> String {
    actions
        .iter()
        .map(|action| match action {
            BoundaryAction::RetractFact { fact_id, reason, .. } => {
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
    fn aligned_boundary_returns_canned_message() {
        use graph_boundary::BoundaryReport;
        let client = MockLlmClient::new("should not be called");
        let world = World::default();
        let names = NameMap::default();
        let report = BoundaryReport {
            confirmed: vec![],
            ghost: vec![],
            shadow: vec![],
            tension: 0.0,
        };
        let result = narrate_boundary(&client, &report, &world, &names).unwrap();
        assert!(result.contains("fully aligned"), "{result}");
    }

    #[test]
    fn boundary_with_drift_invokes_client() {
        use graph_boundary::{BoundaryEdge, BoundaryReport};
        use graph_core::LocusId;
        use graph_schema::DeclaredRelKind;
        let client = MockLlmClient::new("observed narration");
        let world = World::default();
        let names = NameMap::from_pairs([(LocusId(1), "Alice"), (LocusId(2), "Bob")]);
        let report = BoundaryReport {
            confirmed: vec![BoundaryEdge {
                subject: LocusId(1),
                predicate: DeclaredRelKind::new("knows"),
                object: LocusId(2),
                dynamic_rel: None,
            }],
            ghost: vec![],
            shadow: vec![],
            tension: 0.5,
        };
        let result = narrate_boundary(&client, &report, &world, &names).unwrap();
        assert_eq!(result, "observed narration");
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
            severity: 0.5,
        }];

        let result = narrate_prescriptions(&client, &actions, &schema, &names).unwrap();
        assert_eq!(result, "schema advice here");
    }
}
