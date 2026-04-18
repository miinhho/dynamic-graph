//! Oracle test: Davis Southern Women (1941, Davis/Gardner/Gardner)
//!
//! The canonical bipartite dataset from *Deep South: A Social Anthropological
//! Study of Caste and Class* (Davis, Gardner, Gardner, 1941): 18 women in a
//! Southern US town attended 14 informal social events over a 9-month period.
//! The researchers ethnographically identified two overlapping cliques
//! (roughly "Northern" and "Southern") that the attendance matrix reflects.
//!
//! The attendance data below is the canonical 18 × 14 matrix as reproduced
//! in NetworkX (`nx.davis_southern_women_graph`), Freeman (2003) "Finding
//! Social Groups", and dozens of community-detection benchmarks. Total
//! attendance records: 89.
//!
//! ## Why this dataset (vs karate_club)
//!
//! Karate club pre-populates all 78 edges with structural weights and runs
//! `recognize_entities` once. It exercises the *oracle* but not the engine's
//! event-stream → relationship-emergence path.
//!
//! Davis is naturally a **sequence of 14 events**. The dynamic test here
//! feeds each event as a batch of stimuli, lets the engine auto-emerge
//! Woman↔Woman relationships from co-attendance, and asks the oracle to find
//! the two cliques. No relationships are pre-inserted in the dynamic test.
//!
//! ## Two tests (mirrors karate_club.rs)
//!
//! 1. `static_clique_detection` — pre-insert co-attendance-weighted edges,
//!    run `recognize_entities` once. Correctness sanity check of the
//!    oracle on a pre-built graph.
//! 2. `dynamic_clique_emergence` — feed events one batch at a time. Each
//!    attending woman's program broadcasts co-attendance signals to the
//!    other attendees; auto-emergence builds the relationships; finally
//!    `recognize_entities` is called to extract the cliques.
//!
//! ## Ground truth partition
//!
//! Freeman's 2003 meta-analysis of 21 community-detection methods finds
//! strong consensus on two core cliques:
//!
//! - **Northern core**: women 0–5 (Evelyn, Laura, Theresa, Brenda,
//!   Charlotte, Frances) — dense attendance of events E1–E6
//! - **Southern core**: women 11–14 (Katherine, Sylvia, Nora, Helen) —
//!   dense attendance of events E8–E14
//!
//! Women 6–10 and 15–17 are boundary/bridge figures whose assignment
//! varies across methods.  The oracle checks that **core members separate**
//! — cross-pollination of cores into the same entity is a failure; where
//! boundary women land is not asserted.

use graph_core::{
    BatchId, ChangeSubject, Endpoints, InfluenceKindId, KindObservation, Locus, LocusContext,
    LocusId, LocusKindId, LocusProgram, Properties, PropertyValue, ProposedChange, Relationship,
    RelationshipLineage, StateVector,
};
use graph_engine::{
    DefaultEmergencePerspective, Engine, EngineConfig, InfluenceKindConfig, InfluenceKindRegistry,
    LocusKindRegistry,
};
use graph_world::World;

// ── Canonical Davis/Gardner/Gardner attendance data ───────────────────────────
//
// 18 women × 14 events. Each row is the list of women who attended that event.
// Matches NetworkX's `davis_southern_women_graph` exactly.
//
// Row sums should be [3, 3, 6, 4, 8, 8, 10, 14, 12, 5, 4, 6, 3, 3] = 89 ✓

const EVENTS: &[&[u64]] = &[
    &[0, 1, 3],                                       // E1  (3)
    &[0, 1, 2],                                       // E2  (3)
    &[0, 1, 2, 3, 4, 5],                              // E3  (6)
    &[0, 2, 3, 4],                                    // E4  (4)
    &[0, 1, 2, 3, 4, 5, 6, 8],                        // E5  (8)
    &[0, 1, 2, 3, 5, 6, 7, 13],                       // E6  (8)
    &[1, 2, 3, 4, 6, 8, 9, 12, 13, 14],               // E7  (10)
    &[0, 1, 2, 3, 5, 6, 7, 8, 9, 10, 11, 12, 14, 15], // E8  (14)
    &[0, 2, 7, 8, 9, 10, 11, 12, 13, 15, 16, 17],     // E9  (12)
    &[10, 11, 12, 13, 14],                            // E10 (5)
    &[13, 14, 16, 17],                                // E11 (4)
    &[9, 10, 11, 12, 13, 14],                         // E12 (6)
    &[11, 12, 13],                                    // E13 (3)
    &[11, 12, 13],                                    // E14 (3)
];

const N_WOMEN: u64 = 18;

/// Core Northern clique (Freeman 2003 consensus).
const NORTH_CORE: &[u64] = &[0, 1, 2, 3, 4, 5];

/// Core Southern clique (Freeman 2003 consensus).
const SOUTH_CORE: &[u64] = &[11, 12, 13, 14];

const CO_ATTEND: InfluenceKindId = InfluenceKindId(100);
const WOMAN_KIND: LocusKindId = LocusKindId(1);

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Number of events at which both `a` and `b` were present.
fn co_attendance_count(a: u64, b: u64) -> u32 {
    EVENTS
        .iter()
        .filter(|ev| ev.contains(&a) && ev.contains(&b))
        .count() as u32
}

fn insert_women(world: &mut World) {
    for i in 0..N_WOMEN {
        world.insert_locus(Locus::new(LocusId(i), WOMAN_KIND, StateVector::zeros(1)));
    }
}

fn make_inf_reg(decay: f32) -> InfluenceKindRegistry {
    let cfg = InfluenceKindConfig::new("co_attendance")
        .with_decay(decay)
        .with_symmetric(true);
    let mut reg = InfluenceKindRegistry::new();
    reg.insert(CO_ATTEND, cfg);
    reg
}

// ── Static oracle: pre-insert co-attendance edges, recognize once ────────────

/// Pre-populates the world with Woman↔Woman relationships whose activity
/// equals the co-attendance count. Tests that the emergence perspective can
/// identify the two cliques on a pre-built weighted graph.
fn populate_world_static(world: &mut World) {
    insert_women(world);
    for a in 0..N_WOMEN {
        for b in (a + 1)..N_WOMEN {
            let count = co_attendance_count(a, b);
            if count == 0 {
                continue;
            }
            let id = world.relationships_mut().mint_id();
            world.relationships_mut().insert(Relationship {
                id,
                kind: CO_ATTEND,
                endpoints: Endpoints::Symmetric {
                    a: LocusId(a),
                    b: LocusId(b),
                },
                state: StateVector::from_slice(&[count as f32, 0.0]),
                lineage: RelationshipLineage {
                    created_by: None,
                    last_touched_by: None,
                    change_count: 0,
                    kinds_observed: smallvec::smallvec![KindObservation::synthetic(CO_ATTEND)],
                },
                created_batch: BatchId(0),
                last_decayed_batch: 0,
                metadata: None,
            });
        }
    }
}

/// At the default perspective threshold of 0.1, Davis's co-attendance
/// graph is nearly complete (99 of 153 possible pairs share ≥1 event) and
/// weighted label propagation collapses into a single community. Tuning
/// the `min_activity_threshold` above the cross-clique co-attendance
/// count separates the cores.
///
/// **Finding**: `DefaultEmergencePerspective::default()` is tuned for
/// sparse structural graphs (like karate_club's 78 edges at density 14%);
/// dense dense weighted graphs need a higher activity threshold.
#[test]
fn static_clique_detection_default_threshold_merges() {
    let mut world = World::new();
    let inf = make_inf_reg(1.0); // no decay — static snapshot
    populate_world_static(&mut world);

    let engine = Engine::default();
    let perspective = DefaultEmergencePerspective::default();
    engine.recognize_entities(&mut world, &inf, &perspective);

    let entities: Vec<_> = world.entities().active().collect();
    assert_eq!(
        entities.len(),
        1,
        "documenting that default threshold produces a single merged community on Davis; \
         if this changes, update the finding in the test's docstring"
    );
    println!(
        "[static/default] 1 entity covers {} women at threshold 0.1",
        entities[0].current.members.len()
    );
}

/// With `min_activity_threshold = 3.0`, cross-clique pairs (co-attendance
/// count 1 or 2) are excluded and the two cores separate cleanly.
#[test]
fn static_clique_detection_tuned() {
    let mut world = World::new();
    let inf = make_inf_reg(1.0);
    populate_world_static(&mut world);

    let engine = Engine::default();
    let perspective = DefaultEmergencePerspective {
        min_activity_threshold: Some(3.0),
    };
    engine.recognize_entities(&mut world, &inf, &perspective);

    report_partition(&world);
    assert_cores_separate(&world, "static/tuned");
}

// ── Dynamic: events flow through tick loop, relationships auto-emerge ────────

/// Co-attendance propagation program.
///
/// **Event-stimulus path**: when the woman receives a change carrying
/// `metadata.co_attendees: [id, ...]`, she emits one `ProposedChange` to each
/// listed co-attendee. The emitted change has **no metadata**, so receivers
/// don't re-broadcast — the cascade stops at depth 1.
///
/// **Emergence mechanism**: each emitted change has `subject = co_attendee`,
/// but its auto-derived predecessor is the emitter's change (at the emitter's
/// locus). That cross-locus predecessor triggers auto-emergence of a symmetric
/// Woman↔Woman relationship of kind `CO_ATTEND`.
struct CoAttendanceProgram;

impl LocusProgram for CoAttendanceProgram {
    fn process(
        &self,
        locus: &Locus,
        incoming: &[&graph_core::Change],
        _ctx: &dyn LocusContext,
    ) -> Vec<ProposedChange> {
        let mut out = Vec::new();
        for change in incoming {
            if change.subject != ChangeSubject::Locus(locus.id) {
                continue;
            }
            let Some(meta) = change.metadata.as_ref() else {
                continue;
            };
            let Some(PropertyValue::List(ids)) = meta.get("co_attendees") else {
                continue;
            };
            for val in ids {
                let PropertyValue::Int(id) = val else {
                    continue;
                };
                let other = *id as u64;
                if other == locus.id.0 {
                    continue;
                }
                out.push(ProposedChange::new(
                    ChangeSubject::Locus(LocusId(other)),
                    CO_ATTEND,
                    StateVector::from_slice(&[1.0]),
                ));
            }
        }
        out
    }
}

/// Build the stimulus batch for one event: one `ProposedChange` per attendee,
/// carrying the other attendees' ids in metadata.
fn event_stimuli(attendees: &[u64]) -> Vec<ProposedChange> {
    attendees
        .iter()
        .map(|&w| {
            let others: Vec<PropertyValue> = attendees
                .iter()
                .filter(|&&x| x != w)
                .map(|&id| PropertyValue::Int(id as i64))
                .collect();
            let mut meta = Properties::new();
            meta.set("co_attendees", PropertyValue::List(others));
            ProposedChange::new(
                ChangeSubject::Locus(LocusId(w)),
                CO_ATTEND,
                StateVector::from_slice(&[1.0]),
            )
            .with_metadata(meta)
        })
        .collect()
}

/// Helper: drive all 14 events through the tick loop, returning the
/// populated world. Used by both the default-threshold and tuned variants.
fn run_dynamic_events() -> (World, InfluenceKindRegistry) {
    let mut world = World::new();
    insert_women(&mut world);

    let inf = make_inf_reg(0.99);
    let mut loci_reg = LocusKindRegistry::new();
    loci_reg.insert(WOMAN_KIND, Box::new(CoAttendanceProgram));

    // batch 0: event stimuli + women's programs fire.
    // batch 1: pair changes commit (auto-emerge Woman↔Woman).
    let engine = Engine::new(EngineConfig {
        max_batches_per_tick: 3,
    });
    for attendees in EVENTS {
        engine.tick(&mut world, &loci_reg, &inf, event_stimuli(attendees));
    }

    (world, inf)
}

// `dynamic_clique_emergence_default_threshold_merges` removed with Phase 2:
// `DefaultEmergencePerspective::default()` now computes the activity
// threshold from the distribution (p25), so Davis's co-attendance graph
// no longer collapses at the default. Finding 1 is resolved.

/// Dynamic flow at tuned threshold: the full pipeline (event stimulus →
/// auto-emerged Woman↔Woman relationship → community recognition) recovers
/// the Freeman-consensus Northern/Southern split.
///
/// Threshold of 5.0 empirically separates the cores: intra-clique pairs
/// (co-attendance ≥ 6 events × 2 symmetric touches × ~0.93 avg decay
/// ≈ 11) clear the threshold while cross-clique pairs (co-attendance
/// 1–2 events ≈ 1.8–3.6) fall below.
#[test]
fn dynamic_clique_emergence_tuned() {
    let (mut world, inf) = run_dynamic_events();

    let engine = Engine::default();
    let perspective = DefaultEmergencePerspective {
        min_activity_threshold: Some(5.0),
    };
    engine.recognize_entities(&mut world, &inf, &perspective);

    report_partition(&world);
    assert_cores_separate(&world, "dynamic/tuned");
}

// ── Oracle: Freeman-consensus core separation ────────────────────────────────

use std::collections::BTreeSet;

/// The minimum bar: the Northern and Southern cores must not collapse into
/// a single entity. Boundary women (6–10, 15–17) can land anywhere — the
/// literature disagrees on their assignment.
fn assert_cores_separate(world: &World, label: &str) {
    let entities: Vec<_> = world.entities().active().collect();
    assert!(
        !entities.is_empty(),
        "[{label}] no entities recognised — are any relationships above activity threshold?"
    );

    for w_n in NORTH_CORE {
        for w_s in SOUTH_CORE {
            let n_entity = entities
                .iter()
                .position(|e| e.current.members.contains(&LocusId(*w_n)));
            let s_entity = entities
                .iter()
                .position(|e| e.current.members.contains(&LocusId(*w_s)));
            match (n_entity, s_entity) {
                (Some(i), Some(j)) if i == j => {
                    panic!(
                        "[{label}] Northern core {w_n} and Southern core {w_s} are in the SAME entity (index {i}). Cores must separate."
                    );
                }
                _ => {}
            }
        }
    }

    // Also check: every core member is actually placed somewhere.
    let covered: BTreeSet<u64> = entities
        .iter()
        .flat_map(|e| e.current.members.iter().map(|l| l.0))
        .collect();
    for w in NORTH_CORE.iter().chain(SOUTH_CORE.iter()) {
        assert!(
            covered.contains(w),
            "[{label}] core member {w} is not in any entity"
        );
    }

    println!(
        "[{label}] ✓ cores separated: {} entities, {} covered women",
        entities.len(),
        covered.len()
    );
}

/// Print the partition for human inspection. Called by the dynamic test only.
fn report_partition(world: &World) {
    let entities: Vec<_> = world.entities().active().collect();
    println!(
        "\n── Davis dynamic partition ({} entities) ──",
        entities.len()
    );
    let north: BTreeSet<u64> = NORTH_CORE.iter().copied().collect();
    let south: BTreeSet<u64> = SOUTH_CORE.iter().copied().collect();
    for (i, e) in entities.iter().enumerate() {
        let members: BTreeSet<u64> = e.current.members.iter().map(|l| l.0).collect();
        let n_hits = members.intersection(&north).count();
        let s_hits = members.intersection(&south).count();
        let tag = match (n_hits, s_hits) {
            (n, 0) if n > 0 => "NORTH",
            (0, s) if s > 0 => "SOUTH",
            (n, s) if n > 0 && s > 0 => "MIXED",
            _ => "boundary",
        };
        println!(
            "  E{i} [{tag}] coh={:.2} n={} members={:?}",
            e.current.coherence,
            members.len(),
            members,
        );
    }
    println!(
        "  relationships: {} edges at activity ≥ 0.1",
        world
            .relationships()
            .iter()
            .filter(|r| r.activity() >= 0.1)
            .count()
    );
    println!();
}

// ── Sanity ───────────────────────────────────────────────────────────────────

#[test]
fn event_data_has_89_attendance_records() {
    // Guards against data-entry drift in EVENTS.
    let total: usize = EVENTS.iter().map(|e| e.len()).sum();
    assert_eq!(total, 89, "canonical Davis attendance total is 89");
}
