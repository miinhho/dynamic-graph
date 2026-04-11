//! Real-world conflict/event model integration test.
//!
//! Demonstrates all new capabilities working together:
//!
//! 1. **Semantic relationship slots** — relationships carry domain attributes
//!    (`hostility`, `engagement_count`) beyond the built-in activity/weight.
//!
//! 2. **`ctx.relationship_between_kind()`** — programs locate an edge by
//!    its two endpoints and kind, removing hard-coded ID coupling.
//!
//! 3. **`ctx.relationship_slot(rel_id, kind, name)`** — named slot access;
//!    programs read "hostility" by name rather than by magic index.
//!
//! 4. **`StateVector::with_slot()`** — clean partial slot updates.
//!
//! 5. **Subscriber propagation** — an analyst meta-locus subscribes to a
//!    specific relationship; when its state changes the analyst receives the
//!    committed `Change` in its inbox and updates regional tension.
//!
//! 6. **`LocusProgram::initial_subscriptions`** — analyst declares its
//!    subscription at construction time; `bootstrap_subscriptions` wires
//!    it before the first tick instead of a manual `subscribe()` call.
//!
//! 7. **`ProposedChange::with_wall_time` / `with_metadata`** — attach
//!    provenance to changes for downstream query/audit pipelines.
//!
//! 8. **EventLocusProgram** — a locus representing an N-ary event that
//!    creates participation edges to all involved parties once its
//!    activation threshold is crossed.
//!
//! ## World layout
//!
//! ```text
//! [Force_A]  ──── (conflict, hostility=0.3) ────  [Force_B]
//!     │                                               │
//!     └──────────────▶ [Engagement Event] ◀──────────┘
//!                            (EventLocusProgram)
//!
//! [Analyst]  ← subscribes to Force_A↔Force_B relationship
//! ```
//!
//! ## Tick flow
//!
//! ```text
//! Batch 0: stimulus → Force_A
//!          Force_A fires → proposes Relationship(A↔B) hostility update
//!                          + forwards signal to Engagement Event
//!
//! Batch 1: Relationship(A↔B) update committed
//!              → Analyst notified (subscriber), updates regional_tension
//!          Engagement Event receives Force_A signal
//!              → activation threshold crossed → creates Event→A, Event→B edges
//!              → subscribes to Force_A↔Force_B relationship for future updates
//!
//! Batch 2: Analyst state committed (quiescent)
//! ```

use graph_core::{
    Change, ChangeSubject, Endpoints, InfluenceKindId, Locus, LocusContext, LocusId, LocusKindId,
    LocusProgram, Properties, ProposedChange, RelationshipId, StateVector, StructuralProposal,
};
use graph_engine::{
    Engine, EngineConfig, InfluenceKindConfig, InfluenceKindRegistry, LocusKindRegistry,
    RelationshipSlotDef,
};
use graph_testkit::programs::{EventLocusProgram, InertProgram};
use graph_world::World;

// ─── Constants ───────────────────────────────────────────────────────────────

const CONFLICT_KIND: InfluenceKindId = InfluenceKindId(1);
const ANALYSIS_KIND: InfluenceKindId = InfluenceKindId(2);

const FORCE_A_KIND: LocusKindId = LocusKindId(10);
const FORCE_B_KIND: LocusKindId = LocusKindId(11);
const EVENT_KIND_ID: LocusKindId = LocusKindId(12);
const ANALYST_KIND: LocusKindId = LocusKindId(13);

const FORCE_A: LocusId = LocusId(0);
const FORCE_B: LocusId = LocusId(1);
const EVENT_LOCUS: LocusId = LocusId(2);
const ANALYST: LocusId = LocusId(3);

// Extra slot indices in the conflict relationship StateVector.
// Layout: [activity(0), weight(1), hostility(2), engagement_count(3)]
const HOSTILITY_SLOT: usize = 2;
const ENGAGEMENT_SLOT: usize = 3;

// ─── Programs ────────────────────────────────────────────────────────────────

/// Models a conflict actor (Force_A or Force_B).
///
/// On stimulation:
/// - Locates the A↔B relationship by kind using `ctx.relationship_between_kind()`
///   — no hard-coded `RelationshipId` needed.
/// - Reads hostility by name using `ctx.relationship_slot(id, kind, "hostility")`.
/// - Uses `StateVector::with_slot()` for clean partial slot updates.
/// - Forwards the signal to the engagement event locus.
struct ConflictActorProgram {
    /// The other actor's locus ID — used to locate the edge semantically.
    peer: LocusId,
    event_locus: LocusId,
}

impl LocusProgram for ConflictActorProgram {
    fn process(
        &self,
        locus: &Locus,
        incoming: &[&Change],
        ctx: &dyn LocusContext,
    ) -> Vec<ProposedChange> {
        let signal: f32 = incoming
            .iter()
            .filter(|c| matches!(c.subject, ChangeSubject::Locus(_)))
            .flat_map(|c| c.after.as_slice().first().copied())
            .sum();

        if signal < 0.01 {
            return Vec::new();
        }

        let mut proposals = Vec::new();

        if let Some(rel) = ctx.relationship_between_kind(locus.id, self.peer, CONFLICT_KIND) {
            let rel_id = rel.id;
            let base_state = rel.state.clone();

            let hostility = ctx
                .relationship_slot(rel_id, CONFLICT_KIND, "hostility")
                .unwrap_or(0.0);
            let engagements = ctx
                .relationship_slot(rel_id, CONFLICT_KIND, "engagement_count")
                .unwrap_or(0.0);

            let new_hostility = (hostility + signal * 0.3).min(1.0);
            let new_state = base_state
                .with_slot(HOSTILITY_SLOT, new_hostility)
                .with_slot(ENGAGEMENT_SLOT, engagements + 1.0);

            proposals.push(
                ProposedChange::new(
                    ChangeSubject::Relationship(rel_id),
                    CONFLICT_KIND,
                    new_state,
                )
                .with_wall_time(1_700_000_000_000)
                .with_metadata({
                    let mut p = Properties::new();
                    p.set("source", "conflict_actor");
                    p
                }),
            );
        }

        // Forward signal to the engagement event locus.
        proposals.push(ProposedChange::new(
            ChangeSubject::Locus(self.event_locus),
            CONFLICT_KIND,
            StateVector::from_slice(&[signal]),
        ));

        proposals
    }
}

/// Regional analyst meta-locus that monitors the A↔B relationship.
///
/// - `initial_subscriptions`: declares its subscription at construction time
///   so `bootstrap_subscriptions` wires it before the first tick — no manual
///   `world.subscriptions_mut().subscribe()` needed in the world builder.
/// - `process`: on each relationship-change notification, reads the updated
///   hostility by name via `ctx.relationship_slot()` and raises tension.
struct AnalystProgram {
    watch_rel: RelationshipId,
}

impl LocusProgram for AnalystProgram {
    fn process(
        &self,
        locus: &Locus,
        incoming: &[&Change],
        ctx: &dyn LocusContext,
    ) -> Vec<ProposedChange> {
        // Only react to relationship-subject changes delivered by subscription.
        let rel_notifications = incoming
            .iter()
            .filter(|c| matches!(c.subject, ChangeSubject::Relationship(_)))
            .count();

        if rel_notifications == 0 {
            return Vec::new();
        }

        let hostility = ctx
            .relationship_slot(self.watch_rel, CONFLICT_KIND, "hostility")
            .unwrap_or(0.0);

        let current_tension = locus.state.as_slice().first().copied().unwrap_or(0.0);
        let new_tension = (current_tension + hostility * 0.5).min(1.0);

        vec![ProposedChange::new(
            ChangeSubject::Locus(locus.id),
            ANALYSIS_KIND,
            StateVector::from_slice(&[new_tension]),
        )]
    }

    fn structural_proposals(
        &self,
        locus: &Locus,
        incoming: &[&Change],
        _ctx: &dyn LocusContext,
    ) -> Vec<StructuralProposal> {
        // Re-subscribe every batch we're active (idempotent, cheap).
        if incoming.is_empty() {
            return Vec::new();
        }
        vec![StructuralProposal::SubscribeToRelationship {
            subscriber: locus.id,
            rel_id: self.watch_rel,
        }]
    }

    fn initial_subscriptions(&self, _locus: &Locus) -> Vec<RelationshipId> {
        vec![self.watch_rel]
    }
}

// ─── World builder ───────────────────────────────────────────────────────────

fn build_conflict_world() -> (World, LocusKindRegistry, InfluenceKindRegistry, RelationshipId) {
    let mut world = World::new();
    let mut loci_reg = LocusKindRegistry::new();
    let mut inf_reg = InfluenceKindRegistry::new();

    // ── Semantic relationship kind with extra slots ───────────────────────
    // hostility  (slot 2): decays slowly at 0.98/batch
    // engagement (slot 3): no decay — cumulative count
    inf_reg.insert(
        CONFLICT_KIND,
        InfluenceKindConfig::new("conflict")
            .with_decay(0.95)
            .with_extra_slots(vec![
                RelationshipSlotDef::new("hostility", 0.0).with_decay(0.98),
                RelationshipSlotDef::new("engagement_count", 0.0),
            ]),
    );
    inf_reg.insert(
        ANALYSIS_KIND,
        InfluenceKindConfig::new("analysis").with_decay(0.99),
    );

    // ── Pre-create the A↔B conflict relationship ─────────────────────────
    // Initial state: activity=1.0, weight=0.0, hostility=0.3, engagements=0
    // `add_relationship` sets last_decayed_batch = current_batch(), preventing
    // spurious decay debt on first touch.
    let ab_rel_id = world.add_relationship(
        Endpoints::Symmetric { a: FORCE_A, b: FORCE_B },
        CONFLICT_KIND,
        StateVector::from_slice(&[1.0, 0.0, 0.3, 0.0]),
    );

    // ── Loci ─────────────────────────────────────────────────────────────
    world.insert_locus(Locus::new(FORCE_A, FORCE_A_KIND, StateVector::from_slice(&[1.0])));
    world.insert_locus(Locus::new(FORCE_B, FORCE_B_KIND, StateVector::from_slice(&[1.0])));
    world.insert_locus(Locus::new(
        EVENT_LOCUS,
        EVENT_KIND_ID,
        StateVector::zeros(3), // [activation, severity, confidence]
    ));
    world.insert_locus(Locus::new(
        ANALYST,
        ANALYST_KIND,
        StateVector::zeros(1), // [regional_tension]
    ));

    // ── Programs ─────────────────────────────────────────────────────────
    loci_reg.insert(
        FORCE_A_KIND,
        Box::new(ConflictActorProgram {
            peer: FORCE_B,
            event_locus: EVENT_LOCUS,
        }),
    );
    loci_reg.insert(FORCE_B_KIND, Box::new(InertProgram));

    loci_reg.insert(
        EVENT_KIND_ID,
        Box::new(
            EventLocusProgram::new(
                vec![FORCE_A, FORCE_B],
                0.5, // fires when incoming signal >= 0.5
                CONFLICT_KIND,
            )
            .watching(vec![ab_rel_id]),
        ),
    );

    loci_reg.insert(
        ANALYST_KIND,
        Box::new(AnalystProgram { watch_rel: ab_rel_id }),
    );

    let engine = Engine::new(EngineConfig::default());
    engine.bootstrap_subscriptions(&mut world, &loci_reg);

    (world, loci_reg, inf_reg, ab_rel_id)
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[test]
fn conflict_hostility_increases_on_engagement() {
    let (mut world, loci_reg, inf_reg, ab_rel_id) = build_conflict_world();
    let engine = Engine::new(EngineConfig::default());

    let initial_hostility = world
        .relationships()
        .get(ab_rel_id)
        .unwrap()
        .state
        .as_slice()[HOSTILITY_SLOT];

    // Inject a conflict stimulus into Force A.
    engine.tick(
        &mut world,
        &loci_reg,
        &inf_reg,
        vec![ProposedChange::new(
            ChangeSubject::Locus(FORCE_A),
            CONFLICT_KIND,
            StateVector::from_slice(&[0.8]),
        )],
    );

    let rel = world.relationships().get(ab_rel_id).unwrap();
    let new_hostility = rel.state.as_slice()[HOSTILITY_SLOT];
    let engagements = rel.state.as_slice()[ENGAGEMENT_SLOT];

    assert!(
        new_hostility > initial_hostility,
        "hostility should rise after engagement: {initial_hostility} → {new_hostility}"
    );
    assert_eq!(engagements, 1.0, "one engagement recorded");
}

#[test]
fn analyst_reacts_to_relationship_change_via_subscription() {
    let (mut world, loci_reg, inf_reg, _ab_rel_id) = build_conflict_world();
    let engine = Engine::new(EngineConfig::default());

    let initial_tension = world
        .locus(ANALYST)
        .unwrap()
        .state
        .as_slice()
        .first()
        .copied()
        .unwrap_or(0.0);
    assert_eq!(initial_tension, 0.0, "analyst starts at zero tension");

    engine.tick(
        &mut world,
        &loci_reg,
        &inf_reg,
        vec![ProposedChange::new(
            ChangeSubject::Locus(FORCE_A),
            CONFLICT_KIND,
            StateVector::from_slice(&[0.8]),
        )],
    );

    let new_tension = world
        .locus(ANALYST)
        .unwrap()
        .state
        .as_slice()
        .first()
        .copied()
        .unwrap_or(0.0);

    assert!(
        new_tension > initial_tension,
        "analyst regional_tension should rise after conflict escalation: {new_tension}"
    );
}

#[test]
fn event_locus_fires_and_creates_participant_edges() {
    let (mut world, loci_reg, inf_reg, _ab_rel_id) = build_conflict_world();
    let engine = Engine::new(EngineConfig::default());

    let edges_before = world
        .relationships()
        .relationships_for_locus(EVENT_LOCUS)
        .count();

    engine.tick(
        &mut world,
        &loci_reg,
        &inf_reg,
        vec![ProposedChange::new(
            ChangeSubject::Locus(FORCE_A),
            CONFLICT_KIND,
            StateVector::from_slice(&[0.8]),
        )],
    );

    let edges_after = world
        .relationships()
        .relationships_for_locus(EVENT_LOCUS)
        .count();

    assert!(
        edges_after > edges_before,
        "EventLocus should create participation edges to Force_A and Force_B: \
         had {edges_before}, now {edges_after}"
    );

    // Both Force_A and Force_B should be connected to the event.
    let connected_to_a = world
        .relationships()
        .relationships_for_locus(EVENT_LOCUS)
        .any(|r| r.endpoints.involves(FORCE_A));
    let connected_to_b = world
        .relationships()
        .relationships_for_locus(EVENT_LOCUS)
        .any(|r| r.endpoints.involves(FORCE_B));

    assert!(connected_to_a, "EventLocus should be connected to Force_A");
    assert!(connected_to_b, "EventLocus should be connected to Force_B");
}

#[test]
fn ctx_relationship_by_id_returns_current_state() {
    // Verifies that ConflictActorProgram correctly reads the relationship state
    // via ctx.relationship(id) — if it returns None the program would skip the
    // hostility update and engagement_count would stay 0.
    let (mut world, loci_reg, inf_reg, ab_rel_id) = build_conflict_world();
    let engine = Engine::new(EngineConfig::default());

    engine.tick(
        &mut world,
        &loci_reg,
        &inf_reg,
        vec![ProposedChange::new(
            ChangeSubject::Locus(FORCE_A),
            CONFLICT_KIND,
            StateVector::from_slice(&[0.8]),
        )],
    );

    let rel = world.relationships().get(ab_rel_id).unwrap();
    // engagement_count > 0 proves ctx.relationship(id) returned Some.
    assert!(
        rel.state.as_slice()[ENGAGEMENT_SLOT] > 0.0,
        "engagement_count should be > 0, proving ctx.relationship(id) worked"
    );
}

#[test]
fn extra_slots_present_and_initialised() {
    let (world, _loci_reg, _inf_reg, ab_rel_id) = build_conflict_world();
    let rel = world.relationships().get(ab_rel_id).unwrap();
    let slots = rel.state.as_slice();

    // The relationship was manually inserted with [1.0, 0.0, 0.3, 0.0].
    assert_eq!(slots.len(), 4, "4 slots: activity, weight, hostility, engagement_count");
    assert!((slots[0] - 1.0).abs() < 1e-6, "activity = 1.0");
    assert!((slots[1] - 0.0).abs() < 1e-6, "weight = 0.0");
    assert!((slots[2] - 0.3).abs() < 1e-6, "hostility = 0.3");
    assert!((slots[3] - 0.0).abs() < 1e-6, "engagement_count = 0.0");
}

#[test]
fn subscriber_unsubscribe_stops_notifications() {
    let (mut world, loci_reg, inf_reg, ab_rel_id) = build_conflict_world();
    let engine = Engine::new(EngineConfig::default());

    // Cancel the pre-registered subscription before the tick.
    world.subscriptions_mut().unsubscribe(ANALYST, ab_rel_id);

    engine.tick(
        &mut world,
        &loci_reg,
        &inf_reg,
        vec![ProposedChange::new(
            ChangeSubject::Locus(FORCE_A),
            CONFLICT_KIND,
            StateVector::from_slice(&[0.8]),
        )],
    );

    let tension = world
        .locus(ANALYST)
        .unwrap()
        .state
        .as_slice()
        .first()
        .copied()
        .unwrap_or(0.0);

    assert_eq!(
        tension, 0.0,
        "unsubscribed analyst should not receive notifications and remain at 0 tension"
    );
}

#[test]
fn escalating_conflict_over_multiple_ticks() {
    let (mut world, loci_reg, inf_reg, ab_rel_id) = build_conflict_world();
    let engine = Engine::new(EngineConfig::default());

    let mut prev_hostility = world
        .relationships()
        .get(ab_rel_id)
        .unwrap()
        .state
        .as_slice()[HOSTILITY_SLOT];

    // Three consecutive engagements — hostility should climb each time.
    for tick in 0..3 {
        engine.tick(
            &mut world,
            &loci_reg,
            &inf_reg,
            vec![ProposedChange::new(
                ChangeSubject::Locus(FORCE_A),
                CONFLICT_KIND,
                StateVector::from_slice(&[0.8]),
            )],
        );

        let rel = world.relationships().get(ab_rel_id).unwrap();
        let new_hostility = rel.state.as_slice()[HOSTILITY_SLOT];
        let engagements = rel.state.as_slice()[ENGAGEMENT_SLOT];

        assert!(
            new_hostility >= prev_hostility,
            "tick {tick}: hostility should not decrease ({prev_hostility} → {new_hostility})"
        );
        assert_eq!(
            engagements,
            (tick + 1) as f32,
            "tick {tick}: engagement count should be {}",
            tick + 1
        );
        prev_hostility = new_hostility;
    }
}
