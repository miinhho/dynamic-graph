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

use graph_core::{DefaultEntityWeathering, Encoder, EntityWeatheringPolicy, LocusId, LocusKindId, InfluenceKindId, LocusProgram, RelationshipId};
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
            name: Some(name.clone()),
            state_slots: built.state_slots,
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

    /// Returns the initial `StateVector` for relationships of a named influence kind.
    ///
    /// Delegates to `InfluenceKindRegistry::initial_state_for`. Use this during
    /// bootstrap to create pre-wired relationships with the correct slot count
    /// (base `[activity, weight]` + any `extra_slots` declared via `.influence()`):
    ///
    /// ```ignore
    /// let state = builder.initial_relationship_state("supply");
    /// let rel = builder.world_mut().add_relationship(endpoints, supply_kind_id, state);
    /// ```
    ///
    /// Panics in debug builds if `kind_name` was not registered.
    pub fn initial_relationship_state(&self, kind_name: &str) -> graph_core::StateVector {
        let id = self.influence_kind_names.get(kind_name).copied().unwrap_or_else(|| {
            panic!("initial_relationship_state: influence kind \"{kind_name}\" not registered — call .influence() first");
        });
        self.influence_registry.initial_state_for(id)
    }

    /// Look up the `InfluenceKindId` for a registered influence kind name.
    ///
    /// Returns `None` when the name has not been registered.
    pub fn influence_kind(&self, name: &str) -> Option<InfluenceKindId> {
        self.influence_kind_names.get(name).copied()
    }

    /// Look up the `LocusKindId` for a registered locus kind name.
    ///
    /// Returns `None` when the name has not been registered.
    pub fn locus_kind_id(&self, name: &str) -> Option<LocusKindId> {
        self.locus_kind_names.get(name).copied()
    }

    /// Mutable access to the `World` held by this builder.
    ///
    /// Use this during a **bootstrap phase** — after declaring kinds and
    /// influences but before calling `build()` — to:
    ///
    /// - Pre-create relationships whose `RelationshipId`s are needed by
    ///   programs registered via [`add_locus_kind`].
    /// - Set up initial subscriptions via
    ///   `world_mut().subscriptions_mut().subscribe_at(...)`.
    /// - Insert loci with specific `LocusId`s.
    ///
    /// [`add_locus_kind`]: SimulationBuilder::add_locus_kind
    ///
    /// ```ignore
    /// let mut builder = SimulationBuilder::new()
    ///     .locus_kind("SUPPLIER", SupplierProgram { factory: FACTORY })
    ///     .influence("supply", |c| c.with_decay(0.9));
    ///
    /// // Bootstrap: create the edge first, then register the program that needs its ID.
    /// let sup_rel = builder.world_mut().add_relationship(
    ///     Endpoints::directed(SUPPLIER, FACTORY), SUPPLY_KIND, state,
    /// );
    /// builder.world_mut().subscriptions_mut().subscribe_at(ANALYST, sup_rel, None);
    /// builder.add_locus_kind("ANALYST", AnalystProgram { sup_rel });
    ///
    /// let mut sim = builder.build();
    /// ```
    pub fn world_mut(&mut self) -> &mut World {
        &mut self.world
    }

    /// Register a locus kind by name — **mutable** variant for bootstrap use.
    ///
    /// Identical to [`locus_kind`] but takes `&mut self` so it can be called
    /// after `world_mut()` returns relationship IDs that the program needs.
    /// Returns the assigned `LocusKindId` so callers can immediately use it
    /// when inserting loci via `world_mut()`:
    ///
    /// ```ignore
    /// let rel_id = builder.world_mut().add_relationship(...);
    /// let kind = builder.add_locus_kind("OBSERVER", ObserverProgram { rel_id });
    /// builder.world_mut().insert_locus(Locus::new(OBSERVER_ID, kind, state));
    /// ```
    ///
    /// [`locus_kind`]: SimulationBuilder::locus_kind
    pub fn add_locus_kind(
        &mut self,
        name: impl Into<String>,
        program: impl LocusProgram + 'static,
    ) -> LocusKindId {
        let name = name.into();
        let id = LocusKindId(self.next_locus_kind);
        self.next_locus_kind += 1;
        self.loci_registry.insert(id, Box::new(program));
        self.locus_kind_names.insert(name, id);
        id
    }

    /// Register a locus kind with full config — **mutable** variant for bootstrap use.
    ///
    /// Returns the assigned `LocusKindId`. See [`add_locus_kind`] for the
    /// bootstrap pattern.
    ///
    /// [`add_locus_kind`]: SimulationBuilder::add_locus_kind
    pub fn add_locus_kind_with(
        &mut self,
        name: impl Into<String>,
        program: impl LocusProgram + 'static,
        configure: impl FnOnce(LocusKindBuilder) -> LocusKindBuilder,
    ) -> LocusKindId {
        let name = name.into();
        let id = LocusKindId(self.next_locus_kind);
        self.next_locus_kind += 1;
        let built = configure(LocusKindBuilder::default());
        self.loci_registry.insert_with_config(id, LocusKindConfig {
            name: Some(name.clone()),
            state_slots: built.state_slots,
            program: Box::new(program),
            refractory_batches: built.refractory_batches,
            encoder: built.encoder,
            max_proposals_per_dispatch: built.max_proposals_per_dispatch,
        });
        self.locus_kind_names.insert(name, id);
        id
    }

    /// Pre-register subscriptions before the first tick.
    ///
    /// Each `(subscriber, rel_id)` pair is subscribed via
    /// `SubscriptionStore::subscribe_at` with no batch tag.  Subscriptions
    /// registered this way appear in the initial state and are visible to
    /// programs from tick 1.
    ///
    /// For subscriptions that require a `RelationshipId` only known after
    /// the world is partially built, prefer calling
    /// `world_mut().subscriptions_mut().subscribe_at(...)` directly.
    ///
    /// ```ignore
    /// builder.initial_subscriptions(vec![(analyst, sup_a_rel), (analyst, sup_b_rel)])
    /// ```
    pub fn initial_subscriptions(mut self, subs: Vec<(LocusId, RelationshipId)>) -> Self {
        for (subscriber, rel_id) in subs {
            self.world.subscriptions_mut().subscribe_at(subscriber, rel_id, None);
        }
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
    state_slots: Vec<graph_core::StateSlotDef>,
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

    /// Declare the named state slots for loci of this kind.
    ///
    /// Slot `i` in `slots` corresponds to slot index `i` in the `StateVector`.
    /// These are advisory — the engine does not enforce clamping or validate
    /// state updates against these definitions.
    pub fn state_slots(mut self, slots: Vec<graph_core::StateSlotDef>) -> Self {
        self.state_slots = slots;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use graph_core::{Change, Endpoints, Locus, LocusContext, LocusId, ProposedChange, StateVector};

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

    /// Verifies that `world_mut()` + `add_locus_kind()` enable bootstrap patterns
    /// where a program needs a `RelationshipId` generated during world setup.
    #[test]
    fn world_mut_bootstrap_and_add_locus_kind() {
        // A simple observer program that records the relationship ID it watches.
        struct ObserverProgram { watched: RelationshipId }
        impl LocusProgram for ObserverProgram {
            fn process(&self, _: &Locus, _: &[&Change], _: &dyn LocusContext) -> Vec<ProposedChange> {
                Vec::new()
            }
        }

        const NODE_A: LocusId = LocusId(10);
        const NODE_B: LocusId = LocusId(11);
        const OBSERVER: LocusId = LocusId(12);

        let mut builder = SimulationBuilder::new()
            .locus_kind("NODE", NoopProgram)
            .influence("link", |cfg| cfg.with_decay(0.9));

        // Bootstrap: insert loci and a relationship, capture the RelationshipId.
        {
            let w = builder.world_mut();
            w.insert_locus(graph_core::Locus::new(NODE_A, LocusKindId(1), StateVector::zeros(1)));
            w.insert_locus(graph_core::Locus::new(NODE_B, LocusKindId(1), StateVector::zeros(1)));
            w.insert_locus(graph_core::Locus::new(OBSERVER, LocusKindId(2), StateVector::zeros(1)));
        }

        let rel_id = builder.world_mut().add_relationship(
            Endpoints::directed(NODE_A, NODE_B),
            InfluenceKindId(1),
            StateVector::zeros(2),
        );

        // Subscribe the observer to the relationship.
        builder.world_mut().subscriptions_mut().subscribe_at(OBSERVER, rel_id, None);

        // Register the observer program — only possible here because rel_id is now known.
        builder.add_locus_kind("OBSERVER", ObserverProgram { watched: rel_id });

        let sim = builder.build();

        // The subscription is already in place before the first tick.
        assert_eq!(sim.world.subscriptions().subscription_count(), 1);
        // The relationship exists.
        assert!(sim.world.relationships().get(rel_id).is_some());
    }

    /// `initial_subscriptions()` sets subscriptions in bulk before build.
    #[test]
    fn initial_subscriptions_fluent() {
        const NODE_A: LocusId = LocusId(1);
        const NODE_B: LocusId = LocusId(2);
        const WATCHER: LocusId = LocusId(3);

        // Pre-build a world with the relationship so we have a RelationshipId.
        let mut pre_world = World::new();
        pre_world.insert_locus(graph_core::Locus::new(NODE_A, LocusKindId(1), StateVector::zeros(1)));
        pre_world.insert_locus(graph_core::Locus::new(NODE_B, LocusKindId(1), StateVector::zeros(1)));
        pre_world.insert_locus(graph_core::Locus::new(WATCHER, LocusKindId(1), StateVector::zeros(1)));
        let rel_id = pre_world.add_relationship(
            Endpoints::directed(NODE_A, NODE_B),
            InfluenceKindId(1),
            StateVector::zeros(2),
        );

        let sim = SimulationBuilder::new()
            .locus_kind("NODE", NoopProgram)
            .influence("link", |cfg| cfg.with_decay(0.9))
            .world(pre_world)
            .initial_subscriptions(vec![(WATCHER, rel_id)])
            .build();

        assert_eq!(sim.world.subscriptions().subscription_count(), 1);
    }
}
