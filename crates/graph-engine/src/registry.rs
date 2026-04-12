//! Kind registries.
//!
//! Per `docs/redesign.md` §4, the engine owns two kind-keyed registries:
//!
//! - `LocusKindRegistry` maps `LocusKindId` → user-supplied
//!   `LocusProgram`. The batch loop dispatches a locus to its program by
//!   looking up the locus's kind here.
//! - `InfluenceKindRegistry` maps `InfluenceKindId` → per-kind config
//!   (decay, stabilization defaults, display name). The guard rail and
//!   regime classifier key off this — that's why per-kind classification
//!   is the resolved direction in §7.
//!
//! Both registries are populated at world-construction time and treated
//! as immutable for the duration of a run. `require()` always panics on
//! an unregistered kind — use `get()` for lenient lookups that tolerate
//! missing registrations.

use graph_core::{Encoder, InfluenceKindId, LocusKindId, LocusProgram, RelationshipSlotDef, StabilizationConfig, StateSlotDef, StateVector};
use rustc_hash::FxHashMap;

/// Type alias for the slot-definitions map passed into `BatchContext`.
///
/// Keys are influence kind ids; values are the extra-slot definitions for
/// relationships of that kind. Only kinds with at least one extra slot appear.
pub type SlotDefsMap = FxHashMap<InfluenceKindId, Vec<RelationshipSlotDef>>;

/// Hebbian plasticity parameters for one influence kind.
///
/// When `learning_rate > 0`, the engine applies the Hebbian rule at the
/// end of each batch for every relationship of this kind that was
/// touched during the batch:
///
/// ```text
/// Δweight = learning_rate * pre_signal * post_signal
/// weight  = clamp(weight + Δweight, 0, max_weight)
/// weight *= weight_decay   (end-of-batch)
/// ```
///
/// Default is fully disabled (`learning_rate = 0`) — plasticity is
/// opt-in.
#[derive(Debug, Clone, Copy)]
pub struct PlasticityConfig {
    /// Hebbian learning rate η. Must be >= 0. Set to 0 to disable.
    pub learning_rate: f32,
    /// Per-batch multiplicative decay on the weight. `1.0` = no decay.
    pub weight_decay: f32,
    /// Maximum weight value. Weights are clamped to `[0, max_weight]`.
    pub max_weight: f32,
}

impl Default for PlasticityConfig {
    fn default() -> Self {
        Self {
            learning_rate: 0.0,
            weight_decay: 1.0,
            max_weight: f32::MAX,
        }
    }
}

impl PlasticityConfig {
    /// True when plasticity is effectively enabled for this config.
    pub(crate) fn is_active(&self) -> bool {
        self.learning_rate > 0.0
    }
}

/// Per-influence-kind configuration held by `InfluenceKindRegistry`.
///
/// Two classes of tunable live here:
/// - **Decay**: per-batch multiplicative factor for relationship
///   activity (continuous decay from `docs/redesign.md` §3.5).
/// - **Stabilization**: guard-rail parameters (alpha, saturation,
///   trust region) applied when committing changes of this kind. This
///   is the per-kind port of `BasicStabilizer` from phase 1+2, now
///   opinionated about *which kind of influence* needs clamping rather
///   than clamping everything uniformly.
#[derive(Debug, Clone)]
pub struct InfluenceKindConfig {
    /// Human-readable label for diagnostics.
    pub name: String,
    /// Per-batch multiplicative decay on relationship activity (slot 0).
    /// `1.0` = no decay; smaller = fades faster.
    pub decay_per_batch: f32,
    /// Guard-rail parameters for state updates of this kind. Default:
    /// `StabilizationConfig::default()` (alpha=1.0, no saturation, no
    /// trust region — effectively transparent).
    pub stabilization: StabilizationConfig,
    /// Hebbian plasticity parameters. Disabled by default
    /// (`learning_rate = 0`).
    pub plasticity: PlasticityConfig,
    /// Relationships whose activity falls below this threshold after
    /// decay are automatically removed during `flush_relationship_decay`.
    /// `0.0` disables auto-pruning (default).
    pub prune_activity_threshold: f32,
    /// User-defined extra slots appended to the relationship `StateVector`
    /// beyond the built-in activity (slot 0) and weight (slot 1).
    ///
    /// Slots are initialised with their `default` on creation, decayed
    /// per-batch by their individual `decay` rate (if any), and ignored
    /// by the Hebbian plasticity rule.
    pub extra_slots: Vec<RelationshipSlotDef>,
    /// Signed contribution to relationship activity on each touch.
    ///
    /// Added to `state[0]` (the activity slot) every time the engine
    /// observes this influence kind flowing through a relationship.
    ///
    /// - `+1.0` (default) — excitatory: activity grows with each touch.
    /// - `-1.0` — inhibitory: activity decreases with each touch (e.g.,
    ///   an antagonistic or suppressive influence).
    /// - `0.0` — neutral: `change_count` is still incremented but the
    ///   activity level is unaffected.
    ///
    /// The initial activity of a newly auto-emerged relationship is set to
    /// this value (the first touch), so inhibitory relationships start
    /// negative immediately.
    pub activity_contribution: f32,
    /// When `true`, auto-emerged relationships for this kind use
    /// `Endpoints::Symmetric` instead of `Endpoints::Directed`.
    ///
    /// Useful for inherently undirected influences (co-occurrence, mutual
    /// conflict, shared resonance) where A→B and B→A represent the same
    /// coupling. Default: `false` (directed).
    pub symmetric: bool,
    /// Optional type-level constraint: which `(source_kind, target_kind)` pairs
    /// are valid for relationships of this influence kind.
    ///
    /// When non-empty, the engine emits a `WorldEvent::SchemaViolation` (soft
    /// warning, non-blocking) whenever a relationship auto-emerges between loci
    /// whose kinds are not listed here. Empty = no constraint (default).
    pub applies_between: Vec<(LocusKindId, LocusKindId)>,
}

impl InfluenceKindConfig {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            decay_per_batch: 1.0,
            stabilization: StabilizationConfig::default(),
            plasticity: PlasticityConfig::default(),
            prune_activity_threshold: 0.0,
            extra_slots: Vec::new(),
            activity_contribution: 1.0,
            symmetric: false,
            applies_between: Vec::new(),
        }
    }

    pub fn with_decay(mut self, decay_per_batch: f32) -> Self {
        self.decay_per_batch = decay_per_batch;
        self
    }

    pub fn with_stabilization(mut self, config: StabilizationConfig) -> Self {
        self.stabilization = config;
        self
    }

    pub fn with_plasticity(mut self, config: PlasticityConfig) -> Self {
        self.plasticity = config;
        self
    }

    /// Enable Hebbian plasticity with a simple learning rate.
    ///
    /// Shorthand for `with_plasticity(PlasticityConfig { learning_rate: rate,
    /// weight_decay: 0.99, max_weight: 1.0 })`. Use `with_plasticity` directly
    /// when you need non-default `weight_decay` or `max_weight`.
    pub fn with_learning_rate(mut self, rate: f32) -> Self {
        self.plasticity = PlasticityConfig {
            learning_rate: rate,
            weight_decay: 0.99,
            max_weight: 1.0,
        };
        self
    }

    pub fn with_prune_threshold(mut self, threshold: f32) -> Self {
        self.prune_activity_threshold = threshold;
        self
    }

    /// Set the signed activity contribution per touch.
    ///
    /// Positive = excitatory (+1.0 default), negative = inhibitory,
    /// zero = neutral. Must be finite (not NaN or ±inf).
    pub fn with_activity_contribution(mut self, contribution: f32) -> Self {
        self.activity_contribution = contribution;
        self
    }

    pub fn with_extra_slots(mut self, slots: Vec<RelationshipSlotDef>) -> Self {
        self.extra_slots = slots;
        self
    }

    /// Mark this kind as symmetric: auto-emerged edges use
    /// `Endpoints::Symmetric` so A↔B co-occurrence produces one edge
    /// instead of two directed ones.
    pub fn symmetric(mut self) -> Self {
        self.symmetric = true;
        self
    }

    /// Restrict this kind to a specific `(source_kind, target_kind)` pair.
    ///
    /// May be called multiple times to allow multiple valid endpoint-kind
    /// combinations. When any `applies_between` entries are present, the
    /// engine emits a `WorldEvent::SchemaViolation` for edges that fall
    /// outside the declared pairs (non-blocking).
    pub fn with_applies_between(mut self, from_kind: LocusKindId, to_kind: LocusKindId) -> Self {
        self.applies_between.push((from_kind, to_kind));
        self
    }

    /// Check whether a `(from_kind, to_kind)` pair satisfies this kind's
    /// `applies_between` constraint.
    ///
    /// Returns `true` when `applies_between` is empty (no constraint) or
    /// when the pair appears in the list. Returns `false` only when at
    /// least one constraint is declared and the pair is not listed.
    pub fn allows_endpoint_kinds(&self, from_kind: LocusKindId, to_kind: LocusKindId) -> bool {
        if self.applies_between.is_empty() {
            return true;
        }
        self.applies_between.contains(&(from_kind, to_kind))
    }

    /// Build the initial `StateVector` for a new relationship of this kind.
    ///
    /// Activity starts at `activity_contribution` (one touch), weight at
    /// 0.0, then the default values for each extra slot in declaration
    /// order. For inhibitory kinds (`activity_contribution < 0`) the
    /// relationship's activity is negative from birth.
    pub fn initial_relationship_state(&self) -> StateVector {
        let mut values = vec![self.activity_contribution, 0.0f32];
        for slot in &self.extra_slots {
            values.push(slot.default);
        }
        StateVector::from_slice(&values)
    }

    /// Return the `StateVector` index of the named extra slot, or `None` if
    /// the name is not registered for this kind.
    ///
    /// The built-in slots occupy indices 0 (activity) and 1 (weight). Extra
    /// slots begin at index 2 in declaration order.
    pub fn slot_index(&self, name: &str) -> Option<usize> {
        self.extra_slots
            .iter()
            .position(|s| s.name == name)
            .map(|pos| pos + 2)
    }

    /// Read a named slot value from a relationship `StateVector`.
    ///
    /// Returns `None` when the slot name is not registered or the vector is
    /// too short (should not happen for vectors produced by this registry).
    pub fn read_slot(&self, state: &StateVector, name: &str) -> Option<f32> {
        let idx = self.slot_index(name)?;
        state.as_slice().get(idx).copied()
    }
}

/// Per-locus-kind configuration: program + optional refractory period + optional encoder.
pub struct LocusKindConfig {
    /// Human-readable label for this locus kind (e.g. `"Person"`, `"Organization"`).
    ///
    /// Used by name-based kind lookups (`Simulation::locus_kind_by_name`) and
    /// diagnostic output. `None` when registered via `insert()` without a name.
    pub name: Option<String>,
    /// Metadata describing each slot in the locus `StateVector`.
    ///
    /// Slot `i` in `state_slots` corresponds to slot `i` in the `StateVector`.
    /// Empty by default — the engine does not require slot definitions to operate.
    pub state_slots: Vec<StateSlotDef>,
    pub program: Box<dyn LocusProgram>,
    /// Minimum number of batches a locus must wait after firing before
    /// its program is dispatched again. `0` = no refractory period.
    /// This prevents cascade amplification in highly connected networks
    /// by ensuring each locus fires at most once per `refractory_batches`
    /// batches within a single tick.
    pub refractory_batches: u32,
    /// Optional encoder that converts domain `Properties` into the
    /// `StateVector` the engine consumes. Used by the ingest API.
    /// When `None`, the ingest API falls back to `PassthroughEncoder`.
    pub encoder: Option<Box<dyn Encoder>>,
    /// Maximum number of `ProposedChange`s a single dispatch may produce.
    ///
    /// When `Some(n)`, the program's output is silently truncated to the
    /// first `n` proposals after `process` returns. This caps runaway
    /// programs without aborting the tick. `None` means unlimited (default).
    pub max_proposals_per_dispatch: Option<usize>,
}

/// Owns the per-locus-kind program implementations.
#[derive(Default)]
pub struct LocusKindRegistry {
    configs: FxHashMap<LocusKindId, LocusKindConfig>,
}

impl LocusKindRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a program for a locus kind with no refractory period.
    pub fn insert(&mut self, kind: LocusKindId, program: Box<dyn LocusProgram>) {
        self.insert_with_config(kind, LocusKindConfig {
            name: None,
            state_slots: Vec::new(),
            program,
            refractory_batches: 0,
            encoder: None,
            max_proposals_per_dispatch: None,
        });
    }

    /// Register a program for a locus kind with a human-readable name.
    ///
    /// The name is used by `Simulation::locus_kind_by_name` for string-based
    /// kind lookups and in diagnostic output. Must be unique within the registry.
    pub fn insert_named(&mut self, kind: LocusKindId, name: impl Into<String>, program: Box<dyn LocusProgram>) {
        self.insert_with_config(kind, LocusKindConfig {
            name: Some(name.into()),
            state_slots: Vec::new(),
            program,
            refractory_batches: 0,
            encoder: None,
            max_proposals_per_dispatch: None,
        });
    }

    /// Register a program with a full config (refractory period, etc.).
    pub fn insert_with_config(&mut self, kind: LocusKindId, config: LocusKindConfig) {
        if self.configs.insert(kind, config).is_some() {
            panic!("LocusKindRegistry: duplicate registration for {kind:?}");
        }
    }

    pub fn get(&self, kind: LocusKindId) -> Option<&dyn LocusProgram> {
        self.configs.get(&kind).map(|cfg| cfg.program.as_ref())
    }

    /// Return the encoder for a kind, or `None` if no encoder is registered.
    pub fn encoder(&self, kind: LocusKindId) -> Option<&dyn Encoder> {
        self.configs
            .get(&kind)
            .and_then(|cfg| cfg.encoder.as_deref())
    }

    /// Return the full config for a kind (program + refractory period).
    pub fn get_config(&self, kind: LocusKindId) -> Option<&LocusKindConfig> {
        self.configs.get(&kind)
    }

    /// Same as `get` but panics (in both debug and release) when the kind
    /// is missing. Use this in the batch loop where an unregistered kind
    /// is always a programming error.
    pub fn require(&self, kind: LocusKindId) -> Option<&dyn LocusProgram> {
        let found = self.get(kind);
        assert!(found.is_some(), "unregistered LocusKindId: {kind:?}");
        found
    }

    /// Look up a `LocusKindId` by its registered name.
    ///
    /// Returns `None` if no kind with that name was registered via
    /// `insert_named()` or `insert_with_config()` with `name: Some(...)`.
    pub fn kind_by_name(&self, name: &str) -> Option<LocusKindId> {
        self.configs
            .iter()
            .find(|(_, cfg)| cfg.name.as_deref() == Some(name))
            .map(|(&id, _)| id)
    }

    /// Return all registered kinds with their names (only named kinds appear).
    pub fn named_kinds(&self) -> impl Iterator<Item = (LocusKindId, &str)> {
        self.configs.iter()
            .filter_map(|(&id, cfg)| cfg.name.as_deref().map(|n| (id, n)))
    }

    pub fn len(&self) -> usize {
        self.configs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.configs.is_empty()
    }
}

/// Owns the per-influence-kind config used by the guard rail, regime
/// classifier, and relationship layer.
#[derive(Debug, Default, Clone)]
pub struct InfluenceKindRegistry {
    configs: FxHashMap<InfluenceKindId, InfluenceKindConfig>,
    /// Pre-built map of extra slot definitions. Rebuilt whenever a new
    /// kind is inserted. Accessed by the engine to borrow into BatchContext
    /// without per-batch allocation.
    slot_defs: SlotDefsMap,
}

impl InfluenceKindRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a config for an influence kind.
    ///
    /// # Panics
    ///
    /// Panics if `kind` was already registered (duplicate registration) or
    /// if `config` contains out-of-range values:
    ///
    /// - `decay_per_batch` must be in `(0.0, 1.0]` — `0.0` kills all
    ///   relationships in one batch; values > 1.0 cause unbounded growth.
    /// - `plasticity.learning_rate` must be `>= 0.0`.
    /// - `plasticity.weight_decay` must be in `(0.0, 1.0]`.
    /// - `plasticity.max_weight` must be `> 0.0`.
    /// - `prune_activity_threshold` must be `>= 0.0`.
    pub fn insert(&mut self, kind: InfluenceKindId, config: InfluenceKindConfig) {
        assert!(
            config.decay_per_batch > 0.0 && config.decay_per_batch <= 1.0,
            "InfluenceKindConfig '{}': decay_per_batch must be in (0.0, 1.0], got {}",
            config.name, config.decay_per_batch
        );
        assert!(
            config.plasticity.learning_rate >= 0.0,
            "InfluenceKindConfig '{}': plasticity.learning_rate must be >= 0, got {}",
            config.name, config.plasticity.learning_rate
        );
        assert!(
            config.plasticity.weight_decay > 0.0 && config.plasticity.weight_decay <= 1.0,
            "InfluenceKindConfig '{}': plasticity.weight_decay must be in (0.0, 1.0], got {}",
            config.name, config.plasticity.weight_decay
        );
        assert!(
            config.plasticity.max_weight > 0.0,
            "InfluenceKindConfig '{}': plasticity.max_weight must be > 0, got {}",
            config.name, config.plasticity.max_weight
        );
        assert!(
            config.prune_activity_threshold >= 0.0,
            "InfluenceKindConfig '{}': prune_activity_threshold must be >= 0, got {}",
            config.name, config.prune_activity_threshold
        );
        assert!(
            config.activity_contribution.is_finite(),
            "InfluenceKindConfig '{}': activity_contribution must be finite, got {}",
            config.name, config.activity_contribution
        );
        if self.configs.insert(kind, config).is_some() {
            panic!("InfluenceKindRegistry: duplicate registration for {kind:?}");
        }
        self.rebuild_slot_defs();
    }

    fn rebuild_slot_defs(&mut self) {
        self.slot_defs = self.configs
            .iter()
            .filter(|(_, cfg)| !cfg.extra_slots.is_empty())
            .map(|(&k, cfg)| (k, cfg.extra_slots.clone()))
            .collect();
    }

    /// Borrow the pre-built slot-definitions map.
    ///
    /// Pass this into `BatchContext::new` once per tick; the engine holds the
    /// borrow for the duration of a tick to avoid per-batch allocation.
    pub fn slot_defs(&self) -> &SlotDefsMap {
        &self.slot_defs
    }

    pub fn get(&self, kind: InfluenceKindId) -> Option<&InfluenceKindConfig> {
        self.configs.get(&kind)
    }

    /// Build the initial `StateVector` for a new relationship of `kind`.
    ///
    /// Returns the kind-config's `initial_relationship_state()` when registered,
    /// or a minimal `[1.0, 0.0]` (activity=1, weight=0) when the kind is unknown.
    /// Use this in world-construction code that creates relationships before the
    /// first tick so the initial state matches the kind's extra-slot defaults.
    pub fn initial_state_for(&self, kind: InfluenceKindId) -> StateVector {
        self.configs
            .get(&kind)
            .map(|cfg| cfg.initial_relationship_state())
            .unwrap_or_else(|| {
                debug_assert!(
                    false,
                    "initial_state_for called with unregistered InfluenceKindId: {kind:?}; \
                     extra-slot defaults will be missing from the returned state"
                );
                StateVector::from_slice(&[1.0, 0.0])
            })
    }

    pub fn require(&self, kind: InfluenceKindId) -> Option<&InfluenceKindConfig> {
        let found = self.get(kind);
        assert!(found.is_some(), "unregistered InfluenceKindId: {kind:?}");
        found
    }

    pub fn get_mut(&mut self, kind: InfluenceKindId) -> Option<&mut InfluenceKindConfig> {
        self.configs.get_mut(&kind)
    }

    pub fn kinds(&self) -> impl Iterator<Item = InfluenceKindId> + '_ {
        self.configs.keys().copied()
    }

    /// Return the `StateVector` index of a named extra slot for `kind`, or
    /// `None` if the kind is not registered or the slot name is not declared.
    ///
    /// Built-in slot 0 = activity, slot 1 = weight. Extra slots start at 2.
    pub fn slot_index(&self, kind: InfluenceKindId, name: &str) -> Option<usize> {
        self.get(kind)?.slot_index(name)
    }

    /// Read a named slot value from `rel`'s `StateVector` for `rel.kind`.
    ///
    /// Returns `None` if the kind is not registered, the slot name is unknown,
    /// or the state vector is too short.
    pub fn read_slot(&self, kind: InfluenceKindId, rel: &graph_core::Relationship, name: &str) -> Option<f32> {
        self.get(kind)?.read_slot(&rel.state, name)
    }

    pub fn len(&self) -> usize {
        self.configs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.configs.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use graph_core::{Change, Locus, ProposedChange};

    struct NoopProgram;
    impl LocusProgram for NoopProgram {
        fn process(&self, _: &Locus, _: &[&Change], _: &dyn graph_core::LocusContext) -> Vec<ProposedChange> {
            Vec::new()
        }
    }

    #[test]
    fn locus_registry_round_trips() {
        let mut reg = LocusKindRegistry::new();
        reg.insert(LocusKindId(1), Box::new(NoopProgram));
        assert_eq!(reg.len(), 1);
        assert!(reg.get(LocusKindId(1)).is_some());
        assert!(reg.get(LocusKindId(2)).is_none());
    }

    #[test]
    #[should_panic(expected = "duplicate registration")]
    fn duplicate_locus_registration_panics() {
        let mut reg = LocusKindRegistry::new();
        reg.insert(LocusKindId(1), Box::new(NoopProgram));
        reg.insert(LocusKindId(1), Box::new(NoopProgram));
    }

    #[test]
    fn influence_registry_holds_decay_default() {
        let mut reg = InfluenceKindRegistry::new();
        reg.insert(
            InfluenceKindId(7),
            InfluenceKindConfig::new("thermal").with_decay(0.9),
        );
        let cfg = reg.get(InfluenceKindId(7)).unwrap();
        assert_eq!(cfg.name, "thermal");
        assert!((cfg.decay_per_batch - 0.9).abs() < 1e-6);
    }

    #[test]
    #[should_panic(expected = "duplicate registration")]
    fn duplicate_influence_registration_panics() {
        let mut reg = InfluenceKindRegistry::new();
        reg.insert(InfluenceKindId(1), InfluenceKindConfig::new("a"));
        reg.insert(InfluenceKindId(1), InfluenceKindConfig::new("b"));
    }

    #[test]
    fn slot_index_returns_correct_offset() {
        use graph_core::RelationshipSlotDef;
        let cfg = InfluenceKindConfig::new("test").with_extra_slots(vec![
            RelationshipSlotDef::new("tension", 0.0),
            RelationshipSlotDef::new("trust", 1.0),
        ]);
        // Built-in slots: 0=activity, 1=weight. Extra start at 2.
        assert_eq!(cfg.slot_index("tension"), Some(2));
        assert_eq!(cfg.slot_index("trust"), Some(3));
        assert_eq!(cfg.slot_index("unknown"), None);
    }

    #[test]
    fn read_slot_returns_value_from_state_vector() {
        use graph_core::{RelationshipSlotDef, StateVector};
        let cfg = InfluenceKindConfig::new("test").with_extra_slots(vec![
            RelationshipSlotDef::new("tension", 0.0),
            RelationshipSlotDef::new("trust", 1.0),
        ]);
        let state = StateVector::from_slice(&[1.0, 0.5, 0.3, 0.8]);
        assert!((cfg.read_slot(&state, "tension").unwrap() - 0.3).abs() < 1e-6);
        assert!((cfg.read_slot(&state, "trust").unwrap() - 0.8).abs() < 1e-6);
        assert!(cfg.read_slot(&state, "missing").is_none());
    }

    #[test]
    fn slot_defs_cache_rebuilt_on_insert() {
        use graph_core::RelationshipSlotDef;
        let mut reg = InfluenceKindRegistry::new();
        assert!(reg.slot_defs().is_empty());

        reg.insert(
            InfluenceKindId(1),
            InfluenceKindConfig::new("a").with_extra_slots(vec![
                RelationshipSlotDef::new("x", 0.0),
            ]),
        );
        assert_eq!(reg.slot_defs().len(), 1);
        assert!(reg.slot_defs().contains_key(&InfluenceKindId(1)));
    }

    // ── InfluenceKindConfig validation ───────────────────────────────────────

    #[test]
    fn valid_config_inserts_without_panic() {
        let mut reg = InfluenceKindRegistry::new();
        reg.insert(InfluenceKindId(1), InfluenceKindConfig::new("ok").with_decay(0.95));
    }

    #[test]
    #[should_panic(expected = "decay_per_batch must be in (0.0, 1.0]")]
    fn zero_decay_panics() {
        let mut reg = InfluenceKindRegistry::new();
        reg.insert(InfluenceKindId(1), InfluenceKindConfig::new("bad").with_decay(0.0));
    }

    #[test]
    #[should_panic(expected = "decay_per_batch must be in (0.0, 1.0]")]
    fn decay_above_one_panics() {
        let mut reg = InfluenceKindRegistry::new();
        reg.insert(InfluenceKindId(1), InfluenceKindConfig::new("bad").with_decay(1.1));
    }

    #[test]
    #[should_panic(expected = "plasticity.learning_rate must be >= 0")]
    fn negative_learning_rate_panics() {
        let mut reg = InfluenceKindRegistry::new();
        reg.insert(
            InfluenceKindId(1),
            InfluenceKindConfig::new("bad").with_plasticity(PlasticityConfig {
                learning_rate: -0.1,
                weight_decay: 1.0,
                max_weight: 1.0,
            }),
        );
    }

    #[test]
    #[should_panic(expected = "plasticity.max_weight must be > 0")]
    fn zero_max_weight_panics() {
        let mut reg = InfluenceKindRegistry::new();
        reg.insert(
            InfluenceKindId(1),
            InfluenceKindConfig::new("bad").with_plasticity(PlasticityConfig {
                learning_rate: 0.0,
                weight_decay: 1.0,
                max_weight: 0.0,
            }),
        );
    }

    #[test]
    #[should_panic(expected = "prune_activity_threshold must be >= 0")]
    fn negative_prune_threshold_panics() {
        let mut reg = InfluenceKindRegistry::new();
        reg.insert(
            InfluenceKindId(1),
            InfluenceKindConfig::new("bad").with_prune_threshold(-0.1),
        );
    }

    #[test]
    fn with_learning_rate_sets_plasticity_defaults() {
        let mut reg = InfluenceKindRegistry::new();
        reg.insert(
            InfluenceKindId(1),
            InfluenceKindConfig::new("ok").with_learning_rate(0.05),
        );
        let cfg = reg.require(InfluenceKindId(1)).unwrap();
        assert!((cfg.plasticity.learning_rate - 0.05).abs() < 1e-7);
        assert!((cfg.plasticity.weight_decay - 0.99).abs() < 1e-7);
        assert!((cfg.plasticity.max_weight - 1.0).abs() < 1e-7);
    }
}
