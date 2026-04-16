//! Integration tests for the Engine batch loop.
//!
//! Covers: stimulus commits, program dispatch, predecessor wiring,
//! batch cap, cross-locus flow, relationship auto-emergence, activity
//! decay, entity recognition, weathering, structural proposals,
//! Hebbian plasticity, relationship-subject changes, and change log trim.

use graph_core::{
    BatchId, Change, ChangeSubject, CompressionLevel, DefaultEntityWeathering, Entity,
    EntitySnapshot, EntityWeatheringPolicy, Endpoints, InfluenceKindId, LayerTransition, Locus,
    LocusId, LocusKindId, LocusProgram, ProposedChange, RelationshipId, StateVector,
    StructuralProposal, WeatheringEffect,
};
use graph_engine::{
    DefaultCoherePerspective, DefaultEmergencePerspective, Engine, EngineConfig,
    InfluenceKindConfig, PlasticityConfig,
};
use graph_world::World;

// ─── Local helper programs ────────────────────────────────────────────────────

/// On first activation (stimulus, no predecessors), halves its own state.
/// On subsequent activations does nothing — loop quiesces in two batches.
struct DampOnceProgram;
impl LocusProgram for DampOnceProgram {
    fn process(&self, locus: &Locus, incoming: &[&Change], _: &dyn graph_core::LocusContext) -> Vec<ProposedChange> {
        if incoming.iter().all(|c| c.predecessors.is_empty()) {
            let mut next = locus.state.clone();
            for slot in next.as_mut_slice() {
                *slot *= 0.5;
            }
            vec![ProposedChange::new(ChangeSubject::Locus(locus.id), InfluenceKindId(1), next)]
        } else {
            Vec::new()
        }
    }
}

/// On stimulus, forwards the first incoming change's state to a downstream locus.
/// Ignores non-stimulus changes so the loop quiesces after one hand-off.
struct ForwarderProgram {
    downstream: LocusId,
}
impl LocusProgram for ForwarderProgram {
    fn process(&self, _locus: &Locus, incoming: &[&Change], _ctx: &dyn graph_core::LocusContext) -> Vec<ProposedChange> {
        if !incoming.iter().all(|c| c.predecessors.is_empty()) {
            return Vec::new();
        }
        let after = incoming[0].after.clone();
        vec![ProposedChange::new(ChangeSubject::Locus(self.downstream), InfluenceKindId(1), after)]
    }
}

/// Accepts incoming changes; never proposes anything.
struct SinkProgram;
impl LocusProgram for SinkProgram {
    fn process(&self, _: &Locus, _: &[&Change], _: &dyn graph_core::LocusContext) -> Vec<ProposedChange> {
        Vec::new()
    }
}

/// Always proposes another change — used to drive the batch cap.
/// Emits a strictly increasing state on each activation — never quiesces.
/// Used to verify the batch cap: no-op elision does not apply because the
/// state changes every batch, so only the cap can stop this program.
struct InfiniteProgram;
impl LocusProgram for InfiniteProgram {
    fn process(&self, locus: &Locus, _: &[&Change], _: &dyn graph_core::LocusContext) -> Vec<ProposedChange> {
        let next = locus.state.as_slice().first().copied().unwrap_or(0.0) + 0.001;
        vec![ProposedChange::new(
            ChangeSubject::Locus(locus.id),
            InfluenceKindId(1),
            StateVector::from_slice(&[next]),
        )]
    }
}

/// On stimulus, proposes a new relationship to `new_target` and optionally
/// deletes an existing relationship. Used to test structural proposals.
struct WiringProgram {
    new_target: LocusId,
    delete_rel: Option<RelationshipId>,
}
impl LocusProgram for WiringProgram {
    fn process(&self, _: &Locus, _: &[&Change], _: &dyn graph_core::LocusContext) -> Vec<ProposedChange> {
        Vec::new()
    }
    fn structural_proposals(&self, _: &Locus, incoming: &[&Change], _: &dyn graph_core::LocusContext) -> Vec<StructuralProposal> {
        if incoming.iter().all(|c| c.predecessors.is_empty()) {
            let mut out = vec![StructuralProposal::CreateRelationship {
                endpoints: Endpoints::Directed { from: LocusId(1), to: self.new_target },
                kind: InfluenceKindId(1),
                initial_activity: None,
                initial_state: None,
            }];
            if let Some(rid) = self.delete_rel {
                out.push(StructuralProposal::DeleteRelationship { rel_id: rid });
            }
            out
        } else {
            Vec::new()
        }
    }
}

/// On stimulus, writes a change to a downstream locus; ignores non-stimulus.
struct RelationshipWriterProgram {
    downstream: LocusId,
}
impl LocusProgram for RelationshipWriterProgram {
    fn process(&self, _locus: &Locus, incoming: &[&Change], _ctx: &dyn graph_core::LocusContext) -> Vec<ProposedChange> {
        if !incoming.iter().all(|c| c.predecessors.is_empty()) {
            return Vec::new();
        }
        vec![ProposedChange::new(
            ChangeSubject::Locus(self.downstream),
            InfluenceKindId(1),
            StateVector::from_slice(&[1.0]),
        )]
    }
}

/// Always emits a self-change — used to verify relationship-subject changes
/// do not trigger locus program dispatch.
struct BombProgram;
impl LocusProgram for BombProgram {
    fn process(&self, locus: &Locus, _: &[&Change], _: &dyn graph_core::LocusContext) -> Vec<ProposedChange> {
        vec![ProposedChange::new(
            ChangeSubject::Locus(locus.id),
            InfluenceKindId(1),
            StateVector::from_slice(&[1.0]),
        )]
    }
}

// ─── Helper world builders ────────────────────────────────────────────────────

use graph_engine::{InfluenceKindRegistry, LocusKindRegistry};

fn setup() -> (World, LocusKindRegistry, InfluenceKindRegistry) {
    let mut world = World::new();
    world.insert_locus(Locus::new(LocusId(1), LocusKindId(1), StateVector::zeros(2)));
    let mut loci = LocusKindRegistry::new();
    loci.insert(LocusKindId(1), Box::new(DampOnceProgram));
    let mut influences = InfluenceKindRegistry::new();
    influences.insert(InfluenceKindId(1), InfluenceKindConfig::new("test"));
    (world, loci, influences)
}

fn forwarder_world(decay: f32) -> (World, LocusKindRegistry, InfluenceKindRegistry) {
    let mut world = World::new();
    world.insert_locus(Locus::new(LocusId(1), LocusKindId(1), StateVector::zeros(2)));
    world.insert_locus(Locus::new(LocusId(2), LocusKindId(2), StateVector::zeros(2)));
    let mut loci = LocusKindRegistry::new();
    loci.insert(LocusKindId(1), Box::new(ForwarderProgram { downstream: LocusId(2) }));
    loci.insert(LocusKindId(2), Box::new(SinkProgram));
    let mut influences = InfluenceKindRegistry::new();
    influences.insert(InfluenceKindId(1), InfluenceKindConfig::new("excite").with_decay(decay));
    (world, loci, influences)
}

fn fire_stimulus(value: f32) -> ProposedChange {
    ProposedChange::new(
        ChangeSubject::Locus(LocusId(1)),
        InfluenceKindId(1),
        StateVector::from_slice(&[value, 0.0]),
    )
}

fn two_locus_world_after_forwarding_tick() -> (World, InfluenceKindRegistry) {
    let mut world = World::new();
    world.insert_locus(Locus::new(LocusId(1), LocusKindId(1), StateVector::zeros(1)));
    world.insert_locus(Locus::new(LocusId(2), LocusKindId(2), StateVector::zeros(1)));
    let mut loci = LocusKindRegistry::new();
    loci.insert(LocusKindId(1), Box::new(ForwarderProgram { downstream: LocusId(2) }));
    loci.insert(LocusKindId(2), Box::new(SinkProgram));
    let mut influences = InfluenceKindRegistry::new();
    influences.insert(InfluenceKindId(1), InfluenceKindConfig::new("e"));
    let engine = Engine::default();
    let stimulus = ProposedChange::new(
        ChangeSubject::Locus(LocusId(1)),
        InfluenceKindId(1),
        StateVector::from_slice(&[0.5]),
    );
    engine.tick(&mut world, &loci, &influences, vec![stimulus]);
    (world, influences)
}

fn entity_world_two_layers() -> World {
    let mut world = World::new();
    let store = world.entities_mut();
    let id = store.mint_id();
    let snap = EntitySnapshot {
        members: vec![LocusId(1)],
        member_relationships: Vec::new(),
        coherence: 1.0,
    };
    let mut entity = Entity::born(id, BatchId(0), snap.clone());
    entity.deposit(
        BatchId(1),
        snap,
        LayerTransition::MembershipDelta { added: vec![LocusId(2)], removed: Vec::new() },
    );
    store.insert(entity);
    world.advance_batch();
    world.advance_batch();
    world
}

// ─── Batch loop: stimulus and program dispatch ────────────────────────────────

#[test]
fn stimulus_only_commits_one_batch_when_program_is_passive() {
    struct InertProgram;
    impl LocusProgram for InertProgram {
        fn process(&self, _: &Locus, _: &[&Change], _: &dyn graph_core::LocusContext) -> Vec<ProposedChange> {
            Vec::new()
        }
    }
    let mut world = World::new();
    world.insert_locus(Locus::new(LocusId(1), LocusKindId(1), StateVector::zeros(2)));
    let mut loci = LocusKindRegistry::new();
    loci.insert(LocusKindId(1), Box::new(InertProgram));
    let influences = InfluenceKindRegistry::new();

    let engine = Engine::default();
    let stimulus = ProposedChange::new(
        ChangeSubject::Locus(LocusId(1)),
        InfluenceKindId(1),
        StateVector::from_slice(&[1.0, 1.0]),
    );
    let result = engine.tick(&mut world, &loci, &influences, vec![stimulus]);
    assert_eq!(result.batches_committed, 1);
    assert_eq!(result.changes_committed, 1);
    assert!(!result.hit_batch_cap);
    assert_eq!(world.locus(LocusId(1)).unwrap().state.as_slice(), &[1.0, 1.0]);
}

#[test]
fn stimulus_followed_by_program_response_commits_two_batches() {
    let (mut world, loci, influences) = setup();
    let engine = Engine::default();
    let stimulus = ProposedChange::new(
        ChangeSubject::Locus(LocusId(1)),
        InfluenceKindId(1),
        StateVector::from_slice(&[1.0, 1.0]),
    );
    let result = engine.tick(&mut world, &loci, &influences, vec![stimulus]);
    assert_eq!(result.batches_committed, 2);
    assert_eq!(result.changes_committed, 2);
    assert!(!result.hit_batch_cap);
    assert_eq!(world.locus(LocusId(1)).unwrap().state.as_slice(), &[0.5, 0.5]);
}

#[test]
fn internal_change_inherits_stimulus_as_predecessor() {
    let (mut world, loci, influences) = setup();
    let engine = Engine::default();
    let stimulus = ProposedChange::new(
        ChangeSubject::Locus(LocusId(1)),
        InfluenceKindId(1),
        StateVector::from_slice(&[2.0, 0.0]),
    );
    engine.tick(&mut world, &loci, &influences, vec![stimulus]);

    let log: Vec<&Change> = world.log().iter().collect();
    assert_eq!(log.len(), 2);
    assert!(log[0].is_stimulus());
    assert_eq!(log[1].predecessors, vec![log[0].id]);
    assert_eq!(log[0].batch, BatchId(0));
    assert_eq!(log[1].batch, BatchId(1));
}

#[test]
fn batch_cap_engages_on_runaway_program() {
    let mut world = World::new();
    world.insert_locus(Locus::new(LocusId(1), LocusKindId(1), StateVector::zeros(1)));
    let mut loci = LocusKindRegistry::new();
    loci.insert(LocusKindId(1), Box::new(InfiniteProgram));
    let influences = InfluenceKindRegistry::new();

    let engine = Engine::new(EngineConfig { max_batches_per_tick: 5 });
    let stimulus = ProposedChange::new(
        ChangeSubject::Locus(LocusId(1)),
        InfluenceKindId(1),
        StateVector::from_slice(&[0.1]),
    );
    let result = engine.tick(&mut world, &loci, &influences, vec![stimulus]);
    assert!(result.hit_batch_cap);
    assert_eq!(result.batches_committed, 5);
}

// ─── Cross-locus flow and relationship auto-emergence ─────────────────────────

#[test]
fn cross_locus_change_lands_on_downstream_with_correct_predecessor() {
    let mut world = World::new();
    world.insert_locus(Locus::new(LocusId(1), LocusKindId(1), StateVector::zeros(2)));
    world.insert_locus(Locus::new(LocusId(2), LocusKindId(2), StateVector::zeros(2)));
    let mut loci = LocusKindRegistry::new();
    loci.insert(LocusKindId(1), Box::new(ForwarderProgram { downstream: LocusId(2) }));
    loci.insert(LocusKindId(2), Box::new(SinkProgram));
    let mut influences = InfluenceKindRegistry::new();
    influences.insert(InfluenceKindId(1), InfluenceKindConfig::new("excite"));

    let engine = Engine::default();
    let stimulus = ProposedChange::new(
        ChangeSubject::Locus(LocusId(1)),
        InfluenceKindId(1),
        StateVector::from_slice(&[0.7, 0.0]),
    );
    let result = engine.tick(&mut world, &loci, &influences, vec![stimulus]);
    assert_eq!(result.batches_committed, 2);
    assert_eq!(result.changes_committed, 2);
    assert_eq!(world.locus(LocusId(2)).unwrap().state.as_slice(), &[0.7, 0.0]);
    assert_eq!(world.locus(LocusId(1)).unwrap().state.as_slice(), &[0.7, 0.0]);

    let log: Vec<&Change> = world.log().iter().collect();
    // 2 locus changes + 1 relationship creation entry (auto-emerge writes an
    // explicit ChangeSubject::Relationship log entry so causal queries can
    // find the relationship without relying on the lineage.created_by backlink).
    assert_eq!(log.len(), 3);
    assert_eq!(log[0].subject, ChangeSubject::Locus(LocusId(1)));
    assert_eq!(log[0].batch, BatchId(0));
    assert!(log[0].is_stimulus());
    assert_eq!(log[1].subject, ChangeSubject::Locus(LocusId(2)));
    assert_eq!(log[1].batch, BatchId(1));
    assert_eq!(log[1].predecessors, vec![log[0].id]);
    // Relationship creation entry: appended after the batch commit.
    assert!(matches!(log[2].subject, ChangeSubject::Relationship(_)));
    assert_eq!(log[2].batch, BatchId(1));
    assert_eq!(log[2].predecessors, vec![log[1].id]);
}

#[test]
fn changes_to_locus_returns_full_history() {
    let mut world = World::new();
    world.insert_locus(Locus::new(LocusId(1), LocusKindId(1), StateVector::zeros(1)));
    let mut loci = LocusKindRegistry::new();
    loci.insert(LocusKindId(1), Box::new(SinkProgram));
    let influences = InfluenceKindRegistry::new();

    let engine = Engine::default();
    for value in [0.1_f32, 0.2, 0.3] {
        let stimulus = ProposedChange::new(
            ChangeSubject::Locus(LocusId(1)),
            InfluenceKindId(1),
            StateVector::from_slice(&[value]),
        );
        engine.tick(&mut world, &loci, &influences, vec![stimulus]);
    }

    let to_locus: Vec<f32> = world
        .log()
        .changes_to_locus(LocusId(1))
        .map(|c| c.after.as_slice()[0])
        .collect();
    assert_eq!(to_locus, vec![0.3, 0.2, 0.1]);
}

#[test]
fn cross_locus_flow_emerges_one_directed_relationship() {
    let (mut world, loci, influences) = forwarder_world(1.0);
    let engine = Engine::default();
    engine.tick(&mut world, &loci, &influences, vec![fire_stimulus(0.5)]);

    assert_eq!(world.relationships().len(), 1);
    let rel = world.relationships().iter().next().unwrap();
    assert_eq!(rel.endpoints, Endpoints::Directed { from: LocusId(1), to: LocusId(2) });
    assert_eq!(rel.kind, InfluenceKindId(1));
    // activity = activity_contribution × |pre_signal| = 1.0 × 0.5 = 0.5
    assert!((rel.activity() - 0.5).abs() < 1e-6);
    assert_eq!(rel.lineage.change_count, 1);
}

#[test]
fn repeated_cross_locus_flow_increments_activity_and_change_count() {
    let (mut world, loci, influences) = forwarder_world(1.0);
    let engine = Engine::default();
    for v in [0.1, 0.2, 0.3_f32] {
        engine.tick(&mut world, &loci, &influences, vec![fire_stimulus(v)]);
    }
    assert_eq!(world.relationships().len(), 1);
    let rel = world.relationships().iter().next().unwrap();
    // activity = Σ(1.0 × |signal|) = 0.1 + 0.2 + 0.3 = 0.6
    assert!((rel.activity() - 0.6).abs() < 1e-5);
    assert_eq!(rel.lineage.change_count, 3);
}

#[test]
fn relationship_activity_decays_each_batch() {
    let (mut world, loci, influences) = forwarder_world(0.5);
    let engine = Engine::default();
    engine.tick(&mut world, &loci, &influences, vec![fire_stimulus(0.5)]);
    engine.flush_relationship_decay(&mut world, &influences);

    let rel = world.relationships().iter().next().unwrap();
    // initial = 1.0 × 0.5 = 0.5, flush decay(0.5) → 0.5 × 0.5 = 0.25
    assert!(
        (rel.activity() - 0.25).abs() < 1e-6,
        "expected activity 0.25 after one decay tick, got {}",
        rel.activity()
    );

    engine.tick(&mut world, &loci, &influences, vec![fire_stimulus(0.5)]);
    engine.flush_relationship_decay(&mut world, &influences);
    let rel = world.relationships().iter().next().unwrap();
    // lazy decay: 0.25 × 0.5 = 0.125, += 1.0 × 0.5 = 0.625, flush → 0.625 × 0.5 = 0.3125
    assert!(
        (rel.activity() - 0.3125).abs() < 1e-6,
        "expected activity 0.3125 after second tick, got {}",
        rel.activity()
    );
}

// ─── Entity recognition ───────────────────────────────────────────────────────

#[test]
fn recognize_entities_after_forwarding_tick_produces_one_entity() {
    let (mut world, influences) = two_locus_world_after_forwarding_tick();
    let engine = Engine::default();
    let perspective = DefaultEmergencePerspective::default();
    engine.recognize_entities(&mut world, &influences, &perspective);

    assert_eq!(world.entities().active_count(), 1);
    let entity = world.entities().active().next().unwrap();
    let mut members = entity.current.members.clone();
    members.sort();
    assert_eq!(members, vec![LocusId(1), LocusId(2)]);
    assert_eq!(entity.layer_count(), 1);
}

#[test]
fn entity_becomes_dormant_when_relationship_decays_below_threshold() {
    let (mut world, influences) = two_locus_world_after_forwarding_tick();
    let engine = Engine::default();
    // activity = 1.0 × |0.5| = 0.5; threshold below 0.5 → emerges, above → dormant
    let perspective =
        DefaultEmergencePerspective { min_activity_threshold: 0.3, ..Default::default() };
    let perspective_high =
        DefaultEmergencePerspective { min_activity_threshold: 0.8, ..Default::default() };
    engine.recognize_entities(&mut world, &influences, &perspective);
    assert_eq!(world.entities().active_count(), 1);
    engine.recognize_entities(&mut world, &influences, &perspective_high);
    assert_eq!(world.entities().active_count(), 0);
    assert_eq!(world.entities().len(), 1);
}

#[test]
fn extract_cohere_after_entity_recognition_groups_connected_entities() {
    let mut world = World::new();
    world.insert_locus(Locus::new(LocusId(1), LocusKindId(1), StateVector::zeros(1)));
    world.insert_locus(Locus::new(LocusId(2), LocusKindId(2), StateVector::zeros(1)));
    world.insert_locus(Locus::new(LocusId(3), LocusKindId(3), StateVector::zeros(1)));
    world.insert_locus(Locus::new(LocusId(4), LocusKindId(2), StateVector::zeros(1)));

    let mut loci = LocusKindRegistry::new();
    loci.insert(LocusKindId(1), Box::new(ForwarderProgram { downstream: LocusId(2) }));
    loci.insert(LocusKindId(2), Box::new(SinkProgram));
    loci.insert(LocusKindId(3), Box::new(ForwarderProgram { downstream: LocusId(4) }));

    let mut influences = InfluenceKindRegistry::new();
    influences.insert(InfluenceKindId(1), InfluenceKindConfig::new("e"));

    let engine = Engine::default();
    engine.tick(
        &mut world,
        &loci,
        &influences,
        vec![ProposedChange::new(
            ChangeSubject::Locus(LocusId(1)),
            InfluenceKindId(1),
            StateVector::from_slice(&[1.0]),
        )],
    );
    engine.tick(
        &mut world,
        &loci,
        &influences,
        vec![ProposedChange::new(
            ChangeSubject::Locus(LocusId(3)),
            InfluenceKindId(1),
            StateVector::from_slice(&[1.0]),
        )],
    );

    let ep = DefaultEmergencePerspective::default();
    engine.recognize_entities(&mut world, &influences, &ep);
    assert_eq!(world.entities().active_count(), 2, "expected 2 entities");

    let cp = DefaultCoherePerspective::default();
    engine.extract_cohere(&mut world, &influences, &cp);
    let coheres = world.coheres().get("default").unwrap_or(&[]);
    assert_eq!(coheres.len(), 0, "no bridge -> no cohere");
}

#[test]
fn self_targeted_change_does_not_emerge_relationship() {
    let mut world = World::new();
    world.insert_locus(Locus::new(LocusId(1), LocusKindId(1), StateVector::zeros(2)));
    let mut loci = LocusKindRegistry::new();
    loci.insert(LocusKindId(1), Box::new(DampOnceProgram));
    let mut influences = InfluenceKindRegistry::new();
    influences.insert(InfluenceKindId(1), InfluenceKindConfig::new("self"));
    let engine = Engine::default();
    engine.tick(
        &mut world,
        &loci,
        &influences,
        vec![ProposedChange::new(
            ChangeSubject::Locus(LocusId(1)),
            InfluenceKindId(1),
            StateVector::from_slice(&[1.0, 1.0]),
        )],
    );
    assert_eq!(world.relationships().len(), 0);
}

// ─── Entity weathering ────────────────────────────────────────────────────────

#[test]
fn weather_entities_compresses_old_non_significant_layers() {
    struct AggressiveWeathering;
    impl EntityWeatheringPolicy for AggressiveWeathering {
        fn effect(&self, _layer: &graph_core::EntityLayer, age: u64) -> WeatheringEffect {
            if age >= 3 {
                WeatheringEffect::Remove
            } else if age >= 2 {
                WeatheringEffect::Skeleton
            } else if age >= 1 {
                WeatheringEffect::Compress
            } else {
                WeatheringEffect::Preserved
            }
        }
    }

    let mut world = entity_world_two_layers();
    let engine = Engine::default();
    engine.weather_entities(&mut world, &AggressiveWeathering);

    let entity = world.entities().iter().next().unwrap();
    assert_eq!(entity.layers.len(), 2);
    assert!(matches!(entity.layers[0].compression, CompressionLevel::Skeleton { .. }));
    assert!(matches!(entity.layers[1].compression, CompressionLevel::Compressed { .. }));
}

#[test]
fn weather_entities_never_removes_significant_layer() {
    struct AlwaysRemove;
    impl EntityWeatheringPolicy for AlwaysRemove {
        fn effect(&self, _: &graph_core::EntityLayer, _: u64) -> WeatheringEffect {
            WeatheringEffect::Remove
        }
    }

    let mut world = entity_world_two_layers();
    let engine = Engine::default();
    engine.weather_entities(&mut world, &AlwaysRemove);

    let entity = world.entities().iter().next().unwrap();
    assert_eq!(entity.layers.len(), 1, "non-significant layer removed");
    assert!(matches!(entity.layers[0].compression, CompressionLevel::Skeleton { .. }));
}

#[test]
fn default_entity_weathering_preserves_recent_layers() {
    let mut world = entity_world_two_layers();
    let engine = Engine::default();
    engine.weather_entities(&mut world, &DefaultEntityWeathering::default());

    let entity = world.entities().iter().next().unwrap();
    assert_eq!(entity.layers.len(), 2);
    for layer in &entity.layers {
        assert!(matches!(layer.compression, CompressionLevel::Full));
    }
}

// ─── Structural proposals ─────────────────────────────────────────────────────

#[test]
fn structural_proposal_creates_relationship() {
    let mut world = World::new();
    world.insert_locus(Locus::new(LocusId(1), LocusKindId(1), StateVector::zeros(1)));
    world.insert_locus(Locus::new(LocusId(3), LocusKindId(2), StateVector::zeros(1)));
    let mut loci = LocusKindRegistry::new();
    loci.insert(
        LocusKindId(1),
        Box::new(WiringProgram { new_target: LocusId(3), delete_rel: None }),
    );
    loci.insert(LocusKindId(2), Box::new(SinkProgram));
    let mut inf = InfluenceKindRegistry::new();
    inf.insert(InfluenceKindId(1), InfluenceKindConfig::new("t"));
    let engine = Engine::default();

    assert_eq!(world.relationships().len(), 0);
    engine.tick(
        &mut world,
        &loci,
        &inf,
        vec![ProposedChange::new(
            ChangeSubject::Locus(LocusId(1)),
            InfluenceKindId(1),
            StateVector::from_slice(&[1.0]),
        )],
    );
    assert_eq!(world.relationships().len(), 1, "one relationship created");
    let key = Endpoints::Directed { from: LocusId(1), to: LocusId(3) }.key();
    assert!(world.relationships().lookup(&key, InfluenceKindId(1)).is_some());
}

#[test]
fn structural_proposal_create_existing_is_activity_touch() {
    let mut world = World::new();
    world.insert_locus(Locus::new(LocusId(1), LocusKindId(1), StateVector::zeros(1)));
    world.insert_locus(Locus::new(LocusId(3), LocusKindId(2), StateVector::zeros(1)));
    let mut loci = LocusKindRegistry::new();
    loci.insert(
        LocusKindId(1),
        Box::new(WiringProgram { new_target: LocusId(3), delete_rel: None }),
    );
    loci.insert(LocusKindId(2), Box::new(SinkProgram));
    let mut inf = InfluenceKindRegistry::new();
    inf.insert(InfluenceKindId(1), InfluenceKindConfig::new("t"));
    let engine = Engine::default();

    let stim = || {
        ProposedChange::new(
            ChangeSubject::Locus(LocusId(1)),
            InfluenceKindId(1),
            StateVector::from_slice(&[1.0]),
        )
    };
    engine.tick(&mut world, &loci, &inf, vec![stim()]);
    engine.tick(&mut world, &loci, &inf, vec![stim()]);

    assert_eq!(world.relationships().len(), 1);
    let rel = world.relationships().iter().next().unwrap();
    assert!(rel.activity() > 1.0, "activity should have grown after two touches");
}

#[test]
fn structural_proposal_deletes_relationship() {
    let mut world = World::new();
    world.insert_locus(Locus::new(LocusId(1), LocusKindId(1), StateVector::zeros(1)));
    world.insert_locus(Locus::new(LocusId(2), LocusKindId(2), StateVector::zeros(1)));
    let mut loci = LocusKindRegistry::new();
    loci.insert(LocusKindId(1), Box::new(ForwarderProgram { downstream: LocusId(2) }));
    loci.insert(LocusKindId(2), Box::new(SinkProgram));
    let mut inf = InfluenceKindRegistry::new();
    inf.insert(InfluenceKindId(1), InfluenceKindConfig::new("t"));
    let engine = Engine::default();

    engine.tick(
        &mut world,
        &loci,
        &inf,
        vec![ProposedChange::new(
            ChangeSubject::Locus(LocusId(1)),
            InfluenceKindId(1),
            StateVector::from_slice(&[1.0]),
        )],
    );
    assert_eq!(world.relationships().len(), 1, "relationship emerged");
    let rel_id = world.relationships().iter().next().unwrap().id;

    let mut loci2 = LocusKindRegistry::new();
    loci2.insert(
        LocusKindId(1),
        Box::new(WiringProgram { new_target: LocusId(2), delete_rel: Some(rel_id) }),
    );
    loci2.insert(LocusKindId(2), Box::new(SinkProgram));

    engine.tick(
        &mut world,
        &loci2,
        &inf,
        vec![ProposedChange::new(
            ChangeSubject::Locus(LocusId(1)),
            InfluenceKindId(1),
            StateVector::from_slice(&[1.0]),
        )],
    );
    assert_eq!(world.relationships().len(), 0, "relationship should be deleted");
}

#[test]
fn structural_proposals_default_is_empty_for_existing_programs() {
    let (mut world, loci, influences) = setup();
    let engine = Engine::default();
    let result = engine.tick(
        &mut world,
        &loci,
        &influences,
        vec![ProposedChange::new(
            ChangeSubject::Locus(LocusId(1)),
            InfluenceKindId(1),
            StateVector::from_slice(&[1.0, 0.0]),
        )],
    );
    assert!(!result.hit_batch_cap);
    assert_eq!(world.relationships().len(), 0, "no structural proposals emitted");
}

// ─── Hebbian plasticity ───────────────────────────────────────────────────────

fn two_locus_world_with_plasticity(
    learning_rate: f32,
    weight_decay: f32,
) -> (World, LocusKindRegistry, InfluenceKindRegistry) {
    let mut world = World::new();
    world.insert_locus(Locus::new(LocusId(1), LocusKindId(1), StateVector::zeros(1)));
    world.insert_locus(Locus::new(LocusId(2), LocusKindId(2), StateVector::zeros(1)));
    let mut loci = LocusKindRegistry::new();
    loci.insert(LocusKindId(1), Box::new(ForwarderProgram { downstream: LocusId(2) }));
    loci.insert(LocusKindId(2), Box::new(SinkProgram));
    let mut inf = InfluenceKindRegistry::new();
    inf.insert(
        InfluenceKindId(1),
        InfluenceKindConfig::new("hebb").with_plasticity(PlasticityConfig {
            learning_rate,
            weight_decay,
            max_weight: f32::MAX,
            stdp: false,
        }),
    );
    (world, loci, inf)
}

#[test]
fn hebbian_weight_increases_on_correlated_flow() {
    let (mut world, loci, inf) = two_locus_world_with_plasticity(0.1, 1.0);
    let engine = Engine::default();
    engine.tick(
        &mut world,
        &loci,
        &inf,
        vec![ProposedChange::new(
            ChangeSubject::Locus(LocusId(1)),
            InfluenceKindId(1),
            StateVector::from_slice(&[2.0]),
        )],
    );
    let rel = world.relationships().iter().next().expect("relationship must exist");
    let weight = rel.weight();
    assert!((weight - 0.4).abs() < 1e-5, "expected weight ≈ 0.4, got {weight}");
}

#[test]
fn hebbian_weight_is_zero_when_plasticity_disabled() {
    let (mut world, loci, inf) = two_locus_world_with_plasticity(0.0, 1.0);
    let engine = Engine::default();
    engine.tick(
        &mut world,
        &loci,
        &inf,
        vec![ProposedChange::new(
            ChangeSubject::Locus(LocusId(1)),
            InfluenceKindId(1),
            StateVector::from_slice(&[3.0]),
        )],
    );
    let rel = world.relationships().iter().next().expect("relationship must exist");
    assert!(
        rel.weight().abs() < 1e-6,
        "weight must be 0 when learning_rate=0, got {}",
        rel.weight()
    );
}

#[test]
fn hebbian_weight_accumulates_over_multiple_ticks() {
    let (mut world, loci, inf) = two_locus_world_with_plasticity(0.1, 1.0);
    let engine = Engine::default();
    for _ in 0..3 {
        engine.tick(
            &mut world,
            &loci,
            &inf,
            vec![ProposedChange::new(
                ChangeSubject::Locus(LocusId(1)),
                InfluenceKindId(1),
                StateVector::from_slice(&[1.0]),
            )],
        );
    }
    let weight = world.relationships().iter().next().unwrap().weight();
    assert!((weight - 0.3).abs() < 1e-5, "expected weight ≈ 0.3 after 3 ticks, got {weight}");
}

#[test]
fn hebbian_weight_decays_each_batch() {
    let mut world = World::new();
    world.insert_locus(Locus::new(LocusId(1), LocusKindId(1), StateVector::zeros(1)));
    world.insert_locus(Locus::new(LocusId(2), LocusKindId(2), StateVector::zeros(1)));
    let mut loci = LocusKindRegistry::new();
    loci.insert(LocusKindId(1), Box::new(ForwarderProgram { downstream: LocusId(2) }));
    loci.insert(LocusKindId(2), Box::new(SinkProgram));
    let mut inf = InfluenceKindRegistry::new();
    inf.insert(
        InfluenceKindId(1),
        InfluenceKindConfig::new("hebb").with_plasticity(PlasticityConfig {
            learning_rate: 1.0,
            weight_decay: 0.5,
            max_weight: f32::MAX,
            stdp: false,
        }),
    );
    let engine = Engine::default();

    engine.tick(
        &mut world,
        &loci,
        &inf,
        vec![ProposedChange::new(
            ChangeSubject::Locus(LocusId(1)),
            InfluenceKindId(1),
            StateVector::from_slice(&[1.0]),
        )],
    );
    engine.flush_relationship_decay(&mut world, &inf);
    let w1 = world.relationships().iter().next().unwrap().weight();
    assert!((w1 - 0.5).abs() < 1e-5, "after tick 1: expected weight ≈ 0.5, got {w1}");
}

#[test]
fn hebbian_weight_clamped_by_max_weight() {
    let mut world = World::new();
    world.insert_locus(Locus::new(LocusId(1), LocusKindId(1), StateVector::zeros(1)));
    world.insert_locus(Locus::new(LocusId(2), LocusKindId(2), StateVector::zeros(1)));
    let mut loci = LocusKindRegistry::new();
    loci.insert(LocusKindId(1), Box::new(ForwarderProgram { downstream: LocusId(2) }));
    loci.insert(LocusKindId(2), Box::new(SinkProgram));
    let mut inf = InfluenceKindRegistry::new();
    inf.insert(
        InfluenceKindId(1),
        InfluenceKindConfig::new("hebb").with_plasticity(PlasticityConfig {
            learning_rate: 100.0,
            weight_decay: 1.0,
            max_weight: 2.0,
            stdp: false,
        }),
    );
    let engine = Engine::default();
    engine.tick(
        &mut world,
        &loci,
        &inf,
        vec![ProposedChange::new(
            ChangeSubject::Locus(LocusId(1)),
            InfluenceKindId(1),
            StateVector::from_slice(&[1.0]),
        )],
    );
    let w = world.relationships().iter().next().unwrap().weight();
    assert!(w <= 2.0 + 1e-6, "weight {w} must not exceed max_weight 2.0");
    assert!((w - 2.0).abs() < 1e-5, "weight {w} should be clamped to 2.0");
}

// ─── ChangeSubject::Relationship ─────────────────────────────────────────────

#[test]
fn relationship_subject_change_lands_in_log_and_updates_state() {
    let mut world = World::new();
    world.insert_locus(Locus::new(LocusId(1), LocusKindId(1), StateVector::zeros(1)));
    world.insert_locus(Locus::new(LocusId(2), LocusKindId(2), StateVector::zeros(1)));
    let mut loci = LocusKindRegistry::new();
    loci.insert(
        LocusKindId(1),
        Box::new(RelationshipWriterProgram { downstream: LocusId(2) }),
    );
    loci.insert(LocusKindId(2), Box::new(SinkProgram));
    let mut influences = InfluenceKindRegistry::new();
    influences.insert(InfluenceKindId(1), InfluenceKindConfig::new("t"));

    let engine = Engine::default();
    engine.tick(
        &mut world,
        &loci,
        &influences,
        vec![ProposedChange::new(
            ChangeSubject::Locus(LocusId(1)),
            InfluenceKindId(1),
            StateVector::from_slice(&[1.0]),
        )],
    );
    let rel_id = RelationshipId(0);
    assert!(world.relationships().get(rel_id).is_some());

    let log_len_before = world.log().len();
    engine.tick(
        &mut world,
        &loci,
        &influences,
        vec![ProposedChange::new(
            ChangeSubject::Relationship(rel_id),
            InfluenceKindId(1),
            StateVector::from_slice(&[99.0]),
        )],
    );

    assert_eq!(world.log().len(), log_len_before + 1);
    let rel_change = world.log().iter().last().unwrap();
    assert_eq!(rel_change.subject, ChangeSubject::Relationship(rel_id));
    let activity = world.relationships().get(rel_id).unwrap()
        .state.as_slice().first().copied().unwrap_or(0.0);
    assert!((activity - 99.0).abs() < 1e-4);
}

#[test]
fn relationship_subject_change_does_not_trigger_program_dispatch() {
    let mut world = World::new();
    world.insert_locus(Locus::new(LocusId(1), LocusKindId(1), StateVector::zeros(1)));
    world.insert_locus(Locus::new(LocusId(2), LocusKindId(2), StateVector::zeros(1)));
    let mut loci = LocusKindRegistry::new();
    loci.insert(
        LocusKindId(1),
        Box::new(RelationshipWriterProgram { downstream: LocusId(2) }),
    );
    loci.insert(LocusKindId(2), Box::new(BombProgram));
    let mut influences = InfluenceKindRegistry::new();
    influences.insert(InfluenceKindId(1), InfluenceKindConfig::new("t"));
    let engine = Engine::new(EngineConfig { max_batches_per_tick: 4 });

    engine.tick(
        &mut world,
        &loci,
        &influences,
        vec![ProposedChange::new(
            ChangeSubject::Locus(LocusId(1)),
            InfluenceKindId(1),
            StateVector::from_slice(&[1.0]),
        )],
    );

    let result = engine.tick(
        &mut world,
        &loci,
        &influences,
        vec![ProposedChange::new(
            ChangeSubject::Relationship(RelationshipId(0)),
            InfluenceKindId(1),
            StateVector::from_slice(&[5.0]),
        )],
    );
    assert!(!result.hit_batch_cap, "relationship change must not trigger locus programs");
    assert_eq!(result.batches_committed, 1);
}

#[test]
fn changes_to_relationship_query_returns_relationship_changes() {
    let mut world = World::new();
    world.insert_locus(Locus::new(LocusId(1), LocusKindId(1), StateVector::zeros(1)));
    world.insert_locus(Locus::new(LocusId(2), LocusKindId(2), StateVector::zeros(1)));
    let mut loci = LocusKindRegistry::new();
    loci.insert(
        LocusKindId(1),
        Box::new(RelationshipWriterProgram { downstream: LocusId(2) }),
    );
    loci.insert(LocusKindId(2), Box::new(SinkProgram));
    let mut influences = InfluenceKindRegistry::new();
    influences.insert(InfluenceKindId(1), InfluenceKindConfig::new("t"));
    let engine = Engine::default();

    engine.tick(
        &mut world,
        &loci,
        &influences,
        vec![ProposedChange::new(
            ChangeSubject::Locus(LocusId(1)),
            InfluenceKindId(1),
            StateVector::from_slice(&[1.0]),
        )],
    );
    let rel_id = RelationshipId(0);

    for v in [2.0_f32, 3.0] {
        engine.tick(
            &mut world,
            &loci,
            &influences,
            vec![ProposedChange::new(
                ChangeSubject::Relationship(rel_id),
                InfluenceKindId(1),
                StateVector::from_slice(&[v]),
            )],
        );
    }

    let rel_changes: Vec<_> = world.log().changes_to_relationship(rel_id).collect();
    // 1 auto-emerge creation entry + 2 explicit modifications = 3 total.
    assert_eq!(rel_changes.len(), 3);
    // Newest first: the last explicit change has value 3.0.
    assert!(
        (rel_changes[0].after.as_slice().first().copied().unwrap_or(0.0) - 3.0).abs() < 1e-5
    );
}

// ─── Change log trim ──────────────────────────────────────────────────────────

#[test]
fn trim_change_log_removes_old_batches() {
    let (mut world, loci, influences) = setup();
    let engine = Engine::default();
    let stimulus = ProposedChange::new(
        ChangeSubject::Locus(LocusId(1)),
        InfluenceKindId(1),
        StateVector::from_slice(&[1.0, 1.0]),
    );
    engine.tick(&mut world, &loci, &influences, vec![stimulus]);
    assert_eq!(world.log().len(), 2);

    let removed = engine.trim_change_log(&mut world, 1);
    assert_eq!(removed, 1, "batch 0 change removed");
    assert_eq!(world.log().len(), 1);
    assert!(world.log().iter().all(|c| c.batch.0 >= 1));
}

#[test]
fn trim_change_log_zero_retention_removes_all() {
    let (mut world, loci, influences) = setup();
    let engine = Engine::default();
    let stimulus = ProposedChange::new(
        ChangeSubject::Locus(LocusId(1)),
        InfluenceKindId(1),
        StateVector::from_slice(&[1.0, 1.0]),
    );
    engine.tick(&mut world, &loci, &influences, vec![stimulus]);
    let removed = engine.trim_change_log(&mut world, 0);
    assert_eq!(removed, 2);
    assert_eq!(world.log().len(), 0);
}

#[test]
fn trim_change_log_large_retention_is_noop() {
    let (mut world, loci, influences) = setup();
    let engine = Engine::default();
    let stimulus = ProposedChange::new(
        ChangeSubject::Locus(LocusId(1)),
        InfluenceKindId(1),
        StateVector::from_slice(&[1.0, 0.0]),
    );
    engine.tick(&mut world, &loci, &influences, vec![stimulus]);
    let before = world.log().len();
    let removed = engine.trim_change_log(&mut world, 9999);
    assert_eq!(removed, 0);
    assert_eq!(world.log().len(), before);
}

// ─── DeleteLocus structural proposal ─────────────────────────────────────────

#[test]
fn delete_locus_removes_locus_and_its_relationships() {
    // Two connected loci. After a tick, a relationship exists between them.
    // A program on locus 2 then proposes to delete locus 1.
    struct DeleteL1Program;
    impl LocusProgram for DeleteL1Program {
        fn process(&self, _: &Locus, incoming: &[&Change], _: &dyn graph_core::LocusContext) -> Vec<ProposedChange> {
            let _ = incoming;
            Vec::new()
        }
        fn structural_proposals(&self, _: &Locus, incoming: &[&Change], _: &dyn graph_core::LocusContext) -> Vec<StructuralProposal> {
            if incoming.is_empty() { return Vec::new(); }
            vec![StructuralProposal::delete_locus(LocusId(1))]
        }
    }

    let mut world = World::new();
    world.insert_locus(Locus::new(LocusId(1), LocusKindId(1), StateVector::zeros(1)));
    world.insert_locus(Locus::new(LocusId(2), LocusKindId(2), StateVector::zeros(1)));
    let mut loci = LocusKindRegistry::new();
    loci.insert(LocusKindId(1), Box::new(ForwarderProgram { downstream: LocusId(2) }));
    loci.insert(LocusKindId(2), Box::new(DeleteL1Program));
    let mut influences = InfluenceKindRegistry::new();
    influences.insert(InfluenceKindId(1), InfluenceKindConfig::new("signal"));

    let engine = Engine::default();
    engine.tick(&mut world, &loci, &influences, vec![ProposedChange::new(
        ChangeSubject::Locus(LocusId(1)),
        InfluenceKindId(1),
        StateVector::from_slice(&[1.0]),
    )]);

    assert!(world.locus(LocusId(1)).is_none(), "locus 1 must be removed");
    assert_eq!(world.relationships().len(), 0, "relationship must be removed with locus");
}

#[test]
fn delete_locus_pending_changes_are_dropped() {
    // DeleteL1Program fires on the first batch (locus 2 receives the forwarded change).
    // Any subsequent changes targeting locus 1 must be silently dropped, not panic.
    struct DeleteAndReplyProgram;
    impl LocusProgram for DeleteAndReplyProgram {
        fn process(&self, _: &Locus, incoming: &[&Change], _: &dyn graph_core::LocusContext) -> Vec<ProposedChange> {
            if incoming.is_empty() { return Vec::new(); }
            // Try to reply to locus 1 — it will be deleted this batch.
            vec![ProposedChange::new(
                ChangeSubject::Locus(LocusId(1)),
                InfluenceKindId(1),
                StateVector::from_slice(&[0.5]),
            )]
        }
        fn structural_proposals(&self, _: &Locus, incoming: &[&Change], _: &dyn graph_core::LocusContext) -> Vec<StructuralProposal> {
            if incoming.is_empty() { return Vec::new(); }
            vec![StructuralProposal::delete_locus(LocusId(1))]
        }
    }

    let mut world = World::new();
    world.insert_locus(Locus::new(LocusId(1), LocusKindId(1), StateVector::zeros(1)));
    world.insert_locus(Locus::new(LocusId(2), LocusKindId(2), StateVector::zeros(1)));
    let mut loci = LocusKindRegistry::new();
    loci.insert(LocusKindId(1), Box::new(ForwarderProgram { downstream: LocusId(2) }));
    loci.insert(LocusKindId(2), Box::new(DeleteAndReplyProgram));
    let mut influences = InfluenceKindRegistry::new();
    influences.insert(InfluenceKindId(1), InfluenceKindConfig::new("signal"));

    let engine = Engine::default();
    // Must not panic even though locus 1 is deleted mid-tick.
    engine.tick(&mut world, &loci, &influences, vec![ProposedChange::new(
        ChangeSubject::Locus(LocusId(1)),
        InfluenceKindId(1),
        StateVector::from_slice(&[1.0]),
    )]);

    assert!(world.locus(LocusId(1)).is_none(), "locus 1 must be removed");
}

#[test]
fn delete_locus_no_dangling_relationship_from_deleted_locus() {
    // Regression: after DeleteLocus fires for A in batch 1, a change FROM A
    // that appears as a cross-locus predecessor in batch 2 must NOT cause
    // auto_emerge_relationship to recreate a relationship from the now-deleted
    // locus A.
    //
    // Three-locus setup (required so the forwarded-to locus is different from
    // the deleting locus, which is what actually exercises the fix):
    //
    //   A (kind 1): ForwarderProgram → C.
    //   B (kind 2): on stimulus proposes DeleteLocus(A) via structural_proposals.
    //   C (kind 3): SinkProgram — accepts whatever A forwards.
    //
    // Stimuli: [stim(A), stim(B)] committed together in one tick.
    //
    // Batch 1:
    //   - stim(A) and stim(B) committed.
    //   - A dispatches → ForwarderProgram sends change to C (pending,
    //     derived predecessors = [stim_A.id]).
    //   - B dispatches → structural_proposals yields DeleteLocus(A).
    //   - End of batch: A deleted, all relationships touching A removed.
    //
    // Batch 2:
    //   - C's pending change committed. predecessors = [stim_A.id].
    //   - stim_A.subject = Locus(A). Without the `world.locus(pl).is_some()`
    //     fix, cross_locus_preds would include (A, stim_A) and
    //     auto_emerge_relationship(A, C, …) would create a dangling edge.
    //   - With the fix: world.locus(A).is_some() == false → filtered out.
    //
    // Assertion: no relationship references A after the tick.
    struct DeleteAOnStimulusProgram;
    impl LocusProgram for DeleteAOnStimulusProgram {
        fn process(&self, _: &Locus, _: &[&Change], _: &dyn graph_core::LocusContext) -> Vec<ProposedChange> {
            Vec::new()
        }
        fn structural_proposals(&self, _: &Locus, incoming: &[&Change], _: &dyn graph_core::LocusContext) -> Vec<StructuralProposal> {
            if incoming.is_empty() { return Vec::new(); }
            vec![StructuralProposal::delete_locus(LocusId(1))]
        }
    }

    let a = LocusId(1);
    let b = LocusId(2);
    let c = LocusId(3);

    let mut world = World::new();
    world.insert_locus(Locus::new(a, LocusKindId(1), StateVector::zeros(1)));
    world.insert_locus(Locus::new(b, LocusKindId(2), StateVector::zeros(1)));
    world.insert_locus(Locus::new(c, LocusKindId(3), StateVector::zeros(1)));

    let mut loci = LocusKindRegistry::new();
    // A forwards to C (not B), so C's batch-2 change has A as cross-locus pred.
    loci.insert(LocusKindId(1), Box::new(ForwarderProgram { downstream: c }));
    loci.insert(LocusKindId(2), Box::new(DeleteAOnStimulusProgram));
    loci.insert(LocusKindId(3), Box::new(SinkProgram));

    let mut influences = InfluenceKindRegistry::new();
    influences.insert(InfluenceKindId(1), InfluenceKindConfig::new("signal"));

    let engine = Engine::default();
    engine.tick(&mut world, &loci, &influences, vec![
        ProposedChange::new(ChangeSubject::Locus(a), InfluenceKindId(1), StateVector::from_slice(&[1.0])),
        ProposedChange::new(ChangeSubject::Locus(b), InfluenceKindId(1), StateVector::from_slice(&[1.0])),
    ]);

    // A must be deleted.
    assert!(world.locus(a).is_none(), "locus A must be deleted");

    // No relationship may reference A — the cross-locus auto-emerge must not
    // have recreated an A→C edge in batch 2.
    let dangling = world
        .relationships()
        .iter()
        .any(|r| r.endpoints.involves(a));
    assert!(!dangling, "no relationship should reference the deleted locus A");
}

// ─── Feature: CreateRelationship with_initial_state ───────────────────────────

#[test]
fn create_relationship_initial_state_overrides_entire_vector() {
    // A program that creates a relationship via structural proposal using
    // with_initial_state — all slots should match the provided vector.
    struct MakeRelProgram { to: LocusId }
    impl LocusProgram for MakeRelProgram {
        fn process(&self, _: &Locus, _: &[&Change], _: &dyn graph_core::LocusContext) -> Vec<ProposedChange> {
            Vec::new()
        }
        fn structural_proposals(&self, locus: &Locus, incoming: &[&Change], _: &dyn graph_core::LocusContext) -> Vec<StructuralProposal> {
            if incoming.is_empty() { return Vec::new(); }
            vec![
                StructuralProposal::create_directed(locus.id, self.to, InfluenceKindId(1))
                    .with_initial_state(StateVector::from_slice(&[7.0, 3.0])),
            ]
        }
    }

    let mut world = World::new();
    world.insert_locus(Locus::new(LocusId(1), LocusKindId(1), StateVector::zeros(1)));
    world.insert_locus(Locus::new(LocusId(2), LocusKindId(2), StateVector::zeros(1)));

    let mut loci = LocusKindRegistry::new();
    loci.insert(LocusKindId(1), Box::new(MakeRelProgram { to: LocusId(2) }));
    loci.insert(LocusKindId(2), Box::new(SinkProgram));
    let mut influences = InfluenceKindRegistry::new();
    influences.insert(InfluenceKindId(1), InfluenceKindConfig::new("signal"));

    let engine = Engine::default();
    engine.tick(&mut world, &loci, &influences, vec![
        ProposedChange::new(ChangeSubject::Locus(LocusId(1)), InfluenceKindId(1), StateVector::from_slice(&[1.0])),
    ]);

    let rel = world.relationships().iter()
        .find(|r| r.endpoints.involves(LocusId(1)) && r.endpoints.involves(LocusId(2)))
        .expect("relationship must have been created by structural proposal");
    assert_eq!(rel.activity(), 7.0, "activity (slot 0) from initial_state");
    assert_eq!(rel.weight(), 3.0, "weight (slot 1) from initial_state");
}

#[test]
fn create_relationship_initial_state_takes_precedence_over_initial_activity() {
    // When both initial_state and initial_activity are set, initial_state wins.
    struct MakeRelWithBothProgram { to: LocusId }
    impl LocusProgram for MakeRelWithBothProgram {
        fn process(&self, _: &Locus, _: &[&Change], _: &dyn graph_core::LocusContext) -> Vec<ProposedChange> {
            Vec::new()
        }
        fn structural_proposals(&self, locus: &Locus, incoming: &[&Change], _: &dyn graph_core::LocusContext) -> Vec<StructuralProposal> {
            if incoming.is_empty() { return Vec::new(); }
            // Build via explicit struct — initial_state overrides initial_activity.
            vec![StructuralProposal::CreateRelationship {
                endpoints: Endpoints::directed(locus.id, self.to),
                kind: InfluenceKindId(1),
                initial_activity: Some(99.0), // should be ignored
                initial_state: Some(StateVector::from_slice(&[5.0, 1.0])),
            }]
        }
    }

    let mut world = World::new();
    world.insert_locus(Locus::new(LocusId(1), LocusKindId(1), StateVector::zeros(1)));
    world.insert_locus(Locus::new(LocusId(2), LocusKindId(2), StateVector::zeros(1)));
    let mut loci = LocusKindRegistry::new();
    loci.insert(LocusKindId(1), Box::new(MakeRelWithBothProgram { to: LocusId(2) }));
    loci.insert(LocusKindId(2), Box::new(SinkProgram));
    let mut influences = InfluenceKindRegistry::new();
    influences.insert(InfluenceKindId(1), InfluenceKindConfig::new("signal"));

    Engine::default().tick(&mut world, &loci, &influences, vec![
        ProposedChange::new(ChangeSubject::Locus(LocusId(1)), InfluenceKindId(1), StateVector::from_slice(&[1.0])),
    ]);

    let rel = world.relationships().iter()
        .find(|r| r.endpoints.involves(LocusId(1)) && r.endpoints.involves(LocusId(2)))
        .expect("relationship must exist");
    assert_eq!(rel.activity(), 5.0, "initial_state beats initial_activity");
    assert_eq!(rel.weight(), 1.0);
}

// ─── Feature: recent_changes_to_relationship ─────────────────────────────────

#[test]
fn recent_changes_to_relationship_sees_committed_changes() {
    // A monitor locus subscribed to a relationship uses
    // ctx.recent_changes_to_relationship to count how many changes the rel
    // has accumulated. The count is written into the monitor's own state so
    // we can assert it after the tick.
    struct RelChangeEmitter { rel_id: RelationshipId }
    impl LocusProgram for RelChangeEmitter {
        fn process(&self, _: &Locus, incoming: &[&Change], ctx: &dyn graph_core::LocusContext) -> Vec<ProposedChange> {
            if incoming.is_empty() { return Vec::new(); }
            // Bump the relationship's activity slot via an explicit Change.
            let rel = ctx.relationship(self.rel_id).expect("rel must exist");
            let mut next = rel.state.clone();
            next.as_mut_slice()[0] += 1.0;
            vec![ProposedChange::new(
                ChangeSubject::Relationship(self.rel_id),
                InfluenceKindId(1),
                next,
            )]
        }
    }

    // Monitor: subscribed to the relationship; stores the change count in its own state.
    struct RelChangeMonitor { rel_id: RelationshipId }
    impl LocusProgram for RelChangeMonitor {
        fn process(&self, locus: &Locus, incoming: &[&Change], ctx: &dyn graph_core::LocusContext) -> Vec<ProposedChange> {
            let has_rel_notification = incoming.iter()
                .any(|c| matches!(c.subject, ChangeSubject::Relationship(_)));
            if !has_rel_notification { return Vec::new(); }
            // Count all committed changes to this relationship since batch 0.
            let count = ctx.recent_changes_to_relationship(self.rel_id, BatchId(0)).count();
            let mut next = locus.state.clone();
            next.as_mut_slice()[0] = count as f32;
            vec![ProposedChange::new(
                ChangeSubject::Locus(locus.id),
                InfluenceKindId(1),
                next,
            )]
        }
    }

    let mut world = World::new();
    world.insert_locus(Locus::new(LocusId(1), LocusKindId(1), StateVector::zeros(1)));
    world.insert_locus(Locus::new(LocusId(2), LocusKindId(2), StateVector::zeros(1)));

    // Pre-create the relationship so emitter can reference it by ID.
    let rel_id = world.add_relationship(
        Endpoints::directed(LocusId(1), LocusId(2)),
        InfluenceKindId(1),
        StateVector::from_slice(&[1.0, 0.0]),
    );

    // Subscribe the monitor to the relationship before the first tick.
    world.subscriptions_mut().subscribe_at(LocusId(2), rel_id, None);

    let mut loci = LocusKindRegistry::new();
    loci.insert(LocusKindId(1), Box::new(RelChangeEmitter { rel_id }));
    loci.insert(LocusKindId(2), Box::new(RelChangeMonitor { rel_id }));
    let mut influences = InfluenceKindRegistry::new();
    influences.insert(InfluenceKindId(1), InfluenceKindConfig::new("signal"));

    let engine = Engine::default();

    // Tick 1: emitter fires → 1 relationship change committed.
    engine.tick(&mut world, &loci, &influences, vec![
        ProposedChange::new(ChangeSubject::Locus(LocusId(1)), InfluenceKindId(1), StateVector::from_slice(&[1.0])),
    ]);
    let monitor_state = world.locus(LocusId(2)).unwrap().state.as_slice()[0];
    assert_eq!(monitor_state, 1.0, "monitor must see 1 change after first tick");

    // Tick 2: another stimulus → 2nd relationship change.
    engine.tick(&mut world, &loci, &influences, vec![
        ProposedChange::new(ChangeSubject::Locus(LocusId(1)), InfluenceKindId(1), StateVector::from_slice(&[1.0])),
    ]);
    let monitor_state = world.locus(LocusId(2)).unwrap().state.as_slice()[0];
    assert_eq!(monitor_state, 2.0, "monitor must see 2 changes after second tick");
}

// ─── Feature: relationship_patch (slot_patches) ───────────────────────────────

#[test]
fn relationship_patch_updates_only_specified_slots() {
    // A program emits ProposedChange::relationship_patch, which should update
    // only the named slot without touching the weight slot (slot 1).
    const EXTRA_SLOT: usize = 2;

    struct PatchRelProgram { rel_id: RelationshipId }
    impl LocusProgram for PatchRelProgram {
        fn process(&self, _: &Locus, incoming: &[&Change], ctx: &dyn graph_core::LocusContext) -> Vec<ProposedChange> {
            if incoming.is_empty() { return vec![]; }
            // Only patch slot 2 by +0.5; slot 1 (weight) must be untouched.
            let _ = ctx.relationship(self.rel_id); // verify it exists
            vec![ProposedChange::relationship_patch(self.rel_id, InfluenceKindId(1), &[(EXTRA_SLOT, 0.5)])]
        }
    }

    let mut world = World::new();
    world.insert_locus(Locus::new(LocusId(1), LocusKindId(1), StateVector::zeros(1)));
    world.insert_locus(Locus::new(LocusId(2), LocusKindId(2), StateVector::zeros(1)));
    // Pre-create rel with weight=0.9 to verify patch doesn't touch it.
    let rel_id = world.add_relationship(
        Endpoints::directed(LocusId(1), LocusId(2)),
        InfluenceKindId(1),
        StateVector::from_slice(&[1.0, 0.9, 0.0]), // [activity, weight=0.9, extra=0.0]
    );

    let mut loci = LocusKindRegistry::new();
    loci.insert(LocusKindId(1), Box::new(PatchRelProgram { rel_id }));
    loci.insert(LocusKindId(2), Box::new(SinkProgram));
    let mut influences = InfluenceKindRegistry::new();
    influences.insert(InfluenceKindId(1), InfluenceKindConfig::new("signal"));

    Engine::default().tick(&mut world, &loci, &influences, vec![
        ProposedChange::new(ChangeSubject::Locus(LocusId(1)), InfluenceKindId(1), StateVector::from_slice(&[1.0])),
    ]);

    let rel = world.relationships().get(rel_id).expect("rel must exist");
    assert_eq!(rel.state.as_slice().get(EXTRA_SLOT).copied().unwrap_or(0.0), 0.5, "slot 2 must be incremented");
    assert_eq!(rel.weight(), 0.9, "weight slot must be preserved by patch");
}

#[test]
fn relationship_patch_accumulates_across_ticks() {
    // Multiple ticks of slot-delta patches must accumulate correctly.
    const EXTRA_SLOT: usize = 2;

    struct DeltaRelProgram { rel_id: RelationshipId }
    impl LocusProgram for DeltaRelProgram {
        fn process(&self, _: &Locus, incoming: &[&Change], _: &dyn graph_core::LocusContext) -> Vec<ProposedChange> {
            if incoming.is_empty() { return vec![]; }
            vec![ProposedChange::relationship_patch(self.rel_id, InfluenceKindId(1), &[(EXTRA_SLOT, 0.1)])]
        }
    }

    let mut world = World::new();
    world.insert_locus(Locus::new(LocusId(1), LocusKindId(1), StateVector::zeros(1)));
    world.insert_locus(Locus::new(LocusId(2), LocusKindId(2), StateVector::zeros(1)));
    let rel_id = world.add_relationship(
        Endpoints::directed(LocusId(1), LocusId(2)),
        InfluenceKindId(1),
        StateVector::from_slice(&[1.0, 0.0, 0.0]),
    );

    let mut loci = LocusKindRegistry::new();
    loci.insert(LocusKindId(1), Box::new(DeltaRelProgram { rel_id }));
    loci.insert(LocusKindId(2), Box::new(SinkProgram));
    let mut influences = InfluenceKindRegistry::new();
    influences.insert(InfluenceKindId(1), InfluenceKindConfig::new("signal"));

    let engine = Engine::default();
    for _ in 0..3 {
        engine.tick(&mut world, &loci, &influences, vec![
            ProposedChange::new(ChangeSubject::Locus(LocusId(1)), InfluenceKindId(1), StateVector::from_slice(&[1.0])),
        ]);
    }

    let slot_val = world.relationships().get(rel_id)
        .and_then(|r| r.state.as_slice().get(EXTRA_SLOT).copied())
        .unwrap_or(0.0);
    assert!((slot_val - 0.3).abs() < 1e-5, "slot must accumulate 3 × 0.1 = 0.3, got {slot_val}");
}

// ─── Feature: direction-aware LocusContext methods ────────────────────────────

#[test]
fn incoming_relationships_of_kind_filters_to_arriving_directed_edges() {
    // A program reads ctx.incoming_relationships_of_kind and counts them.
    // The world has: A→C (directed), B→C (directed), C→D (directed).
    // From C's perspective: 2 incoming (A,B), 1 outgoing (D).
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    let incoming_seen = Arc::new(AtomicUsize::new(0));
    let outgoing_seen = Arc::new(AtomicUsize::new(0));
    let in_clone = Arc::clone(&incoming_seen);
    let out_clone = Arc::clone(&outgoing_seen);

    struct DirectionCountProgram {
        incoming: Arc<AtomicUsize>,
        outgoing: Arc<AtomicUsize>,
    }
    impl LocusProgram for DirectionCountProgram {
        fn process(&self, locus: &Locus, incoming: &[&Change], ctx: &dyn graph_core::LocusContext) -> Vec<ProposedChange> {
            if incoming.is_empty() { return vec![]; }
            let in_count = ctx.incoming_relationships_of_kind(locus.id, InfluenceKindId(1)).count();
            let out_count = ctx.outgoing_relationships_of_kind(locus.id, InfluenceKindId(1)).count();
            self.incoming.store(in_count, Ordering::Relaxed);
            self.outgoing.store(out_count, Ordering::Relaxed);
            vec![]
        }
    }

    const C: LocusId = LocusId(3);

    let mut world = World::new();
    for id in [1u64, 2, 3, 4] {
        world.insert_locus(Locus::new(LocusId(id), LocusKindId(id), StateVector::zeros(1)));
    }
    // A(1)→C(3), B(2)→C(3), C(3)→D(4)
    for (from, to) in [(1u64, 3u64), (2, 3), (3, 4)] {
        world.add_relationship(
            Endpoints::directed(LocusId(from), LocusId(to)),
            InfluenceKindId(1),
            StateVector::from_slice(&[1.0, 0.0]),
        );
    }

    let mut loci = LocusKindRegistry::new();
    loci.insert(LocusKindId(1), Box::new(ForwarderProgram { downstream: C }));
    loci.insert(LocusKindId(2), Box::new(ForwarderProgram { downstream: C }));
    loci.insert(LocusKindId(3), Box::new(DirectionCountProgram {
        incoming: in_clone,
        outgoing: out_clone,
    }));
    loci.insert(LocusKindId(4), Box::new(SinkProgram));

    let mut influences = InfluenceKindRegistry::new();
    influences.insert(InfluenceKindId(1), InfluenceKindConfig::new("signal"));

    Engine::default().tick(&mut world, &loci, &influences, vec![
        ProposedChange::new(ChangeSubject::Locus(LocusId(1)), InfluenceKindId(1), StateVector::from_slice(&[1.0])),
        ProposedChange::new(ChangeSubject::Locus(LocusId(2)), InfluenceKindId(1), StateVector::from_slice(&[1.0])),
    ]);

    assert_eq!(incoming_seen.load(Ordering::Relaxed), 2, "C has 2 incoming directed edges");
    assert_eq!(outgoing_seen.load(Ordering::Relaxed), 1, "C has 1 outgoing directed edge");
}

// ─── Feature: relationship_changes / locus_changes inbox helpers ─────────────

#[test]
fn relationship_changes_and_locus_changes_partition_inbox() {
    use graph_core::{locus_changes, relationship_changes};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    let rel_count = Arc::new(AtomicUsize::new(0));
    let loc_count = Arc::new(AtomicUsize::new(0));
    let rel_clone = Arc::clone(&rel_count);
    let loc_clone = Arc::clone(&loc_count);

    struct InboxPartitionProgram {
        rel_count: Arc<AtomicUsize>,
        loc_count: Arc<AtomicUsize>,
    }
    impl LocusProgram for InboxPartitionProgram {
        fn process(&self, _: &Locus, incoming: &[&Change], _: &dyn graph_core::LocusContext) -> Vec<ProposedChange> {
            self.rel_count.store(relationship_changes(incoming).len(), Ordering::Relaxed);
            self.loc_count.store(locus_changes(incoming).len(), Ordering::Relaxed);
            vec![]
        }
    }

    let mut world = World::new();
    world.insert_locus(Locus::new(LocusId(1), LocusKindId(1), StateVector::zeros(1)));
    world.insert_locus(Locus::new(LocusId(2), LocusKindId(2), StateVector::zeros(1)));

    // Pre-create a relationship and subscribe locus 2 to it.
    let rel_id = world.add_relationship(
        Endpoints::directed(LocusId(1), LocusId(2)),
        InfluenceKindId(1),
        StateVector::from_slice(&[1.0, 0.0]),
    );
    world.subscriptions_mut().subscribe_at(LocusId(2), rel_id, None);

    // Locus 1 program: emits a locus change AND a relationship change.
    struct DualEmitterProgram { rel_id: RelationshipId }
    impl LocusProgram for DualEmitterProgram {
        fn process(&self, _: &Locus, incoming: &[&Change], ctx: &dyn graph_core::LocusContext) -> Vec<ProposedChange> {
            if incoming.is_empty() { return vec![]; }
            let rel = ctx.relationship(self.rel_id).expect("rel must exist");
            vec![
                // Relationship change (notifies subscriber locus 2).
                ProposedChange::new(
                    ChangeSubject::Relationship(self.rel_id),
                    InfluenceKindId(1),
                    rel.state.clone().with_slot_delta(0, 1.0),
                ),
            ]
        }
    }

    let mut loci = LocusKindRegistry::new();
    loci.insert(LocusKindId(1), Box::new(DualEmitterProgram { rel_id }));
    loci.insert(LocusKindId(2), Box::new(InboxPartitionProgram {
        rel_count: rel_clone,
        loc_count: loc_clone,
    }));

    let mut influences = InfluenceKindRegistry::new();
    influences.insert(InfluenceKindId(1), InfluenceKindConfig::new("signal"));

    Engine::default().tick(&mut world, &loci, &influences, vec![
        ProposedChange::new(ChangeSubject::Locus(LocusId(1)), InfluenceKindId(1), StateVector::from_slice(&[1.0])),
    ]);

    // Locus 2 receives: 1 relationship change (via subscription) + 0 direct locus changes.
    assert_eq!(rel_count.load(Ordering::Relaxed), 1, "subscriber sees 1 relationship change");
    assert_eq!(loc_count.load(Ordering::Relaxed), 0, "no direct locus changes in subscriber inbox");
}

// ── ctx.relationships_between (plural) ───────────────────────────────────────

/// Program that reads the relationship count between `self` and `peer` via
/// `ctx.relationships_between(locus.id, peer)` and records it.
struct CountEdgesBetweenProgram {
    peer: LocusId,
    count_out: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}
impl LocusProgram for CountEdgesBetweenProgram {
    fn process(&self, locus: &Locus, _incoming: &[&Change], ctx: &dyn graph_core::LocusContext) -> Vec<ProposedChange> {
        let n = ctx.relationships_between(locus.id, self.peer).count();
        self.count_out.store(n, std::sync::atomic::Ordering::Relaxed);
        vec![]
    }
}

#[test]
fn ctx_relationships_between_returns_all_edges_between_pair() {
    use graph_engine::LocusKindRegistry;
    use std::sync::{Arc, atomic::{AtomicUsize, Ordering}};

    let mut world = World::new();
    world.insert_locus(Locus::new(LocusId(1), LocusKindId(1), StateVector::zeros(1)));
    world.insert_locus(Locus::new(LocusId(2), LocusKindId(2), StateVector::zeros(1)));

    // Two edges between L1 and L2 of different kinds.
    world.add_relationship(Endpoints::directed(LocusId(1), LocusId(2)), InfluenceKindId(1), StateVector::from_slice(&[1.0, 0.0]));
    world.add_relationship(Endpoints::directed(LocusId(1), LocusId(2)), InfluenceKindId(2), StateVector::from_slice(&[1.0, 0.0]));

    let count = Arc::new(AtomicUsize::new(0));
    let count_clone = count.clone();

    let mut loci = LocusKindRegistry::new();
    loci.insert(LocusKindId(1), Box::new(CountEdgesBetweenProgram {
        peer: LocusId(2),
        count_out: count_clone,
    }));
    loci.insert(LocusKindId(2), Box::new(SinkProgram));

    let mut influences = graph_engine::InfluenceKindRegistry::new();
    influences.insert(InfluenceKindId(1), InfluenceKindConfig::new("kind1"));
    influences.insert(InfluenceKindId(2), InfluenceKindConfig::new("kind2"));

    Engine::default().tick(
        &mut world, &loci, &influences,
        vec![ProposedChange::activation(LocusId(1), InfluenceKindId(1), 1.0)],
    );

    assert_eq!(count.load(Ordering::Relaxed), 2, "ctx.relationships_between returns both edges");
}

// ── ProposedChange::relationship_slot_patch ───────────────────────────────────

#[test]
fn relationship_slot_patch_single_slot_convenience() {
    use graph_engine::{InfluenceKindRegistry, LocusKindRegistry};

    let mut world = World::new();
    world.insert_locus(Locus::new(LocusId(1), LocusKindId(1), StateVector::zeros(1)));
    world.insert_locus(Locus::new(LocusId(2), LocusKindId(2), StateVector::zeros(1)));

    // Pre-create a relationship with an extra slot (index 2 = 0.5).
    let rel_id = world.add_relationship(
        Endpoints::directed(LocusId(1), LocusId(2)),
        InfluenceKindId(1),
        StateVector::from_slice(&[1.0, 0.0, 0.5]),
    );

    // Program uses relationship_slot_patch to increment slot 2 by 0.3.
    struct SlotPatchProgram { rel_id: RelationshipId }
    impl LocusProgram for SlotPatchProgram {
        fn process(&self, _locus: &Locus, _incoming: &[&Change], _ctx: &dyn graph_core::LocusContext) -> Vec<ProposedChange> {
            vec![ProposedChange::relationship_slot_patch(self.rel_id, InfluenceKindId(1), 2, 0.3)]
        }
    }

    let mut loci = LocusKindRegistry::new();
    loci.insert(LocusKindId(1), Box::new(SlotPatchProgram { rel_id }));
    loci.insert(LocusKindId(2), Box::new(SinkProgram));

    let mut influences = InfluenceKindRegistry::new();
    influences.insert(InfluenceKindId(1), InfluenceKindConfig::new("sig"));

    Engine::default().tick(
        &mut world, &loci, &influences,
        vec![ProposedChange::activation(LocusId(1), InfluenceKindId(1), 1.0)],
    );

    let slot2 = world.relationships().get(rel_id).unwrap().state.as_slice()[2];
    assert!((slot2 - 0.8).abs() < 1e-5, "expected 0.5 + 0.3 = 0.8, got {slot2}");
}

// ─── Task 1: Cross-kind interaction rules in the batch loop ──────────────────

/// Forwards each incoming root-stimulus change to `downstream`, preserving the
/// original influence kind. This lets tests produce multi-kind flows through a
/// single edge in one tick — something `ForwarderProgram` (which always emits
/// kind 1) cannot do.
struct KindPreservingForwarder {
    downstream: LocusId,
}
impl LocusProgram for KindPreservingForwarder {
    fn process(&self, _: &Locus, incoming: &[&Change], _: &dyn graph_core::LocusContext) -> Vec<ProposedChange> {
        incoming
            .iter()
            .filter(|c| c.predecessors.is_empty()) // root stimuli only
            .map(|c| ProposedChange::new(ChangeSubject::Locus(self.downstream), c.kind, c.after.clone()))
            .collect()
    }
}

/// Helper: two-locus world with a kind-preserving forwarder at locus 1.
///
/// Both kinds must be pre-registered; the helper registers no interactions —
/// callers do that after receiving the registry.
fn interaction_world(
    kind_a: InfluenceKindId,
    kind_b: InfluenceKindId,
) -> (World, LocusKindRegistry, graph_engine::InfluenceKindRegistry) {
    use graph_engine::InfluenceKindRegistry;
    let mut world = World::new();
    world.insert_locus(Locus::new(LocusId(1), LocusKindId(1), StateVector::zeros(1)));
    world.insert_locus(Locus::new(LocusId(2), LocusKindId(2), StateVector::zeros(1)));
    let mut loci = LocusKindRegistry::new();
    loci.insert(LocusKindId(1), Box::new(KindPreservingForwarder { downstream: LocusId(2) }));
    loci.insert(LocusKindId(2), Box::new(SinkProgram));
    let mut influences = InfluenceKindRegistry::new();
    influences.insert(kind_a, InfluenceKindConfig::new("kind_a"));
    influences.insert(kind_b, InfluenceKindConfig::new("kind_b"));
    (world, loci, influences)
}

/// Fire both kinds as root stimuli in the same tick, returning the activity of
/// the kind_a relationship between locus 1 and locus 2 after the tick.
fn fire_two_kinds(
    world: &mut World,
    loci: &LocusKindRegistry,
    influences: &graph_engine::InfluenceKindRegistry,
    kind_a: InfluenceKindId,
    kind_b: InfluenceKindId,
) -> f32 {
    Engine::default().tick(
        world,
        loci,
        influences,
        vec![
            ProposedChange::new(ChangeSubject::Locus(LocusId(1)), kind_a, StateVector::from_slice(&[1.0])),
            ProposedChange::new(ChangeSubject::Locus(LocusId(1)), kind_b, StateVector::from_slice(&[1.0])),
        ],
    );
    // Both kind_a and kind_b create separate relationships between locus1 and locus2.
    // The interaction multiplier is applied to both; we return the kind_a one.
    world
        .relationships()
        .iter()
        .find(|r| r.kind == kind_a && r.involves(LocusId(1)) && r.involves(LocusId(2)))
        .map(|r| r.activity())
        .expect("kind_a relationship must have auto-emerged")
}

/// Two kinds with a Synergistic{boost:2.0} interaction.
///
/// Flow: locus1 receives [stimulus_k1, stimulus_k2] in one tick. The
/// KindPreservingForwarder emits one forwarded change per kind to locus2.
/// Each forwarded change carries both inbox changes (A, B) as derived
/// predecessors, so each forwarded change produces 2 cross-locus auto-emerges.
/// Total touches per kind: 2 (→ activity = 2.0 per kind-relationship).
/// Both relationships share the same endpoint pair → interaction fires.
/// With Synergistic{boost:2.0}: both relationships' activity × 2.0 → 4.0.
#[test]
fn synergistic_interaction_boosts_activity() {
    use graph_core::InteractionEffect;
    let (kind_a, kind_b) = (InfluenceKindId(1), InfluenceKindId(2));
    let (mut world, loci, mut influences) = interaction_world(kind_a, kind_b);
    influences.register_interaction(kind_a, kind_b, InteractionEffect::Synergistic { boost: 2.0 });

    let activity = fire_two_kinds(&mut world, &loci, &influences, kind_a, kind_b);
    // 2 touches per kind → activity = 2.0 per relationship; × boost 2.0 = 4.0
    assert!(
        (activity - 4.0).abs() < 1e-4,
        "expected activity ≈ 4.0 (synergistic boost), got {activity}"
    );
}

/// Two kinds with an Antagonistic{dampen:0.5} interaction.
///
/// 2 touches per kind → activity = 2.0, dampened × 0.5 = 1.0.
#[test]
fn antagonistic_interaction_dampens_activity() {
    use graph_core::InteractionEffect;
    let (kind_a, kind_b) = (InfluenceKindId(1), InfluenceKindId(2));
    let (mut world, loci, mut influences) = interaction_world(kind_a, kind_b);
    influences.register_interaction(kind_a, kind_b, InteractionEffect::Antagonistic { dampen: 0.5 });

    let activity = fire_two_kinds(&mut world, &loci, &influences, kind_a, kind_b);
    // 2 touches per kind → activity = 2.0; × dampen 0.5 = 1.0
    assert!(
        (activity - 1.0).abs() < 1e-4,
        "expected activity ≈ 1.0 (antagonistic dampen), got {activity}"
    );
}

/// Only one kind touches the edge — no interaction should be applied.
///
/// Even though a Synergistic{boost:10.0} rule exists for the pair, it must
/// not fire when only one kind touches the edge in a single batch.
#[test]
fn single_kind_no_interaction_applied() {
    use graph_core::InteractionEffect;
    use graph_engine::InfluenceKindRegistry;

    let mut world = World::new();
    world.insert_locus(Locus::new(LocusId(1), LocusKindId(1), StateVector::zeros(1)));
    world.insert_locus(Locus::new(LocusId(2), LocusKindId(2), StateVector::zeros(1)));

    let mut loci = LocusKindRegistry::new();
    loci.insert(LocusKindId(1), Box::new(KindPreservingForwarder { downstream: LocusId(2) }));
    loci.insert(LocusKindId(2), Box::new(SinkProgram));

    let mut influences = InfluenceKindRegistry::new();
    influences.insert(InfluenceKindId(1), InfluenceKindConfig::new("only_kind"));
    influences.insert(InfluenceKindId(2), InfluenceKindConfig::new("other"));
    // Rule exists, but kind 2 never fires.
    influences.register_interaction(
        InfluenceKindId(1),
        InfluenceKindId(2),
        InteractionEffect::Synergistic { boost: 10.0 },
    );

    Engine::default().tick(
        &mut world,
        &loci,
        &influences,
        vec![
            ProposedChange::new(ChangeSubject::Locus(LocusId(1)), InfluenceKindId(1), StateVector::from_slice(&[1.0])),
        ],
    );

    let rel = world
        .relationships()
        .iter()
        .find(|r| r.involves(LocusId(1)) && r.involves(LocusId(2)))
        .expect("relationship must have auto-emerged");

    // Only 1 touch, no cross-kind interaction → activity = 1.0 (unmodified).
    assert!(
        (rel.activity() - 1.0).abs() < 1e-4,
        "expected activity ≈ 1.0 (no interaction), got {}",
        rel.activity()
    );
}
