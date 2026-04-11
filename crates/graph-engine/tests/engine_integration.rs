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
struct InfiniteProgram;
impl LocusProgram for InfiniteProgram {
    fn process(&self, locus: &Locus, _: &[&Change], _: &dyn graph_core::LocusContext) -> Vec<ProposedChange> {
        vec![ProposedChange::new(
            ChangeSubject::Locus(locus.id),
            InfluenceKindId(1),
            locus.state.clone(),
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
    assert_eq!(log.len(), 2);
    assert_eq!(log[0].subject, ChangeSubject::Locus(LocusId(1)));
    assert_eq!(log[0].batch, BatchId(0));
    assert!(log[0].is_stimulus());
    assert_eq!(log[1].subject, ChangeSubject::Locus(LocusId(2)));
    assert_eq!(log[1].batch, BatchId(1));
    assert_eq!(log[1].predecessors, vec![log[0].id]);
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
    assert!((rel.activity() - 1.0).abs() < 1e-6);
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
    assert!((rel.activity() - 3.0).abs() < 1e-6);
    assert_eq!(rel.lineage.change_count, 3);
}

#[test]
fn relationship_activity_decays_each_batch() {
    let (mut world, loci, influences) = forwarder_world(0.5);
    let engine = Engine::default();
    engine.tick(&mut world, &loci, &influences, vec![fire_stimulus(0.5)]);
    engine.flush_relationship_decay(&mut world, &influences);

    let rel = world.relationships().iter().next().unwrap();
    assert!(
        (rel.activity() - 0.5).abs() < 1e-6,
        "expected activity 0.5 after one decay tick, got {}",
        rel.activity()
    );

    engine.tick(&mut world, &loci, &influences, vec![fire_stimulus(0.5)]);
    engine.flush_relationship_decay(&mut world, &influences);
    let rel = world.relationships().iter().next().unwrap();
    assert!(
        (rel.activity() - 0.625).abs() < 1e-6,
        "expected activity 0.625 after second tick, got {}",
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
    let perspective =
        DefaultEmergencePerspective { min_activity_threshold: 0.8, ..Default::default() };
    let perspective_high =
        DefaultEmergencePerspective { min_activity_threshold: 2.0, ..Default::default() };
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
    assert_eq!(rel_changes.len(), 2);
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
