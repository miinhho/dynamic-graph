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
//! as immutable for the duration of a run. Lookup panics in debug builds
//! and returns `None` in release builds — debug-only panics are how O6
//! mitigates the loss of compile-time type safety.

use std::collections::HashMap;

use graph_core::{InfluenceKindId, LocusKindId, LocusProgram};

/// Per-influence-kind configuration. Held verbatim by the
/// `InfluenceKindRegistry`. Tunables that the guard rail and regime
/// classifier consume land here in later commits; for now this is just
/// a name + a continuous-decay default so the type carries something
/// real.
#[derive(Debug, Clone)]
pub struct InfluenceKindConfig {
    /// Human-readable label for diagnostics. Not used by the engine.
    pub name: String,
    /// Per-batch multiplicative decay applied to relationship activity
    /// of this kind. `1.0` means "no decay"; smaller means "fades
    /// faster". This is the *continuous decay* mode from
    /// `docs/redesign.md` §3.5, not the staged-transformation mode
    /// used by entity sediment.
    pub decay_per_batch: f32,
}

impl InfluenceKindConfig {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            decay_per_batch: 1.0,
        }
    }

    pub fn with_decay(mut self, decay_per_batch: f32) -> Self {
        self.decay_per_batch = decay_per_batch;
        self
    }
}

/// Owns the per-locus-kind program implementations.
#[derive(Default)]
pub struct LocusKindRegistry {
    programs: HashMap<LocusKindId, Box<dyn LocusProgram>>,
}

impl LocusKindRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a program for a locus kind. Panics if the kind is
    /// already registered — duplicate registration is a programming
    /// error, not a runtime situation we want to silently overwrite.
    pub fn insert(&mut self, kind: LocusKindId, program: Box<dyn LocusProgram>) {
        if self.programs.insert(kind, program).is_some() {
            panic!("LocusKindRegistry: duplicate registration for {kind:?}");
        }
    }

    pub fn get(&self, kind: LocusKindId) -> Option<&dyn LocusProgram> {
        self.programs.get(&kind).map(|boxed| boxed.as_ref())
    }

    /// Same as `get` but panics in debug builds when the kind is missing.
    /// This is the lookup path the batch loop uses — see O6 in
    /// `docs/redesign.md` §8 for why debug-only panics are the chosen
    /// safety net.
    pub fn require(&self, kind: LocusKindId) -> Option<&dyn LocusProgram> {
        let found = self.get(kind);
        debug_assert!(found.is_some(), "unregistered LocusKindId: {kind:?}");
        found
    }

    pub fn len(&self) -> usize {
        self.programs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.programs.is_empty()
    }
}

/// Owns the per-influence-kind config used by the guard rail, regime
/// classifier, and relationship layer.
#[derive(Debug, Default, Clone)]
pub struct InfluenceKindRegistry {
    configs: HashMap<InfluenceKindId, InfluenceKindConfig>,
}

impl InfluenceKindRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, kind: InfluenceKindId, config: InfluenceKindConfig) {
        if self.configs.insert(kind, config).is_some() {
            panic!("InfluenceKindRegistry: duplicate registration for {kind:?}");
        }
    }

    pub fn get(&self, kind: InfluenceKindId) -> Option<&InfluenceKindConfig> {
        self.configs.get(&kind)
    }

    pub fn require(&self, kind: InfluenceKindId) -> Option<&InfluenceKindConfig> {
        let found = self.get(kind);
        debug_assert!(found.is_some(), "unregistered InfluenceKindId: {kind:?}");
        found
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
        fn process(&self, _: &Locus, _: &[Change]) -> Vec<ProposedChange> {
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
}
