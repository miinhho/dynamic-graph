//! J1 — mock-backend regression harness.
//!
//! Pins the *structure* of every prompt `graph-llm` sends, without
//! depending on any model's output. Each test drives one public entry
//! point with a `CapturingLlmClient`, then asserts that the captured
//! `(system, user)` strings contain the fields and section markers
//! that downstream consumers (logging, caching, observability) will
//! rely on.
//!
//! Prose phrasing is free to evolve — assertions target only the
//! stable structural landmarks (quadrant headers, key JSON field
//! names, section separators).

use graph_core::{BatchId, Endpoints, LocusId};
use graph_llm::{
    CapturingLlmClient, TextIngestor, answer_with_graph, configure_cohere, configure_emergence,
    configure_influence, narrate_boundary, narrate_counterfactual, narrate_entity_deviations,
    narrate_prescriptions,
};
use graph_query::NameMap;
use graph_schema::{DeclaredRelKind, SchemaWorld};
use graph_world::World;

fn assert_contains(haystack: &str, needle: &str, label: &str) {
    assert!(
        haystack.contains(needle),
        "{label} prompt missing expected substring {:?}\n--- captured ---\n{haystack}",
        needle,
    );
}

// ── narrate_counterfactual ────────────────────────────────────────────────────

#[test]
fn narrate_counterfactual_prompt_lists_removed_relationships() {
    let client = CapturingLlmClient::new("ok");
    let pairs = vec![
        ("Alice".to_owned(), "Bob".to_owned()),
        ("Bob".to_owned(), "Carol".to_owned()),
    ];
    let _ = narrate_counterfactual(&client, &pairs).unwrap();
    let (system, user) = client.last().expect("client invoked");
    assert_contains(&system, "dynamic knowledge graph", "counterfactual/system");
    assert_contains(&user, "Alice ↔ Bob", "counterfactual/user");
    assert_contains(&user, "Bob ↔ Carol", "counterfactual/user");
    assert_contains(&user, "would not exist", "counterfactual/user");
}

#[test]
fn narrate_counterfactual_empty_skips_client() {
    let client = CapturingLlmClient::new("ok");
    let result = narrate_counterfactual(&client, &[]).unwrap();
    assert!(client.calls().is_empty(), "empty input must not call LLM");
    assert!(result.contains("No relationships"));
}

// ── narrate_entity_deviations ─────────────────────────────────────────────────

#[test]
fn narrate_entity_deviations_empty_skips_client() {
    let client = CapturingLlmClient::new("ok");
    let names = NameMap::default();
    let result = narrate_entity_deviations(&client, &[], &names).unwrap();
    assert!(client.calls().is_empty());
    assert!(result.contains("No entity changes"));
}

// ── narrate_boundary / narrate_prescriptions ──────────────────────────────────

#[test]
fn narrate_boundary_prompt_carries_four_quadrant_structure() {
    use graph_boundary::{BoundaryEdge, BoundaryReport};
    let client = CapturingLlmClient::new("ok");
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
    let _ = narrate_boundary(&client, &report, &world, &names).unwrap();
    let (system, user) = client.last().expect("client invoked");
    assert_contains(&system, "boundary report", "boundary/system");
    assert_contains(&system, "declared schema", "boundary/system");
    assert_contains(&user, "tension = 0.500", "boundary/user");
    assert_contains(&user, "CONFIRMED", "boundary/user");
    assert_contains(&user, "Alice -[knows]→ Bob", "boundary/user");
}

#[test]
fn narrate_boundary_aligned_skips_client() {
    use graph_boundary::BoundaryReport;
    let client = CapturingLlmClient::new("ok");
    let world = World::default();
    let names = NameMap::default();
    let aligned = BoundaryReport {
        confirmed: vec![],
        ghost: vec![],
        shadow: vec![],
        tension: 0.0,
    };
    let _ = narrate_boundary(&client, &aligned, &world, &names).unwrap();
    assert!(client.calls().is_empty(), "aligned report must not call LLM");
}

#[test]
fn narrate_prescriptions_prompt_lists_retract_and_assert_lines() {
    use graph_boundary::BoundaryAction;
    use graph_core::RelationshipId;
    let client = CapturingLlmClient::new("ok");
    let schema = SchemaWorld::new();
    let names = NameMap::from_pairs([(LocusId(1), "Alice"), (LocusId(2), "Bob")]);
    let actions = vec![BoundaryAction::AssertFact {
        subject: LocusId(1),
        predicate: DeclaredRelKind::new("collab"),
        object: LocusId(2),
        shadow_rel: RelationshipId(0),
        severity: 0.5,
    }];
    let _ = narrate_prescriptions(&client, &actions, &schema, &names).unwrap();
    let (system, user) = client.last().expect("client invoked");
    assert_contains(&system, "schema advisor", "prescribe/system");
    assert_contains(&user, "ASSERT:", "prescribe/user");
    assert_contains(&user, "Alice -[collab]→ Bob", "prescribe/user");
}

#[test]
fn narrate_prescriptions_empty_skips_client() {
    let client = CapturingLlmClient::new("ok");
    let schema = SchemaWorld::new();
    let names = NameMap::default();
    let result = narrate_prescriptions(&client, &[], &schema, &names).unwrap();
    assert!(client.calls().is_empty());
    assert!(result.contains("no updates are recommended"));
}

// ── configure_* ───────────────────────────────────────────────────────────────

#[test]
fn configure_influence_prompt_forwards_description_as_user() {
    // Accept any JSON response — we only care about the prompt shape.
    let client = CapturingLlmClient::new(
        r#"{"retention_per_batch": 0.9, "activity_contribution": 1.0, "learning_rate": 0.0, "weight_decay": 1.0, "max_weight": 5.0}"#,
    );
    let desc = "gentle decay with moderate Hebbian strengthening";
    let _ = configure_influence(&client, "signal", desc).unwrap();
    let (system, user) = client.last().expect("client invoked");
    assert_contains(
        &system,
        "parameter configurator",
        "configure_influence/system",
    );
    assert_contains(&system, "retention_per_batch", "configure_influence/system");
    assert_eq!(user, desc, "user prompt must be the description verbatim");
}

#[test]
fn configure_emergence_prompt_mentions_threshold_field() {
    let client = CapturingLlmClient::new(r#"{"min_activity_threshold": 0.1}"#);
    let desc = "only count strong pairs";
    let _ = configure_emergence(&client, desc).unwrap();
    let (system, user) = client.last().expect("client invoked");
    assert_contains(
        &system,
        "min_activity_threshold",
        "configure_emergence/system",
    );
    assert_eq!(user, desc);
}

#[test]
fn configure_cohere_prompt_mentions_bridge_field() {
    let client = CapturingLlmClient::new(r#"{"min_bridge_activity": 0.1}"#);
    let desc = "bridges are meaningful only when sustained";
    let _ = configure_cohere(&client, desc).unwrap();
    let (system, user) = client.last().expect("client invoked");
    assert_contains(&system, "min_bridge_activity", "configure_cohere/system");
    assert_eq!(user, desc);
}

// ── ingest ────────────────────────────────────────────────────────────────────

#[test]
fn text_ingestor_prompt_lists_accepted_kinds() {
    let client = CapturingLlmClient::new(r#"{"nodes": []}"#);
    let ingestor = TextIngestor::new(&client);
    let _ = ingestor.extract("Alice met Bob.", &["PERSON", "ORG"]).unwrap();
    let (system, user) = client.last().expect("client invoked");
    assert_contains(&system, "PERSON", "ingest/system");
    assert_contains(&system, "ORG", "ingest/system");
    assert_contains(&system, "\"nodes\":", "ingest/system");
    assert_eq!(user, "Alice met Bob.", "ingest must send raw text as user");
}

// ── answer_with_graph ─────────────────────────────────────────────────────────

#[test]
fn answer_with_graph_without_matches_forwards_question_only() {
    let client = CapturingLlmClient::new("answer");
    let world = World::default();
    let names = NameMap::default();
    let _ = answer_with_graph(&client, "Who is Alice?", &world, &names, 5).unwrap();
    let (_system, user) = client.last().expect("client invoked");
    assert_contains(&user, "Who is Alice?", "answer_with_graph/user");
}

#[test]
fn answer_with_graph_with_context_includes_graph_section() {
    use graph_core::{InfluenceKindId, Locus, LocusKindId, Relationship, RelationshipId, StateVector};
    use smallvec::SmallVec;

    let mut world = World::default();
    world.loci_mut().insert(Locus::new(
        LocusId(1),
        LocusKindId(0),
        StateVector::zeros(1),
    ));
    world.loci_mut().insert(Locus::new(
        LocusId(2),
        LocusKindId(0),
        StateVector::zeros(1),
    ));
    world.relationships_mut().insert(Relationship {
        id: RelationshipId(0),
        kind: InfluenceKindId(0),
        endpoints: Endpoints::symmetric(LocusId(1), LocusId(2)),
        state: StateVector::from_slice(&[0.9, 0.0]),
        lineage: graph_core::RelationshipLineage {
            created_by: None,
            last_touched_by: None,
            change_count: 0,
            kinds_observed: SmallVec::new(),
        },
        created_batch: BatchId(0),
        last_decayed_batch: 0,
        metadata: None,
    });
    let names = NameMap::from_pairs([(LocusId(1), "Alice"), (LocusId(2), "Bob")]);

    let client = CapturingLlmClient::new("answer");
    let _ = answer_with_graph(&client, "Who does Alice know?", &world, &names, 5).unwrap();
    let (_system, user) = client.last().expect("client invoked");
    assert_contains(&user, "Alice", "answer_with_graph/user(named match)");
    assert_contains(&user, "Bob", "answer_with_graph/user(neighbour)");
    assert_contains(&user, "Who does Alice know?", "answer_with_graph/user(question)");
}
