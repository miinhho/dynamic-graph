//! Configuration types and observable output for `Simulation`.
//!
//! Phase 8 (2026-04-18): `history_window`, `change_retention_batches`,
//! `cold_relationship_threshold`, `cold_relationship_min_idle_batches`,
//! `auto_weather_every_ticks`, `event_history_len`,
//! `pending_stimuli_capacity`, `backpressure_policy` were hard-coded —
//! no benchmark used non-default values. The public knobs on
//! `SimulationConfig` are now `engine` (which still owns
//! `max_batches_per_tick`), `auto_commit` (storage feature only), and
//! `adaptive` (a ZST post Phase 7). See `docs/complexity-audit.md`.

use rustc_hash::FxHashMap;

use graph_core::{InfluenceKindId, WorldEvent};

use crate::engine::{EngineConfig, TickResult};
use crate::regime::AdaptiveConfig;
use crate::regime::DynamicsRegime;

/// Internal defaults for the lifecycle parameters hard-coded in Phase 8.
pub(crate) const HISTORY_WINDOW: usize = 8;
pub(crate) const COLD_RELATIONSHIP_MIN_IDLE_BATCHES: u64 = 50;
pub(crate) const BACKPRESSURE_POLICY: BackpressurePolicy = BackpressurePolicy::Reject;

/// Snapshot of observable state after one `step()`.
#[derive(Debug, Clone)]
pub struct StepObservation {
    /// Result from the underlying `Engine::tick` call.
    pub tick: TickResult,
    /// Dynamical regime classified from the rolling history window.
    pub regime: DynamicsRegime,
    /// Total relationships in the world.
    pub relationships: usize,
    /// Number of entities with `EntityStatus::Active`.
    pub active_entities: usize,
    /// Current guard-rail scale per registered influence kind.
    pub scales: FxHashMap<InfluenceKindId, f32>,
    /// Current plasticity learning-rate scale per registered influence kind.
    pub plasticity_scales: FxHashMap<InfluenceKindId, f32>,
    /// Events emitted during this step.
    pub events: Vec<WorldEvent>,
    /// Rich per-tick summary.
    pub summary: super::observability::TickSummary,
}

/// What to do when `pending_stimuli` is full and a new stimulus arrives.
///
/// Retained as an enum for internal use; no longer a public knob on
/// `SimulationConfig` (hard-coded to `Reject` in Phase 8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum BackpressurePolicy {
    #[default]
    Reject,
    DropOldest,
    DropNewest,
}

/// Configuration for `Simulation`. The post-Phase-8 public surface is
/// intentionally small — all former lifecycle knobs are internal constants.
#[derive(Debug, Clone)]
pub struct SimulationConfig {
    pub engine: EngineConfig,
    pub adaptive: AdaptiveConfig,
    /// Path to a redb database file. `Some` enables persistence via
    /// `graph-storage`. Requires the `storage` feature.
    #[cfg(feature = "storage")]
    pub storage_path: Option<std::path::PathBuf>,
    /// When `true`, each `step()` automatically calls
    /// `Storage::commit_batch` for every batch just committed. When
    /// `false`, batches accumulate in memory until
    /// `Simulation::flush()`. No-op without the `storage` feature.
    ///
    /// **Default**: `true` — the safe, durability-first choice for
    /// the storage feature (matches "every step survives a crash").
    ///
    /// **Override when**: throughput matters more than durability and
    /// the caller will explicitly batch-flush (bulk ingest, offline
    /// replay, simulation sweeps). Set `false`, issue many `step()`s,
    /// then `Simulation::flush()` once at the end.
    #[cfg(feature = "storage")]
    pub auto_commit: bool,
}

impl Default for SimulationConfig {
    fn default() -> Self {
        Self {
            engine: EngineConfig::default(),
            adaptive: AdaptiveConfig,
            #[cfg(feature = "storage")]
            storage_path: None,
            #[cfg(feature = "storage")]
            auto_commit: true,
        }
    }
}
