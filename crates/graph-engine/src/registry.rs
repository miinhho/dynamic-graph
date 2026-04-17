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

use graph_core::{Encoder, InfluenceKindId, InteractionEffect, LocusKindId, LocusProgram, RelationshipSlotDef, StabilizationConfig, StateSlotDef, StateVector};
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
    /// LTD (Long-Term Depression) rate for anti-causal STDP (PostFirst).
    /// When > 0, overrides `learning_rate` for weight-decrease updates.
    /// `0.0` (default) means use `learning_rate` for both LTP and LTD —
    /// symmetric rates. Set higher than `learning_rate` to make suppression
    /// faster than potentiation (common in biological STDP).
    pub ltd_rate: f32,
    /// When true, use STDP rule with engine-native causal DAG timing:
    ///   - PreFirst (pre caused post): Δw = +η × pre × post  (LTP)
    ///   - PostFirst (feedback loop detected): Δw = -ltd_rate × pre × post  (LTD)
    ///   - Simultaneous (same batch): Δw = +η × pre × post  (standard Hebbian)
    /// Feedback is detected via bounded multi-hop DAG traversal in the
    /// compute phase — no extra cost in the apply phase.
    /// When false (default), use standard symmetric Hebbian.
    pub stdp: bool,
    /// When `Some(τ)`, use BCM (Bienenstock-Cooper-Munro) rule instead of plain Hebbian.
    ///   `Δw = η × pre × post × (post − θ_M)`
    /// where θ_M is a per-locus sliding threshold stored in `World::bcm_thresholds`.
    /// θ_M adapts each batch: `θ_M += (post² − θ_M) / τ`.
    /// When `post > θ_M` → LTP (weight up); when `post < θ_M` → LTD (weight down).
    /// Mutually exclusive with `stdp` — `with_bcm` clears `stdp`.
    /// `None` (default) disables BCM.
    pub bcm_tau: Option<f32>,
}

impl Default for PlasticityConfig {
    fn default() -> Self {
        Self {
            learning_rate: 0.0,
            ltd_rate: 0.0,
            weight_decay: 1.0,
            max_weight: f32::MAX,
            stdp: false,
            bcm_tau: None,
        }
    }
}

impl PlasticityConfig {
    /// True when plasticity is effectively enabled for this config.
    pub(crate) fn is_active(&self) -> bool {
        self.learning_rate > 0.0 || self.bcm_tau.is_some()
    }

    /// Enable STDP (Spike-Timing Dependent Plasticity).
    ///
    /// When enabled:
    /// - causal flow (pre before post): weight increases (`+η × pre × post`)
    /// - anti-causal flow (post before pre): weight decreases (`-ltd_rate × pre × post`)
    /// - simultaneous (same batch): same as standard Hebbian
    ///
    /// Clears `bcm_tau` (mutually exclusive with BCM).
    pub fn with_stdp(mut self) -> Self {
        self.stdp = true;
        self.bcm_tau = None;
        self
    }

    /// Set a separate LTD rate for anti-causal (PostFirst) STDP weight decreases.
    ///
    /// When `0.0` (default), `learning_rate` is used for both LTP and LTD.
    /// Setting `ltd_rate > learning_rate` makes suppression faster than
    /// potentiation — causes the engine to more aggressively prune
    /// feedback paths relative to how quickly it strengthens causal ones.
    pub fn with_ltd_rate(mut self, rate: f32) -> Self {
        self.ltd_rate = rate.max(0.0);
        self
    }

    /// Enable BCM (Bienenstock-Cooper-Munro) plasticity rule.
    ///
    /// `Δw = η × pre × post × (post − θ_M)` where θ_M is a per-locus sliding
    /// threshold that adapts as `θ_M += (post² − θ_M) / tau`.
    ///
    /// Clears `stdp` (mutually exclusive).
    pub fn with_bcm(mut self, tau: f32) -> Self {
        self.bcm_tau = Some(tau);
        self.stdp = false;
        self
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
    /// Relationships whose Hebbian/BCM weight falls at or below this threshold
    /// after a plasticity update are automatically deleted at the end of that
    /// batch. Subscribers receive a tombstone notification in the next batch,
    /// identical to an explicit `DeleteRelationship` proposal.
    ///
    /// `0.0` disables weight-based pruning (default). A typical value is `0.0`
    /// (prune when fully suppressed) or a small positive value like `0.05`
    /// (prune when nearly suppressed). Only meaningful when plasticity is active
    /// (`learning_rate > 0`).
    pub prune_weight_threshold: f32,
    /// User-defined extra slots appended to the relationship `StateVector`
    /// beyond the built-in activity (slot 0) and weight (slot 1).
    ///
    /// Slots are initialised with their `default` on creation, decayed
    /// per-batch by their individual `decay` rate (if any), and ignored
    /// by the Hebbian plasticity rule.
    pub extra_slots: Vec<RelationshipSlotDef>,
    /// Scale factor applied to the pre-synaptic signal when updating
    /// relationship activity.
    ///
    /// On each auto-emergence touch, the engine increments the activity slot
    /// by `activity_contribution × |pre_signal|`, where `pre_signal` is the
    /// first slot of the predecessor change's after-state (the signal that
    /// actually flowed through the relationship).
    ///
    /// This coupling ties the activity scale to the simulation's signal
    /// domain, so thresholds can be expressed in the same units as program
    /// outputs without independent scale calibration.
    ///
    /// - `+1.0` (default) — activity tracks signal directly.
    /// - `+2.0` — amplified: large signals accumulate activity faster.
    /// - `-1.0` — inhibitory: strong signals push activity negative.
    /// - `0.0` — neutral: signal is observed but activity is unaffected.
    ///
    /// **Steady-state** (regular stimulation, signal magnitude `s`):
    /// `activity_contribution × s / (1 − decay_per_batch)`.
    ///
    /// **Note**: `StructuralProposal::CreateRelationship` touches use
    /// `activity_contribution` directly (no signal context), treating the
    /// explicit declaration as a full-strength touch.
    pub activity_contribution: f32,
    /// Optional parent kind in the influence-kind hierarchy.
    ///
    /// When set, this kind is treated as a specialisation of the parent.
    /// `InfluenceKindRegistry::ancestors_of(kind)` walks the parent chain;
    /// `is_subkind_of(child, ancestor)` tests membership.
    ///
    /// The parent **must already be registered** at the time `insert()` is
    /// called. This constraint makes cycles structurally impossible:
    /// the new kind cannot yet appear in any ancestor chain.
    pub parent: Option<InfluenceKindId>,
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
    /// Minimum cross-locus signal magnitude required to auto-emerge a new
    /// relationship of this kind.
    ///
    /// When a predecessor's signal (`|after[0]|`) is below this threshold
    /// the engine skips relationship creation for that predecessor.  An
    /// **already-existing** relationship is always updated regardless of
    /// this threshold — only the initial creation is gated.
    ///
    /// `0.0` (default) — every cross-locus causal flow creates a relationship.
    pub min_emerge_activity: f32,
    /// Optional hard cap on the relationship activity slot.
    ///
    /// When `Some(cap)`, the activity slot is clamped to `[-cap, cap]` after
    /// every touch (both creation and update). This prevents unbounded
    /// accumulation in high-frequency or long-running simulations where
    /// `decay_per_batch` is less than 1.0 but stimulation keeps arriving.
    ///
    /// The steady-state activity without a cap is `activity_contribution /
    /// (1 − decay_per_batch)`, which can be large for slow-decay kinds.
    /// Setting `max_activity` to a meaningful upper bound keeps values in a
    /// human-readable range without changing the decay dynamics.
    ///
    /// `None` (default) — no cap applied.
    pub max_activity: Option<f32>,
    /// Automatic relationship demotion policy (E3).
    ///
    /// When set, the engine evaluates this policy at the end of each tick
    /// (after decay) and evicts matching relationships to cold storage.
    /// Evicted relationships remain in the persistent storage backend and
    /// are promoted back on demand via `Simulation::promote_relationship`.
    ///
    /// `None` (default) — no automatic demotion.
    pub demotion_policy: Option<DemotionPolicy>,
}

/// Condition under which a relationship is automatically demoted to cold storage (E3).
///
/// Evaluated after per-batch decay. A relationship is demoted when it satisfies
/// the condition AND has been idle for at least `min_idle_batches` since it was
/// last touched by the engine.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum DemotionPolicy {
    /// Demote when `activity < threshold`.
    ActivityFloor(f32),
    /// Demote when the relationship has not been touched for `N` batches.
    IdleBatches(u64),
    /// Demote when the in-memory relationship count for this kind exceeds
    /// `capacity`, keeping the `capacity` most recently touched.
    LruCapacity(usize),
}

impl InfluenceKindConfig {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            decay_per_batch: 1.0,
            stabilization: StabilizationConfig::default(),
            plasticity: PlasticityConfig::default(),
            prune_activity_threshold: 0.0,
            prune_weight_threshold: 0.0,
            extra_slots: Vec::new(),
            activity_contribution: 1.0,
            parent: None,
            symmetric: false,
            applies_between: Vec::new(),
            min_emerge_activity: 0.0,
            max_activity: None,
            demotion_policy: None,
        }
    }

    pub fn with_decay(mut self, decay_per_batch: f32) -> Self {
        self.decay_per_batch = decay_per_batch;
        self
    }

    /// Mark relationships of this kind as symmetric (undirected).
    ///
    /// When `true`, auto-emerged relationships use `Endpoints::Symmetric`
    /// and the lookup key matches pre-inserted symmetric relationships.
    /// Set this when pre-inserting `Endpoints::Symmetric` relationships so that
    /// Hebbian plasticity can find and update them via the emergence path.
    pub fn with_symmetric(mut self, symmetric: bool) -> Self {
        self.symmetric = symmetric;
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
            ..Default::default()
        };
        self
    }

    pub fn with_prune_threshold(mut self, threshold: f32) -> Self {
        self.prune_activity_threshold = threshold;
        self
    }

    /// Delete relationships whose Hebbian/BCM weight falls at or below
    /// `threshold` after a plasticity update.
    ///
    /// Subscribers receive a tombstone notification in the next batch.
    /// Only meaningful when plasticity is active (`learning_rate > 0`).
    /// Typical values: `0.0` (prune when fully suppressed) or `0.05`
    /// (prune when nearly suppressed).
    pub fn with_prune_weight_threshold(mut self, threshold: f32) -> Self {
        self.prune_weight_threshold = threshold.max(0.0);
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

    /// Declare this kind as a child of `parent` in the influence-kind hierarchy.
    ///
    /// The parent must already be registered before calling `InfluenceKindRegistry::insert`.
    /// Cycles are impossible by construction: the new kind is not yet in the registry
    /// when `insert` validates the parent link.
    pub fn with_parent(mut self, parent: InfluenceKindId) -> Self {
        self.parent = Some(parent);
        self
    }

    pub fn with_extra_slots(mut self, slots: Vec<RelationshipSlotDef>) -> Self {
        self.extra_slots = slots;
        self
    }

    /// Minimum signal magnitude required to auto-emerge a new relationship.
    ///
    /// Only gates relationship *creation*; existing relationships are updated
    /// regardless of the threshold.
    pub fn with_min_emerge_activity(mut self, threshold: f32) -> Self {
        self.min_emerge_activity = threshold;
        self
    }

    /// Cap the activity slot at `±cap` after each touch.
    ///
    /// Prevents unbounded accumulation in high-frequency simulations.
    /// The cap is applied to the absolute value: `activity.clamp(-cap, cap)`.
    pub fn with_max_activity(mut self, cap: f32) -> Self {
        self.max_activity = Some(cap);
        self
    }

    /// Enable automatic relationship demotion for this kind (E3).
    ///
    /// When set, the engine evaluates `policy` at the end of each tick (after
    /// decay) and evicts qualifying relationships to cold storage. Demoted
    /// relationships can be promoted back on demand.
    ///
    /// See [`DemotionPolicy`] for available policies.
    pub fn with_demotion(mut self, policy: DemotionPolicy) -> Self {
        self.demotion_policy = Some(policy);
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
    /// Cross-kind interaction effects registered via `register_interaction`.
    /// Keyed by `(kind_a, kind_b)` in canonical (min, max) order so lookup
    /// is symmetric: `interaction_between(A, B)` == `interaction_between(B, A)`.
    interactions: FxHashMap<(InfluenceKindId, InfluenceKindId), InteractionEffect>,
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
            config.plasticity.ltd_rate >= 0.0,
            "InfluenceKindConfig '{}': plasticity.ltd_rate must be >= 0, got {}",
            config.name, config.plasticity.ltd_rate
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
            config.prune_weight_threshold >= 0.0,
            "InfluenceKindConfig '{}': prune_weight_threshold must be >= 0, got {}",
            config.name, config.prune_weight_threshold
        );
        assert!(
            config.activity_contribution.is_finite(),
            "InfluenceKindConfig '{}': activity_contribution must be finite, got {}",
            config.name, config.activity_contribution
        );
        if let Some(cap) = config.max_activity {
            assert!(
                cap > 0.0 && cap.is_finite(),
                "InfluenceKindConfig '{}': max_activity must be finite and > 0, got {}",
                config.name, cap
            );
        }
        if let Some(parent) = config.parent {
            assert!(
                self.configs.contains_key(&parent),
                "InfluenceKindConfig '{}': parent {parent:?} must be registered before inserting child",
                config.name
            );
        }
        if self.configs.insert(kind, config).is_some() {
            panic!("InfluenceKindRegistry: duplicate registration for {kind:?}");
        }
        self.rebuild_slot_defs();
    }

    // ─── Kind hierarchy ────────────────────────────────────────────────────────

    /// Walk the parent chain of `kind` and return every ancestor, nearest first.
    ///
    /// Returns an empty `Vec` when `kind` is a root (has no parent) or is
    /// not registered. Does not include `kind` itself.
    pub fn ancestors_of(&self, kind: InfluenceKindId) -> Vec<InfluenceKindId> {
        let mut result = Vec::new();
        let mut current = kind;
        while let Some(cfg) = self.configs.get(&current) {
            if let Some(parent) = cfg.parent {
                result.push(parent);
                current = parent;
            } else {
                break;
            }
        }
        result
    }

    /// Return `true` when `child` has `ancestor` anywhere in its parent chain.
    ///
    /// Returns `false` for unregistered kinds and when `child == ancestor`.
    pub fn is_subkind_of(&self, child: InfluenceKindId, ancestor: InfluenceKindId) -> bool {
        self.ancestors_of(child).contains(&ancestor)
    }

    /// Return `kind` plus all kinds that have `kind` anywhere in their ancestor chain.
    ///
    /// Scans all registered configs — O(n × depth). Fast for typical registry sizes.
    pub fn kind_and_descendants(&self, kind: InfluenceKindId) -> Vec<InfluenceKindId> {
        let mut result = vec![kind];
        for (&id, _) in &self.configs {
            if id != kind && self.is_subkind_of(id, kind) {
                result.push(id);
            }
        }
        result
    }

    // ─── Kind interaction rules ────────────────────────────────────────────────

    /// Declare the interaction effect between two influence kinds.
    ///
    /// The pair `(kind_a, kind_b)` is stored in canonical order so
    /// `interaction_between(a, b)` and `interaction_between(b, a)` return
    /// the same result. Registering a pair a second time overwrites the
    /// previous effect.
    pub fn register_interaction(
        &mut self,
        kind_a: InfluenceKindId,
        kind_b: InfluenceKindId,
        effect: InteractionEffect,
    ) {
        let key = canonical_pair(kind_a, kind_b);
        self.interactions.insert(key, effect);
    }

    /// Return the declared interaction effect for `(kind_a, kind_b)`, or `None`
    /// if no interaction has been registered for this pair.
    ///
    /// Lookup is symmetric: `interaction_between(a, b)` == `interaction_between(b, a)`.
    pub fn interaction_between(
        &self,
        kind_a: InfluenceKindId,
        kind_b: InfluenceKindId,
    ) -> Option<&InteractionEffect> {
        let key = canonical_pair(kind_a, kind_b);
        self.interactions.get(&key)
    }

    // ─── Slot inheritance ─────────────────────────────────────────────────────

    /// Return the resolved extra-slot list for `kind`, merging ancestor slots.
    ///
    /// Walks the parent chain from the root down to `kind`. A child's slot
    /// definition overrides a parent's slot of the same name. The returned
    /// `Vec` is in declaration order: ancestor slots first, then child slots
    /// (with any overrides applied in place).
    ///
    /// If `kind` is not registered, returns an empty `Vec`.
    pub fn resolved_extra_slots(&self, kind: InfluenceKindId) -> Vec<RelationshipSlotDef> {
        // Collect the ancestry chain (furthest ancestor first, kind last).
        let mut chain: Vec<InfluenceKindId> = self.ancestors_of(kind);
        chain.reverse();   // root → ... → parent
        chain.push(kind);  // root → ... → parent → kind

        // Merge: start from root slots, child overrides on name collision.
        let mut merged: Vec<RelationshipSlotDef> = Vec::new();
        for k in chain {
            if let Some(cfg) = self.configs.get(&k) {
                for slot in &cfg.extra_slots {
                    if let Some(existing) = merged.iter_mut().find(|s| s.name == slot.name) {
                        *existing = slot.clone();
                    } else {
                        merged.push(slot.clone());
                    }
                }
            }
        }
        merged
    }

    /// Build the initial `StateVector` for a new relationship of `kind`,
    /// including inherited extra slots from ancestor kinds.
    ///
    /// Layout: `[activity_contribution, 0.0, ...resolved_extra_slots...]`
    ///
    /// Child kinds override parent slots of the same name. Slots from
    /// ancestors are prepended in root→child order; own slots follow.
    pub fn resolved_initial_state_for(&self, kind: InfluenceKindId) -> StateVector {
        let activity_contribution = self.configs
            .get(&kind)
            .map(|c| c.activity_contribution)
            .unwrap_or(1.0);
        let resolved = self.resolved_extra_slots(kind);
        let mut values = vec![activity_contribution, 0.0f32];
        for slot in &resolved {
            values.push(slot.default);
        }
        StateVector::from_slice(&values)
    }

    fn rebuild_slot_defs(&mut self) {
        // Use resolved (inherited) slots so BatchContext sees the full slot
        // layout including ancestor-defined slots.
        self.slot_defs = self.configs
            .keys()
            .copied()
            .filter_map(|k| {
                let resolved = self.resolved_extra_slots(k);
                if resolved.is_empty() { None } else { Some((k, resolved)) }
            })
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

    /// Build the initial `StateVector` for a new relationship of `kind`,
    /// including inherited extra slots from ancestor kinds.
    ///
    /// Delegates to [`resolved_initial_state_for`] so child kinds automatically
    /// inherit parent-defined extra slots. Returns a minimal `[1.0, 0.0]` when
    /// the kind is not registered.
    ///
    /// Use this instead of `InfluenceKindConfig::initial_relationship_state()` in
    /// any code that should honour the kind hierarchy.
    pub fn initial_state_for(&self, kind: InfluenceKindId) -> StateVector {
        if !self.configs.contains_key(&kind) {
            debug_assert!(
                false,
                "initial_state_for called with unregistered InfluenceKindId: {kind:?}; \
                 extra-slot defaults will be missing from the returned state"
            );
            return StateVector::from_slice(&[1.0, 0.0]);
        }
        self.resolved_initial_state_for(kind)
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
    /// Only searches the kind's own `extra_slots` (not inherited ones).
    /// Use `resolved_slot_index` when the slot may come from an ancestor kind.
    ///
    /// Built-in slot 0 = activity, slot 1 = weight. Extra slots start at 2.
    pub fn slot_index(&self, kind: InfluenceKindId, name: &str) -> Option<usize> {
        self.get(kind)?.slot_index(name)
    }

    /// Return the `StateVector` index of a named extra slot for `kind`,
    /// searching the fully-resolved slot list (own + inherited from ancestors).
    ///
    /// Child overrides shadow parent slots of the same name. Returns `None`
    /// if the kind is not registered or the name is not found anywhere in the
    /// ancestry chain.
    ///
    /// Built-in slot 0 = activity, slot 1 = weight. Extra slots start at 2.
    pub fn resolved_slot_index(&self, kind: InfluenceKindId, name: &str) -> Option<usize> {
        self.resolved_extra_slots(kind)
            .into_iter()
            .position(|s| s.name == name)
            .map(|pos| pos + 2)
    }

    /// Read a named slot value from `rel`'s `StateVector` for `rel.kind`,
    /// searching own and inherited slots.
    ///
    /// Returns `None` if the kind is not registered, the slot name is unknown
    /// in the resolved slot list, or the state vector is too short.
    pub fn read_slot(&self, kind: InfluenceKindId, rel: &graph_core::Relationship, name: &str) -> Option<f32> {
        let idx = self.resolved_slot_index(kind, name)?;
        rel.state.as_slice().get(idx).copied()
    }

    pub fn len(&self) -> usize {
        self.configs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.configs.is_empty()
    }
}

/// Return a canonical (min, max) pair so interaction lookups are symmetric.
#[inline]
fn canonical_pair(a: InfluenceKindId, b: InfluenceKindId) -> (InfluenceKindId, InfluenceKindId) {
    if a <= b { (a, b) } else { (b, a) }
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
                stdp: false,
            ..Default::default()
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
                stdp: false,
            ..Default::default()
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

    // ─── Kind hierarchy ────────────────────────────────────────────────────────

    fn three_level_registry() -> InfluenceKindRegistry {
        // root → mid → leaf hierarchy
        // root(1), mid(2, parent=1), leaf(3, parent=2)
        let mut reg = InfluenceKindRegistry::new();
        reg.insert(InfluenceKindId(1), InfluenceKindConfig::new("root"));
        reg.insert(
            InfluenceKindId(2),
            InfluenceKindConfig::new("mid").with_parent(InfluenceKindId(1)),
        );
        reg.insert(
            InfluenceKindId(3),
            InfluenceKindConfig::new("leaf").with_parent(InfluenceKindId(2)),
        );
        reg
    }

    #[test]
    fn ancestors_of_root_is_empty() {
        let reg = three_level_registry();
        assert!(reg.ancestors_of(InfluenceKindId(1)).is_empty());
    }

    #[test]
    fn ancestors_of_mid_contains_root() {
        let reg = three_level_registry();
        let ancestors = reg.ancestors_of(InfluenceKindId(2));
        assert_eq!(ancestors, vec![InfluenceKindId(1)]);
    }

    #[test]
    fn ancestors_of_leaf_is_root_and_mid() {
        let reg = three_level_registry();
        let ancestors = reg.ancestors_of(InfluenceKindId(3));
        assert_eq!(ancestors, vec![InfluenceKindId(2), InfluenceKindId(1)]);
    }

    #[test]
    fn is_subkind_of_transitive() {
        let reg = three_level_registry();
        assert!(reg.is_subkind_of(InfluenceKindId(3), InfluenceKindId(1))); // leaf is sub of root
        assert!(reg.is_subkind_of(InfluenceKindId(3), InfluenceKindId(2))); // leaf is sub of mid
        assert!(reg.is_subkind_of(InfluenceKindId(2), InfluenceKindId(1))); // mid is sub of root
        assert!(!reg.is_subkind_of(InfluenceKindId(1), InfluenceKindId(3))); // root is NOT sub of leaf
        assert!(!reg.is_subkind_of(InfluenceKindId(3), InfluenceKindId(3))); // not sub of itself
    }

    #[test]
    fn kind_and_descendants_includes_all_children() {
        let reg = three_level_registry();
        let mut desc = reg.kind_and_descendants(InfluenceKindId(1));
        desc.sort();
        assert_eq!(desc, vec![InfluenceKindId(1), InfluenceKindId(2), InfluenceKindId(3)]);

        let mut mid_desc = reg.kind_and_descendants(InfluenceKindId(2));
        mid_desc.sort();
        assert_eq!(mid_desc, vec![InfluenceKindId(2), InfluenceKindId(3)]);

        let leaf_desc = reg.kind_and_descendants(InfluenceKindId(3));
        assert_eq!(leaf_desc, vec![InfluenceKindId(3)]);
    }

    #[test]
    #[should_panic(expected = "parent")]
    fn insert_with_unregistered_parent_panics() {
        let mut reg = InfluenceKindRegistry::new();
        // parent(99) is not registered
        reg.insert(
            InfluenceKindId(1),
            InfluenceKindConfig::new("orphan").with_parent(InfluenceKindId(99)),
        );
    }

    // ─── Kind interaction rules ────────────────────────────────────────────────

    #[test]
    fn interaction_between_is_symmetric() {
        let mut reg = InfluenceKindRegistry::new();
        reg.insert(InfluenceKindId(1), InfluenceKindConfig::new("excite"));
        reg.insert(InfluenceKindId(2), InfluenceKindConfig::new("inhibit"));
        reg.register_interaction(
            InfluenceKindId(1),
            InfluenceKindId(2),
            InteractionEffect::Antagonistic { dampen: 0.5 },
        );
        // Both orderings return the same result
        assert_eq!(
            reg.interaction_between(InfluenceKindId(1), InfluenceKindId(2)),
            reg.interaction_between(InfluenceKindId(2), InfluenceKindId(1)),
        );
        assert!(matches!(
            reg.interaction_between(InfluenceKindId(1), InfluenceKindId(2)),
            Some(InteractionEffect::Antagonistic { dampen }) if (*dampen - 0.5).abs() < 1e-6
        ));
    }

    #[test]
    fn interaction_between_unregistered_pair_is_none() {
        let mut reg = InfluenceKindRegistry::new();
        reg.insert(InfluenceKindId(1), InfluenceKindConfig::new("a"));
        reg.insert(InfluenceKindId(2), InfluenceKindConfig::new("b"));
        assert!(reg.interaction_between(InfluenceKindId(1), InfluenceKindId(2)).is_none());
    }

    #[test]
    fn register_interaction_overwrite() {
        let mut reg = InfluenceKindRegistry::new();
        reg.insert(InfluenceKindId(1), InfluenceKindConfig::new("a"));
        reg.insert(InfluenceKindId(2), InfluenceKindConfig::new("b"));
        reg.register_interaction(
            InfluenceKindId(1),
            InfluenceKindId(2),
            InteractionEffect::Synergistic { boost: 1.5 },
        );
        reg.register_interaction(
            InfluenceKindId(1),
            InfluenceKindId(2),
            InteractionEffect::Neutral,
        );
        assert_eq!(
            reg.interaction_between(InfluenceKindId(1), InfluenceKindId(2)),
            Some(&InteractionEffect::Neutral),
        );
    }

    // ─── Slot inheritance ─────────────────────────────────────────────────────

    #[test]
    fn resolved_extra_slots_root_only() {
        use graph_core::RelationshipSlotDef;
        let mut reg = InfluenceKindRegistry::new();
        reg.insert(
            InfluenceKindId(1),
            InfluenceKindConfig::new("root").with_extra_slots(vec![
                RelationshipSlotDef::new("tension", 0.5),
            ]),
        );
        let slots = reg.resolved_extra_slots(InfluenceKindId(1));
        assert_eq!(slots.len(), 1);
        assert_eq!(slots[0].name, "tension");
        assert!((slots[0].default - 0.5).abs() < 1e-6);
    }

    #[test]
    fn resolved_extra_slots_child_inherits_parent_slot() {
        use graph_core::RelationshipSlotDef;
        let mut reg = InfluenceKindRegistry::new();
        reg.insert(
            InfluenceKindId(1),
            InfluenceKindConfig::new("parent").with_extra_slots(vec![
                RelationshipSlotDef::new("trust", 1.0),
            ]),
        );
        reg.insert(
            InfluenceKindId(2),
            InfluenceKindConfig::new("child")
                .with_parent(InfluenceKindId(1))
                .with_extra_slots(vec![
                    RelationshipSlotDef::new("hostility", 0.0),
                ]),
        );
        let slots = reg.resolved_extra_slots(InfluenceKindId(2));
        // trust (from parent) first, hostility (own) second
        assert_eq!(slots.len(), 2);
        assert_eq!(slots[0].name, "trust");
        assert_eq!(slots[1].name, "hostility");
    }

    #[test]
    fn resolved_extra_slots_child_overrides_parent_slot_by_name() {
        use graph_core::RelationshipSlotDef;
        let mut reg = InfluenceKindRegistry::new();
        reg.insert(
            InfluenceKindId(1),
            InfluenceKindConfig::new("parent").with_extra_slots(vec![
                RelationshipSlotDef::new("trust", 1.0),
            ]),
        );
        reg.insert(
            InfluenceKindId(2),
            InfluenceKindConfig::new("child")
                .with_parent(InfluenceKindId(1))
                .with_extra_slots(vec![
                    RelationshipSlotDef::new("trust", 0.5), // overrides parent default
                ]),
        );
        let slots = reg.resolved_extra_slots(InfluenceKindId(2));
        // Only one "trust" slot — child override wins
        assert_eq!(slots.len(), 1);
        assert_eq!(slots[0].name, "trust");
        assert!((slots[0].default - 0.5).abs() < 1e-6);
    }

    #[test]
    fn initial_state_for_includes_inherited_slots() {
        use graph_core::RelationshipSlotDef;
        let mut reg = InfluenceKindRegistry::new();
        reg.insert(
            InfluenceKindId(1),
            InfluenceKindConfig::new("parent").with_extra_slots(vec![
                RelationshipSlotDef::new("trust", 0.8),
            ]),
        );
        reg.insert(
            InfluenceKindId(2),
            InfluenceKindConfig::new("child")
                .with_parent(InfluenceKindId(1))
                .with_extra_slots(vec![
                    RelationshipSlotDef::new("hostility", 0.3),
                ]),
        );
        let state = reg.initial_state_for(InfluenceKindId(2));
        let s = state.as_slice();
        // [activity=1.0, weight=0.0, trust=0.8, hostility=0.3]
        assert_eq!(s.len(), 4);
        assert!((s[0] - 1.0).abs() < 1e-6, "activity");
        assert!((s[1] - 0.0).abs() < 1e-6, "weight");
        assert!((s[2] - 0.8).abs() < 1e-6, "trust (inherited)");
        assert!((s[3] - 0.3).abs() < 1e-6, "hostility (own)");
    }

    #[test]
    fn resolved_slot_index_finds_inherited_slot() {
        use graph_core::RelationshipSlotDef;
        let mut reg = InfluenceKindRegistry::new();
        reg.insert(
            InfluenceKindId(1),
            InfluenceKindConfig::new("parent").with_extra_slots(vec![
                RelationshipSlotDef::new("trust", 1.0),
            ]),
        );
        reg.insert(
            InfluenceKindId(2),
            InfluenceKindConfig::new("child")
                .with_parent(InfluenceKindId(1))
                .with_extra_slots(vec![
                    RelationshipSlotDef::new("hostility", 0.0),
                ]),
        );
        // "trust" is at slot 2 (first extra slot, inherited from parent)
        assert_eq!(reg.resolved_slot_index(InfluenceKindId(2), "trust"), Some(2));
        // "hostility" is at slot 3 (second extra slot, own)
        assert_eq!(reg.resolved_slot_index(InfluenceKindId(2), "hostility"), Some(3));
        // Plain slot_index only sees own slots, so "trust" not found for child
        assert_eq!(reg.slot_index(InfluenceKindId(2), "trust"), None);
    }
}
