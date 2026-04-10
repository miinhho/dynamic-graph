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
        }
    }
}
