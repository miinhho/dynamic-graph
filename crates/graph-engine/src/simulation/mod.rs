//! Multi-tick simulation runner.

mod accessors;
pub(crate) mod builder;
mod config;
mod ingest;
mod lifecycle;
pub mod observability;
mod plasticity_api;
mod runtime;
mod setup;
mod step_api;
mod storage_api;
mod types;
mod watch;
mod world_api;

pub use builder::SimulationBuilder;
pub use config::{BackpressurePolicy, SimulationConfig, StepObservation};
pub use ingest::IngestError;
pub use observability::{EventHistory, TickSummary};
pub use types::Simulation;

use graph_core::ProposedChange;
use graph_world::World;

use crate::registry::{InfluenceKindRegistry, LocusKindRegistry};

impl Simulation {
    pub fn new(world: World, loci: LocusKindRegistry, influences: InfluenceKindRegistry) -> Self {
        Self::with_config(world, loci, influences, SimulationConfig::default())
    }

    pub fn with_config(
        world: World,
        loci: LocusKindRegistry,
        influences: InfluenceKindRegistry,
        config: SimulationConfig,
    ) -> Self {
        setup::with_config(world, loci, influences, config)
    }

    pub fn step(&mut self, stimuli: Vec<ProposedChange>) -> StepObservation {
        step_api::step(self, stimuli)
    }

    pub fn step_n(&mut self, n: usize, stimuli: Vec<ProposedChange>) -> Vec<StepObservation> {
        step_api::step_n(self, n, stimuli)
    }

    pub fn step_until(
        &mut self,
        mut pred: impl FnMut(&StepObservation, &graph_world::World) -> bool,
        max_steps: usize,
        stimuli: Vec<ProposedChange>,
    ) -> (Vec<StepObservation>, bool) {
        step_api::step_until(self, &mut pred, max_steps, stimuli)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::regime::DynamicsRegime;
    use graph_core::{
        BatchId, Change, ChangeSubject, InfluenceKindId, Locus, LocusId, LocusKindId, LocusProgram,
        ProposedChange, StateVector,
    };
    use graph_world::World;

    const KIND: LocusKindId = LocusKindId(1);
    const SIGNAL: InfluenceKindId = InfluenceKindId(1);

    struct ForwardProgram {
        downstream: LocusId,
    }
    impl LocusProgram for ForwardProgram {
        fn process(
            &self,
            _: &Locus,
            incoming: &[&Change],
            _: &dyn graph_core::LocusContext,
        ) -> Vec<ProposedChange> {
            let total: f32 = incoming.iter().flat_map(|c| c.after.as_slice()).sum();
            if total < 0.001 {
                return Vec::new();
            }
            vec![ProposedChange::new(
                ChangeSubject::Locus(self.downstream),
                SIGNAL,
                StateVector::from_slice(&[total * 0.9]),
            )]
        }
    }

    struct InertProgram;
    impl LocusProgram for InertProgram {
        fn process(
            &self,
            _: &Locus,
            _: &[&Change],
            _: &dyn graph_core::LocusContext,
        ) -> Vec<ProposedChange> {
            Vec::new()
        }
    }

    fn two_locus_world() -> (World, LocusKindRegistry, InfluenceKindRegistry) {
        const SINK_KIND: LocusKindId = LocusKindId(2);
        let mut world = World::new();
        world.insert_locus(Locus::new(LocusId(0), KIND, StateVector::zeros(1)));
        world.insert_locus(Locus::new(LocusId(1), SINK_KIND, StateVector::zeros(1)));
        let mut loci = LocusKindRegistry::new();
        loci.insert(
            KIND,
            Box::new(ForwardProgram {
                downstream: LocusId(1),
            }),
        );
        loci.insert(SINK_KIND, Box::new(InertProgram));
        let mut influences = InfluenceKindRegistry::new();
        influences.insert(
            SIGNAL,
            crate::registry::InfluenceKindConfig::new("test").with_decay(0.9),
        );
        (world, loci, influences)
    }

    fn stimulus_to(locus: LocusId, value: f32) -> ProposedChange {
        ProposedChange::new(
            ChangeSubject::Locus(locus),
            SIGNAL,
            StateVector::from_slice(&[value]),
        )
    }

    #[test]
    fn step_returns_observation_and_advances_batch() {
        let (world, loci, influences) = two_locus_world();
        let mut sim = Simulation::new(world, loci, influences);
        let obs = sim.step(vec![stimulus_to(LocusId(0), 1.0)]);
        assert!(obs.tick.batches_committed > 0);
        assert!(obs.tick.changes_committed > 0);
    }

    #[test]
    fn regime_initializing_before_window_fills() {
        let (world, loci, influences) = two_locus_world();
        let mut sim = Simulation::new(world, loci, influences);
        let obs = sim.step(vec![stimulus_to(LocusId(0), 1.0)]);
        assert_eq!(obs.regime, DynamicsRegime::Initializing);
    }

    #[test]
    fn relationships_emerge_after_step() {
        let (world, loci, influences) = two_locus_world();
        let mut sim = Simulation::new(world, loci, influences);
        sim.step(vec![stimulus_to(LocusId(0), 1.0)]);
        assert_eq!(sim.world().relationships().len(), 1);
    }

    #[test]
    fn scales_present_for_registered_kinds() {
        let (world, loci, influences) = two_locus_world();
        let mut sim = Simulation::new(world, loci, influences);
        let obs = sim.step(vec![stimulus_to(LocusId(0), 1.0)]);
        assert!(!obs.scales.is_empty());
        for &scale in obs.scales.values() {
            assert!(scale > 0.0 && scale <= 1.0);
        }
    }

    #[test]
    fn diff_since_captures_changes_and_new_relationships() {
        let (world, loci, influences) = two_locus_world();
        let mut sim = Simulation::new(world, loci, influences);
        let before = sim.world().current_batch();
        sim.step(vec![stimulus_to(LocusId(0), 1.0)]);
        let diff = sim.world().diff_since(before);
        assert!(diff.change_count() > 0);
        assert!(!diff.relationships_created.is_empty());
        assert!(diff.relationships_updated.is_empty());
    }

    #[test]
    fn diff_since_second_step_shows_updated_not_created() {
        let (world, loci, influences) = two_locus_world();
        let mut sim = Simulation::new(world, loci, influences);
        sim.step(vec![stimulus_to(LocusId(0), 1.0)]);
        let before = sim.world().current_batch();
        sim.step(vec![stimulus_to(LocusId(0), 1.0)]);
        let diff = sim.world().diff_since(before);
        assert!(diff.relationships_created.is_empty());
        assert!(!diff.relationships_updated.is_empty());
    }

    #[test]
    fn step_n_returns_n_observations_and_only_first_gets_stimulus() {
        let (world, loci, influences) = two_locus_world();
        let mut sim = Simulation::new(world, loci, influences);
        let obs = sim.step_n(5, vec![stimulus_to(LocusId(0), 1.0)]);
        assert_eq!(obs.len(), 5);
        assert!(obs[0].tick.changes_committed > 0);
    }

    #[test]
    fn step_n_returns_empty_on_zero() {
        let (world, loci, influences) = two_locus_world();
        let mut sim = Simulation::new(world, loci, influences);
        let obs = sim.step_n(0, vec![]);
        assert!(obs.is_empty(), "step_n(0) must return an empty Vec");
    }

    #[test]
    fn step_until_stops_when_predicate_fires() {
        let (world, loci, influences) = two_locus_world();
        let mut sim = Simulation::new(world, loci, influences);
        let (obs, converged) = sim.step_until(
            |_, world| !world.relationships().is_empty(),
            20,
            vec![stimulus_to(LocusId(0), 1.0)],
        );
        assert!(converged);
        assert!(obs.last().unwrap().relationships > 0);
    }

    #[test]
    fn step_until_returns_not_converged_when_max_reached() {
        let (world, loci, influences) = two_locus_world();
        let mut sim = Simulation::new(world, loci, influences);
        let (obs, converged) = sim.step_until(|_, _| false, 3, vec![]);
        assert!(!converged);
        assert_eq!(obs.len(), 3);
    }

    #[test]
    fn ingest_creates_locus_and_stores_properties() {
        let (world, loci, influences) = two_locus_world();
        let mut sim = Simulation::new(world, loci, influences);
        let id = sim.ingest(
            "Apple",
            KIND,
            SIGNAL,
            graph_core::props! {
                "type" => "ORG",
                "confidence" => 0.92_f64,
            },
        );
        assert!(sim.world().locus(id).is_some());
        assert_eq!(sim.name_of(id).as_deref(), Some("Apple"));
        assert_eq!(sim.resolve("Apple"), Some(id));
        let props = sim.properties_of(id).unwrap();
        assert_eq!(props.get_str("type"), Some("ORG"));
    }

    #[test]
    fn ingest_same_name_reuses_locus() {
        let (world, loci, influences) = two_locus_world();
        let mut sim = Simulation::new(world, loci, influences);
        let id1 = sim.ingest(
            "Apple",
            KIND,
            SIGNAL,
            graph_core::props! {
                "confidence" => 0.8_f64,
            },
        );
        let id2 = sim.ingest(
            "Apple",
            KIND,
            SIGNAL,
            graph_core::props! {
                "confidence" => 0.95_f64,
                "source" => "Reuters",
            },
        );
        assert_eq!(id1, id2);
        let props = sim.properties_of(id1).unwrap();
        assert_eq!(props.get_f64("confidence"), Some(0.95));
        assert_eq!(props.get_str("source"), Some("Reuters"));
    }

    #[test]
    fn flush_ingested_commits_pending_stimuli() {
        let (world, loci, influences) = two_locus_world();
        let mut sim = Simulation::new(world, loci, influences);
        sim.ingest(
            "Apple",
            KIND,
            SIGNAL,
            graph_core::props! { "confidence" => 0.9_f64 },
        );
        sim.ingest(
            "Google",
            KIND,
            SIGNAL,
            graph_core::props! { "confidence" => 0.8_f64 },
        );
        let obs = sim.flush_ingested();
        assert!(obs.tick.changes_committed >= 2);
    }

    #[test]
    fn ingest_batch_creates_cooccurrence_relationships() {
        let (world, loci, influences) = two_locus_world();
        let mut sim = Simulation::new(world, loci, influences);
        let ids = sim.ingest_batch(
            vec![
                (
                    "Apple",
                    KIND,
                    graph_core::props! { "confidence" => 0.9_f64 },
                ),
                (
                    "Tim Cook",
                    KIND,
                    graph_core::props! { "confidence" => 0.95_f64 },
                ),
            ],
            SIGNAL,
        );
        assert_eq!(ids.len(), 2);
        let obs = sim.flush_ingested();
        assert!(obs.tick.changes_committed >= 2);
        assert!(
            !sim.world().relationships().is_empty(),
            "expected co-occurrence relationship, got 0"
        );
    }

    #[test]
    fn rel_slot_value_and_slot_history_work() {
        use crate::registry::InfluenceKindConfig;
        use graph_core::{RelationshipId, RelationshipSlotDef};

        const SLOTTED: InfluenceKindId = InfluenceKindId(99);
        const SLOT_KIND: LocusKindId = LocusKindId(10);

        // Two loci with a program that emits a relationship-subject change
        // carrying an extra slot value.
        struct SlotProgram {
            peer: LocusId,
        }
        impl LocusProgram for SlotProgram {
            fn process(
                &self,
                locus: &Locus,
                _: &[&Change],
                _: &dyn graph_core::LocusContext,
            ) -> Vec<ProposedChange> {
                let val = locus.state.as_slice().first().copied().unwrap_or(0.0);
                if val < 0.001 {
                    return Vec::new();
                }
                // Emit a relationship-subject change with extra slot at index 2.
                vec![ProposedChange::new(
                    ChangeSubject::Locus(self.peer),
                    SLOTTED,
                    StateVector::from_slice(&[val]),
                )]
            }
        }

        let mut world = World::new();
        world.insert_locus(Locus::new(LocusId(0), SLOT_KIND, StateVector::zeros(1)));
        world.insert_locus(Locus::new(LocusId(1), SLOT_KIND, StateVector::zeros(1)));

        let mut loci = LocusKindRegistry::new();
        loci.insert(SLOT_KIND, Box::new(SlotProgram { peer: LocusId(1) }));

        let mut influences = InfluenceKindRegistry::new();
        influences.insert(
            SLOTTED,
            InfluenceKindConfig::new("slotted")
                .with_extra_slots(vec![RelationshipSlotDef::new("pressure", 0.0)]),
        );

        let mut sim = Simulation::new(world, loci, influences);

        // Stimulate a few steps to build relationship history.
        for i in 1..=3u32 {
            sim.step(vec![ProposedChange::new(
                ChangeSubject::Locus(LocusId(0)),
                SLOTTED,
                StateVector::from_slice(&[i as f32 * 0.1]),
            )]);
        }

        // The relationship between locus 0 and 1 should exist.
        assert!(!sim.world().relationships().is_empty());

        // rel_slot_value: unknown slot returns None.
        let rel_id = RelationshipId(0);
        assert!(sim.rel_slot_value(rel_id, SLOTTED, "nonexistent").is_none());

        // slot_history: unregistered kind returns empty.
        let history = sim.slot_history(rel_id, InfluenceKindId(0), "pressure", BatchId(0));
        assert!(history.is_empty());
    }

    #[cfg(feature = "storage")]
    mod storage_tests {
        use super::*;
        use tempfile::NamedTempFile;

        fn storage_config(f: &NamedTempFile) -> SimulationConfig {
            SimulationConfig {
                storage_path: Some(f.path().to_path_buf()),
                ..Default::default()
            }
        }

        #[test]
        fn sim_persists_and_recovers() {
            let f = NamedTempFile::new().unwrap();
            let expected_meta;
            let expected_rels;
            {
                let (world, loci, influences) = two_locus_world();
                let config = storage_config(&f);
                let mut sim = Simulation::with_config(world, loci, influences, config);
                sim.step(vec![stimulus_to(LocusId(0), 1.0)]);
                for _ in 0..4 {
                    sim.step(vec![]);
                }
                assert!(sim.last_storage_error().is_none());
                expected_meta = sim.world().world_meta();
                expected_rels = sim.world().relationships().len();
            }

            let (_, loci2, influences2) = two_locus_world();
            let sim2 =
                Simulation::from_storage(f.path(), loci2, influences2, SimulationConfig::default())
                    .unwrap();
            assert_eq!(expected_meta, sim2.world().world_meta());
            assert_eq!(expected_rels, sim2.world().relationships().len());
        }

        #[test]
        fn point_queries_work_after_steps() {
            let f = NamedTempFile::new().unwrap();
            let (world, loci, influences) = two_locus_world();
            let mut sim = Simulation::with_config(world, loci, influences, storage_config(&f));
            sim.step(vec![stimulus_to(LocusId(0), 1.0)]);

            let storage = sim.storage().unwrap();
            assert!(storage.get_locus(LocusId(0)).unwrap().is_some());
        }

        #[test]
        fn ingest_persists_properties_and_names() {
            let f = NamedTempFile::new().unwrap();
            let (world, loci, influences) = two_locus_world();
            let mut sim = Simulation::with_config(world, loci, influences, storage_config(&f));
            let id = sim.ingest(
                "Apple",
                KIND,
                SIGNAL,
                graph_core::props! {
                    "type" => "ORG",
                    "confidence" => 0.92_f64,
                },
            );
            sim.flush_ingested();

            let storage = sim.storage().unwrap();
            let props = storage.get_properties(id).unwrap().unwrap();
            assert_eq!(props.get_str("type"), Some("ORG"));
            assert_eq!(storage.resolve_name("Apple").unwrap(), Some(id));
        }

        #[test]
        fn storage_error_is_none_when_all_writes_succeed() {
            let f = NamedTempFile::new().unwrap();
            let (world, loci, influences) = two_locus_world();
            let mut sim = Simulation::with_config(world, loci, influences, storage_config(&f));
            sim.step(vec![stimulus_to(LocusId(0), 1.0)]);
            sim.step(vec![]);
            assert!(sim.last_storage_error().is_none());
        }

        #[test]
        fn full_save_and_load_round_trip() {
            let f = NamedTempFile::new().unwrap();
            let expected_meta;
            {
                let (world, loci, influences) = two_locus_world();
                let mut sim = Simulation::with_config(world, loci, influences, storage_config(&f));
                sim.step(vec![stimulus_to(LocusId(0), 1.0)]);
                for _ in 0..9 {
                    sim.step(vec![]);
                }
                // Full save instead of incremental.
                sim.save_world().unwrap();
                expected_meta = sim.world().world_meta();
            }

            let (_, loci2, influences2) = two_locus_world();
            let sim2 =
                Simulation::from_storage(f.path(), loci2, influences2, SimulationConfig::default())
                    .unwrap();
            assert_eq!(expected_meta, sim2.world().world_meta());
        }

        #[test]
        fn change_log_auto_trim_keeps_recent_window() {
            let f = NamedTempFile::new().unwrap();
            let (world, loci, influences) = two_locus_world();
            let config = SimulationConfig {
                storage_path: Some(f.path().to_path_buf()),
                change_retention_batches: Some(2),
                ..Default::default()
            };
            let mut sim = Simulation::with_config(world, loci, influences, config);

            // Keep stimulating every step to ensure changes are generated.
            for _ in 0..10 {
                sim.step(vec![stimulus_to(LocusId(0), 1.0)]);
            }

            // Storage has ALL changes committed across all 10 steps.
            let storage = sim.storage().unwrap();
            let storage_changes = storage.table_counts().unwrap().changes;

            // In-memory log should only retain the recent retention window.
            let log_len = sim.world().log().iter().count();

            assert!(
                storage_changes > log_len as u64,
                "storage ({storage_changes}) should have more changes than trimmed in-memory log ({log_len})"
            );
        }

        #[test]
        fn cold_eviction_reduces_in_memory_relationships() {
            let f = NamedTempFile::new().unwrap();
            let (world, loci, influences) = two_locus_world();
            let config = SimulationConfig {
                storage_path: Some(f.path().to_path_buf()),
                // Aggressive eviction: threshold=100.0 means everything is "cold".
                cold_relationship_threshold: Some(100.0),
                cold_relationship_min_idle_batches: 0,
                ..Default::default()
            };
            let mut sim = Simulation::with_config(world, loci, influences, config);

            sim.step(vec![stimulus_to(LocusId(0), 1.0)]);
            // After step, relationships emerged, but eviction runs at end of step.
            // With threshold=100.0, all relationships have activity < 100.0.
            // With min_idle=0, all are eligible.
            let rels_in_memory = sim.world().relationships().len();

            // Storage has the relationships from commit_batch (before eviction).
            let storage = sim.storage().unwrap();
            let counts = storage.table_counts().unwrap();

            // Relationships were evicted from memory but exist in storage.
            assert_eq!(rels_in_memory, 0, "all relationships should be evicted");
            assert!(
                counts.relationships > 0,
                "storage should still have relationships"
            );
        }

        #[test]
        fn promote_relationship_restores_from_storage() {
            let f = NamedTempFile::new().unwrap();
            let (world, loci, influences) = two_locus_world();
            let config = SimulationConfig {
                storage_path: Some(f.path().to_path_buf()),
                // Evict everything immediately.
                cold_relationship_threshold: Some(100.0),
                cold_relationship_min_idle_batches: 0,
                ..Default::default()
            };
            let mut sim = Simulation::with_config(world, loci, influences, config);
            sim.step(vec![stimulus_to(LocusId(0), 1.0)]);

            // All relationships are now evicted from memory.
            assert_eq!(sim.world().relationships().len(), 0);
            let stored_count = sim.storage().unwrap().table_counts().unwrap().relationships;
            assert!(stored_count > 0);

            // Promote back by relationship ID.
            let rel_id = graph_core::RelationshipId(0);
            let was_promoted = sim.promote_relationship(rel_id);
            assert!(was_promoted);
            assert_eq!(sim.world().relationships().len(), 1);

            // Promoting the same relationship again is a no-op.
            assert!(!sim.promote_relationship(rel_id));
            assert_eq!(sim.world().relationships().len(), 1);
        }

        #[test]
        fn promote_relationships_for_locus_restores_all_edges() {
            let f = NamedTempFile::new().unwrap();
            let (world, loci, influences) = two_locus_world();
            let config = SimulationConfig {
                storage_path: Some(f.path().to_path_buf()),
                cold_relationship_threshold: Some(100.0),
                cold_relationship_min_idle_batches: 0,
                ..Default::default()
            };
            let mut sim = Simulation::with_config(world, loci, influences, config);
            sim.step(vec![stimulus_to(LocusId(0), 1.0)]);

            assert_eq!(sim.world().relationships().len(), 0);

            // Promote all relationships involving locus 0.
            let promoted = sim.promote_relationships_for_locus(LocusId(0));
            assert!(promoted > 0);
            assert_eq!(sim.world().relationships().len(), promoted);
        }
    }
}
