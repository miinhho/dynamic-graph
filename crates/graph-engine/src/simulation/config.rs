//! Configuration types and observable output for `Simulation`.

use rustc_hash::FxHashMap;

use graph_core::{InfluenceKindId, WorldEvent};

use crate::regime::AdaptiveConfig;
use crate::engine::{EngineConfig, TickResult};
use crate::regime::DynamicsRegime;

/// Snapshot of observable state after one `step()`.
#[derive(Debug, Clone)]
pub struct StepObservation {
    /// Result from the underlying `Engine::tick` call.
    pub tick: TickResult,
    /// Dynamical regime classified from the rolling history window.
    pub regime: DynamicsRegime,
    /// Total relationships in the world (all, not just active above a
    /// threshold — relationships are kept until explicitly deleted).
    pub relationships: usize,
    /// Number of entities with `EntityStatus::Active`.
    pub active_entities: usize,
    /// Current guard-rail scale per registered influence kind. A scale
    /// of 1.0 means the guard rail is fully open; < 1.0 means it has
    /// tightened in response to divergence.
    pub scales: FxHashMap<InfluenceKindId, f32>,
    /// Events emitted during this step (entity lifecycle, pruning,
    /// regime shifts). Empty unless `recognize_entities` or
    /// `flush_relationship_decay` was called within the step, or the
    /// regime changed.
    pub events: Vec<WorldEvent>,
    /// Rich per-tick summary: per-batch stats, loci changed, categorized
    /// event counts. A superset of the information in `tick` and `events`.
    pub summary: super::observability::TickSummary,
}

/// What to do when `pending_stimuli` is full and a new stimulus arrives.
///
/// Configured via [`SimulationConfig::pending_stimuli_capacity`] and
/// [`SimulationConfig::backpressure_policy`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BackpressurePolicy {
    /// Silently drop the incoming stimulus. The locus is still created/merged;
    /// its change is just not queued this cycle. This is the default.
    #[default]
    Reject,
    /// Remove the oldest queued stimulus to make room for the new one.
    DropOldest,
    /// Discard the incoming stimulus (same observable effect as `Reject`,
    /// but signals intentional newest-first priority rather than arrival-first).
    DropNewest,
}

/// Configuration for `Simulation`.
#[derive(Debug, Clone)]
pub struct SimulationConfig {
    pub engine: EngineConfig,
    pub adaptive: AdaptiveConfig,
    /// Number of ticks to keep in history for regime classification.
    /// Regime is `Initializing` until the window is full.
    pub history_window: usize,
    /// Path to a redb database file. If `Some`, each step is persisted
    /// via `graph-storage` (ACID transactions, random-access reads,
    /// automatic compaction). `None` disables persistence.
    /// Requires the `storage` feature.
    #[cfg(feature = "storage")]
    pub storage_path: Option<std::path::PathBuf>,
    /// When set, the in-memory `ChangeLog` is automatically trimmed to
    /// retain only the most recent `N` batches of changes. Older changes
    /// remain accessible via the storage backend (if configured).
    /// `None` disables automatic trimming (unbounded growth).
    pub change_retention_batches: Option<u64>,
    /// When set, relationships with `activity < cold_relationship_threshold`
    /// that have not been touched for `cold_relationship_min_idle_batches`
    /// are evicted from memory after each step. They remain in storage
    /// and are promoted back on demand.
    pub cold_relationship_threshold: Option<f32>,
    /// Minimum number of batches a relationship must be idle (untouched)
    /// before it can be evicted to cold storage. Only meaningful when
    /// `cold_relationship_threshold` is `Some`. Default: 50.
    pub cold_relationship_min_idle_batches: u64,
    /// When set, `EntityWeatheringPolicy` is applied automatically every
    /// N `step()` calls. The policy itself is supplied separately via
    /// `SimulationBuilder::auto_weather` or `auto_weather_with`.
    /// `None` disables automatic weathering (the default).
    pub auto_weather_every_ticks: Option<u32>,
    /// When `true` (the default), each `step()` automatically persists
    /// committed batches to the redb storage backend. When `false`,
    /// batches accumulate in memory until the caller explicitly calls
    /// `Simulation::flush()`. No-op when storage is not configured.
    ///
    /// Lazy flushing reduces write amplification for high-frequency
    /// simulations: batch N writes of 1 tick each → 1 write of N ticks.
    #[cfg(feature = "storage")]
    pub auto_commit: bool,
    /// Number of [`TickSummary`][super::observability::TickSummary] records
    /// to retain in the rolling [`EventHistory`][super::observability::EventHistory]
    /// attached to `Simulation`.
    ///
    /// `0` (the default) disables the history ring buffer. When non-zero,
    /// `Simulation::history()` returns an `EventHistory` with the last N
    /// summaries, accessible for trend analysis across steps.
    pub event_history_len: usize,
    /// Maximum number of stimuli that may be queued in `pending_stimuli`
    /// between `step()` calls. `0` means unbounded (the default).
    ///
    /// When the queue is full and a new ingest arrives, [`backpressure_policy`]
    /// determines what happens.
    ///
    /// [`backpressure_policy`]: SimulationConfig::backpressure_policy
    pub pending_stimuli_capacity: usize,
    /// What to do when `pending_stimuli` is at capacity.
    /// Only meaningful when [`pending_stimuli_capacity`] is non-zero.
    ///
    /// [`pending_stimuli_capacity`]: SimulationConfig::pending_stimuli_capacity
    pub backpressure_policy: BackpressurePolicy,
}

impl Default for SimulationConfig {
    fn default() -> Self {
        Self {
            engine: EngineConfig::default(),
            adaptive: AdaptiveConfig::default(),
            history_window: 8,
            #[cfg(feature = "storage")]
            storage_path: None,
            change_retention_batches: None,
            cold_relationship_threshold: None,
            cold_relationship_min_idle_batches: 50,
            auto_weather_every_ticks: None,
            #[cfg(feature = "storage")]
            auto_commit: true,
            event_history_len: 0,
            pending_stimuli_capacity: 0,
            backpressure_policy: BackpressurePolicy::default(),
        }
    }
}
