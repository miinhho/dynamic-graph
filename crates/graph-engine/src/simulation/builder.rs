//! Fluent builder for `Simulation`.
//!
//! Eliminates manual `LocusKindId` / `InfluenceKindId` management:
//! the builder assigns IDs internally and lets the user work with
//! string names throughout.
//!
//! ```ignore
//! use graph_engine::SimulationBuilder;
//!
//! let mut sim = SimulationBuilder::new()
//!     .locus_kind("ORG", OrgProgram)
//!     .locus_kind("PERSON", PersonProgram)
//!     .influence("cooccurrence", |cfg| cfg.with_decay(0.95))
//!     .default_influence("cooccurrence")
//!     .build();
//!
//! sim.ingest_named("Apple", "ORG", props! { "confidence" => 0.92 });
//! ```

use graph_core::{DefaultEntityWeathering, Encoder, EntityWeatheringPolicy, LocusKindId, InfluenceKindId, LocusProgram};
use graph_world::World;
use rustc_hash::FxHashMap;

use crate::registry::{
    InfluenceKindConfig, InfluenceKindRegistry, LocusKindConfig, LocusKindRegistry,
};
use super::{Simulation, SimulationConfig};
use crate::engine::EngineConfig;
use crate::regime::AdaptiveConfig;

/// Fluent builder for constructing a `Simulation` from string-based
/// kind names instead of raw numeric IDs.
pub struct SimulationBuilder {
    world: World,
    loci_registry: LocusKindRegistry,
    influence_registry: InfluenceKindRegistry,
    locus_kind_names: FxHashMap<String, LocusKindId>,
    influence_kind_names: FxHashMap<String, InfluenceKindId>,
    next_locus_kind: u64,
    next_influence_kind: u64,
    default_influence: Option<String>,
    config: SimulationConfig,
    auto_weather_policy: Option<Box<dyn EntityWeatheringPolicy>>,
}

impl SimulationBuilder {
    pub fn new() -> Self {
        Self {
            world: World::new(),
            loci_registry: LocusKindRegistry::new(),
            influence_registry: InfluenceKindRegistry::new(),
            locus_kind_names: FxHashMap::default(),
            influence_kind_names: FxHashMap::default(),
            next_locus_kind: 1,
            next_influence_kind: 1,
            default_influence: None,
            config: SimulationConfig::default(),
            auto_weather_policy: None,
        }
    }

    /// Register a locus kind by name with a program.
    ///
    /// ```ignore
    /// builder.locus_kind("ORG", OrgProgram::new())
    /// ```
    pub fn locus_kind(
        mut self,
        name: impl Into<String>,
        program: impl LocusProgram + 'static,
    ) -> Self {
        let name = name.into();
        let id = LocusKindId(self.next_locus_kind);
        self.next_locus_kind += 1;
        self.loci_registry.insert(id, Box::new(program));
        self.locus_kind_names.insert(name, id);
        self
    }

    /// Register a locus kind with full config (encoder, refractory period).
    ///
    /// ```ignore
    /// builder.locus_kind_with("LOCATION", LocationProgram::new(), |cfg| {
    ///     cfg.encoder(GeoEncoder::new()).refractory_batches(2)
    /// })
    /// ```
    pub fn locus_kind_with(
        mut self,
        name: impl Into<String>,
        program: impl LocusProgram + 'static,
        configure: impl FnOnce(LocusKindBuilder) -> LocusKindBuilder,
    ) -> Self {
        let name = name.into();
        let id = LocusKindId(self.next_locus_kind);
        self.next_locus_kind += 1;
        let built = configure(LocusKindBuilder::default());
        self.loci_registry.insert_with_config(id, LocusKindConfig {
            program: Box::new(program),
            refractory_batches: built.refractory_batches,
            encoder: built.encoder,
            max_proposals_per_dispatch: built.max_proposals_per_dispatch,
        });
        self.locus_kind_names.insert(name, id);
        self
    }

    /// Register an influence kind by name.
    ///
    /// ```ignore
    /// builder.influence("cooccurrence", |cfg| cfg.with_decay(0.95))
    /// ```
    pub fn influence(
        mut self,
        name: impl Into<String>,
        configure: impl FnOnce(InfluenceKindConfig) -> InfluenceKindConfig,
    ) -> Self {
        let name = name.into();
        let id = InfluenceKindId(self.next_influence_kind);
        self.next_influence_kind += 1;
        let cfg = configure(InfluenceKindConfig::new(&name));
        self.influence_registry.insert(id, cfg);
        self.influence_kind_names.insert(name, id);
        self
    }

    /// Set which influence kind is used by default in `ingest_named()`.
    pub fn default_influence(mut self, name: impl Into<String>) -> Self {
        self.default_influence = Some(name.into());
        self
    }

    /// Provide a pre-built `World`. If not called, an empty world is used.
    pub fn world(mut self, world: World) -> Self {
        self.world = world;
        self
    }

    /// Configure the engine.
    pub fn engine(mut self, configure: impl FnOnce(EngineConfig) -> EngineConfig) -> Self {
        self.config.engine = configure(self.config.engine);
        self
    }

    /// Configure the adaptive guard rail.
    pub fn adaptive(mut self, configure: impl FnOnce(AdaptiveConfig) -> AdaptiveConfig) -> Self {
        self.config.adaptive = configure(self.config.adaptive);
        self
    }

    /// Set the history window for regime classification.
    pub fn history_window(mut self, window: usize) -> Self {
        self.config.history_window = window;
        self
    }

    /// Enable automatic entity weathering every `every_ticks` steps,
    /// using `DefaultEntityWeathering`.
    pub fn auto_weather(mut self, every_ticks: u32) -> Self {
        assert!(every_ticks > 0, "auto_weather interval must be > 0");
        self.config.auto_weather_every_ticks = Some(every_ticks);
        self.auto_weather_policy = Some(Box::new(DefaultEntityWeathering::default()));
        self
    }

    /// Enable automatic entity weathering every `every_ticks` steps,
    /// using a custom `EntityWeatheringPolicy`.
    pub fn auto_weather_with(
        mut self,
        every_ticks: u32,
        policy: impl EntityWeatheringPolicy + 'static,
    ) -> Self {
        assert!(every_ticks > 0, "auto_weather interval must be > 0");
        self.config.auto_weather_every_ticks = Some(every_ticks);
        self.auto_weather_policy = Some(Box::new(policy));
        self
    }

    /// Build the `Simulation`.
    ///
    /// Panics if `default_influence` was set to a name that was not
    /// registered via `influence()`.
    pub fn build(self) -> Simulation {
        let mut sim = Simulation::with_config(
            self.world,
            self.loci_registry,
            self.influence_registry,
            self.config,
        );

        // Transfer name maps.
        sim.locus_kind_names = self.locus_kind_names;
        sim.influence_kind_names = self.influence_kind_names;

        // Set default influence.
        if let Some(ref name) = self.default_influence {
            let id = sim.influence_kind_names.get(name).unwrap_or_else(|| {
                panic!(
                    "default_influence \"{name}\" not found — register it with .influence() first"
                )
            });
            sim.default_influence = Some(*id);
        }

        // Transfer auto-weather policy (trait objects can't go through SimulationConfig).
        sim.auto_weather_policy = self.auto_weather_policy;

        sim
    }
}

impl Default for SimulationBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Sub-builder for locus kind configuration in `locus_kind_with()`.
#[derive(Default)]
pub struct LocusKindBuilder {
    refractory_batches: u32,
    encoder: Option<Box<dyn Encoder>>,
    max_proposals_per_dispatch: Option<usize>,
}

impl LocusKindBuilder {
    pub fn refractory_batches(mut self, batches: u32) -> Self {
        self.refractory_batches = batches;
        self
    }

    pub fn encoder(mut self, encoder: impl Encoder + 'static) -> Self {
        self.encoder = Some(Box::new(encoder));
        self
    }

    /// Cap the number of `ProposedChange`s this locus kind may produce per dispatch.
    ///
    /// Proposals beyond the limit are silently dropped after `process` returns.
    /// Use this to bound cascades from high-fanout programs.
    pub fn max_proposals(mut self, n: usize) -> Self {
        self.max_proposals_per_dispatch = Some(n);
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use graph_core::{Change, Locus, LocusContext, ProposedChange};

    struct NoopProgram;
    impl LocusProgram for NoopProgram {
        fn process(
            &self,
            _: &Locus,
            _: &[&Change],
            _: &dyn LocusContext,
        ) -> Vec<ProposedChange> {
            Vec::new()
        }
    }

    #[test]
    fn builder_creates_working_simulation() {
        let mut sim = SimulationBuilder::new()
            .locus_kind("ORG", NoopProgram)
            .locus_kind("PERSON", NoopProgram)
            .influence("cooccurrence", |cfg| cfg.with_decay(0.95))
            .default_influence("cooccurrence")
            .build();

        let id = sim.ingest_named("Apple", "ORG", graph_core::props! {
            "confidence" => 0.92_f64,
        });
        assert!(sim.world.locus(id).is_some());
        assert_eq!(sim.resolve("Apple"), Some(id));
    }

    #[test]
    fn builder_batch_ingest_creates_relationships() {
        let mut sim = SimulationBuilder::new()
            .locus_kind("ORG", NoopProgram)
            .locus_kind("PERSON", NoopProgram)
            .influence("cooccurrence", |cfg| cfg.with_decay(0.95))
            .default_influence("cooccurrence")
            .build();

        sim.ingest_batch_named(vec![
            ("Apple", "ORG", graph_core::props! { "confidence" => 0.92_f64 }),
            ("Tim Cook", "PERSON", graph_core::props! { "confidence" => 0.98_f64 }),
        ]);
        let obs = sim.flush_ingested();
        assert!(obs.tick.changes_committed >= 2);
    }

    #[test]
    fn builder_with_refractory_and_encoder() {
        let mut sim = SimulationBuilder::new()
            .locus_kind_with("LOCATION", NoopProgram, |cfg| {
                cfg.refractory_batches(3)
                    .encoder(graph_core::PassthroughEncoder)
            })
            .influence("spatial", |cfg| cfg.with_decay(0.99))
            .default_influence("spatial")
            .build();

        let id = sim.ingest_named("New York", "LOCATION", graph_core::props! {
            "confidence" => 0.85_f64,
        });
        assert_eq!(sim.name_of(id), Some("New York"));
    }

    #[test]
    #[should_panic(expected = "unknown locus kind")]
    fn unregistered_kind_panics() {
        let mut sim = SimulationBuilder::new()
            .influence("signal", |cfg| cfg)
            .default_influence("signal")
            .build();

        sim.ingest_named("X", "MISSING", graph_core::props! {});
    }

    #[test]
    #[should_panic(expected = "no default influence")]
    fn missing_default_influence_panics() {
        let mut sim = SimulationBuilder::new()
            .locus_kind("ORG", NoopProgram)
            .influence("signal", |cfg| cfg)
            // no .default_influence()
            .build();

        sim.ingest_named("X", "ORG", graph_core::props! {});
    }
}
