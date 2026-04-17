//! High-level domain-data ingestion API.
//!
//! Users work with names, properties, and relationship labels; the ingest
//! layer handles LocusId allocation, PropertyStore/NameIndex bookkeeping,
//! encoding, and stimulus generation.
//!
//! ## Error-safe variants
//!
//! Every panic-based method (`ingest_named`, `ingest_batch_named`,
//! `ingest_named_with`) has a `try_` counterpart that returns
//! `Result<_, IngestError>` instead of panicking. Use the `try_` variants
//! in production pipelines where a typo in a kind name should not crash the
//! process.

use graph_core::ProposedChange;

use super::Simulation;
use super::config::{BackpressurePolicy, StepObservation};

// ── Error type ────────────────────────────────────────────────────────────────

/// Errors that can occur during string-based ingestion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IngestError {
    /// The locus kind name was never registered.
    ///
    /// Register it with [`SimulationBuilder::locus_kind`] or
    /// [`Simulation::register_locus_kind_name`].
    UnknownLocusKind { name: String },
    /// The influence kind name was never registered.
    ///
    /// Register it with [`SimulationBuilder::influence`] or
    /// [`Simulation::register_influence_name`].
    UnknownInfluenceKind { name: String },
    /// No default influence kind has been set.
    ///
    /// Call [`SimulationBuilder::default_influence`] or
    /// [`Simulation::set_default_influence`].
    NoDefaultInfluence,
}

impl std::fmt::Display for IngestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IngestError::UnknownLocusKind { name } =>
                write!(f, "unknown locus kind \"{name}\" — register it with SimulationBuilder::locus_kind() or register_locus_kind_name()"),
            IngestError::UnknownInfluenceKind { name } =>
                write!(f, "unknown influence kind \"{name}\" — register it with SimulationBuilder::influence() or register_influence_name()"),
            IngestError::NoDefaultInfluence =>
                write!(f, "no default influence kind set — call SimulationBuilder::default_influence() or set_default_influence()"),
        }
    }
}

impl std::error::Error for IngestError {}

impl Simulation {
    /// Push a stimulus onto `pending_stimuli`, applying backpressure if at capacity.
    ///
    /// Returns `true` when the stimulus was queued, `false` when it was dropped
    /// (`Reject` / `DropNewest` policy). The locus is always created/updated
    /// before this is called, so a dropped stimulus does not roll back the locus.
    pub(crate) fn push_pending_stimulus(&mut self, stimulus: ProposedChange) -> bool {
        let cap = self.pending_stimuli_capacity;
        if cap == 0 || self.pending_stimuli.len() < cap {
            self.pending_stimuli.push(stimulus);
            return true;
        }
        match self.backpressure_policy {
            BackpressurePolicy::Reject | BackpressurePolicy::DropNewest => false,
            BackpressurePolicy::DropOldest => {
                self.pending_stimuli.remove(0);
                self.pending_stimuli.push(stimulus);
                true
            }
        }
    }

    /// Ingest a named entity with domain properties.
    ///
    /// - If `name` is new: creates a `Locus`, stores properties, registers
    ///   the name, and returns the fresh `LocusId`.
    /// - If `name` already exists: merges properties into the existing
    ///   entry and returns the existing `LocusId`.
    ///
    /// In both cases a stimulus `ProposedChange` is pushed to
    /// `pending_stimuli`. Call [`step`](Simulation::step) (or
    /// [`flush_ingested`](Simulation::flush_ingested)) to commit them to
    /// the engine.
    ///
    /// `kind` determines which `LocusProgram` and `Encoder` handle this
    /// entity.
    pub fn ingest(
        &mut self,
        name: &str,
        kind: graph_core::LocusKindId,
        influence: graph_core::InfluenceKindId,
        properties: graph_core::Properties,
    ) -> graph_core::LocusId {
        let state = self.encode(kind, &properties);

        let locus_id = {
            let mut world = self.world.write().unwrap();
            if let Some(existing) = world.names().resolve(name) {
                if let Some(props) = world.properties_mut().get_mut(existing) {
                    props.extend(&properties);
                }
                existing
            } else {
                let id = world.loci().next_id();
                let locus = graph_core::Locus::new(id, kind, state.clone());
                world.insert_locus(locus);
                world.names_mut().insert(name, id);
                world.properties_mut().insert(id, properties);
                id
            }
        };

        self.push_pending_stimulus(ProposedChange::new(
            graph_core::ChangeSubject::Locus(locus_id),
            influence,
            state,
        ));

        locus_id
    }

    /// Ingest a batch of co-occurring entities (e.g. from one document).
    ///
    /// All entities are ingested and their stimuli are queued. When the next
    /// `step()` / `tick()` fires, the engine auto-emerges relationships between
    /// loci that share cross-locus causal predecessors in the same batch — this
    /// happens inside the batch loop, not here.
    ///
    /// Returns the `LocusId`s in the same order as `entries`.
    pub fn ingest_batch(
        &mut self,
        entries: Vec<(&str, graph_core::LocusKindId, graph_core::Properties)>,
        influence: graph_core::InfluenceKindId,
    ) -> Vec<graph_core::LocusId> {
        entries
            .into_iter()
            .map(|(name, kind, props)| self.ingest(name, kind, influence, props))
            .collect()
    }

    /// Step the simulation, draining all pending ingested stimuli plus
    /// any additional explicit stimuli.
    pub fn step_with_ingest(&mut self, extra_stimuli: Vec<ProposedChange>) -> StepObservation {
        self.step(extra_stimuli)
    }

    /// Drain pending stimuli without additional stimuli.
    pub fn flush_ingested(&mut self) -> StepObservation {
        self.step_with_ingest(Vec::new())
    }

    /// Resolve a name to its `LocusId`, if known.
    pub fn resolve(&self, name: &str) -> Option<graph_core::LocusId> {
        self.world.read().unwrap().names().resolve(name)
    }

    /// Get the domain properties for a locus (cloned owned value).
    pub fn properties_of(&self, id: graph_core::LocusId) -> Option<graph_core::Properties> {
        self.world.read().unwrap().properties().get(id).cloned()
    }

    /// Get the canonical name for a locus (owned `String`).
    pub fn name_of(&self, id: graph_core::LocusId) -> Option<String> {
        self.world.read().unwrap().names().name_of(id).map(str::to_owned)
    }

    // ── String-based convenience API ────────────────────────────────────

    /// Register a string name for a `LocusKindId` so that `ingest` can
    /// accept `"ORG"` instead of `LocusKindId(1)`.
    pub fn register_locus_kind_name(&mut self, name: impl Into<String>, kind: graph_core::LocusKindId) {
        self.locus_kind_names.insert(name.into(), kind);
    }

    /// Register a string name for an `InfluenceKindId`.
    pub fn register_influence_name(&mut self, name: impl Into<String>, kind: graph_core::InfluenceKindId) {
        self.influence_kind_names.insert(name.into(), kind);
    }

    /// Set the default influence kind used by string-based ingest methods.
    pub fn set_default_influence(&mut self, kind: graph_core::InfluenceKindId) {
        self.default_influence = Some(kind);
    }

    /// Returns `true` if `name` has been registered as a locus kind.
    ///
    /// Useful for guard checks before calling `ingest_named` / `try_ingest_named`.
    pub fn has_locus_kind(&self, name: &str) -> bool {
        self.locus_kind_names.contains_key(name)
    }

    /// Returns `true` if `name` has been registered as an influence kind.
    pub fn has_influence(&self, name: &str) -> bool {
        self.influence_kind_names.contains_key(name)
    }

    /// Resolve a locus kind name to its `LocusKindId`.
    pub(crate) fn resolve_locus_kind(&self, name: &str) -> graph_core::LocusKindId {
        *self.locus_kind_names.get(name).unwrap_or_else(|| {
            panic!("unknown locus kind \"{name}\" — register it via SimulationBuilder::locus_kind() or register_locus_kind_name()")
        })
    }

    /// Fallible version of `resolve_locus_kind`.
    fn try_resolve_locus_kind(&self, name: &str) -> Result<graph_core::LocusKindId, IngestError> {
        self.locus_kind_names.get(name).copied().ok_or_else(|| IngestError::UnknownLocusKind { name: name.to_owned() })
    }

    /// Resolve an influence kind name, or fall back to the default.
    pub(crate) fn resolve_influence(&self, name: Option<&str>) -> graph_core::InfluenceKindId {
        match name {
            Some(n) => *self.influence_kind_names.get(n).unwrap_or_else(|| {
                panic!("unknown influence kind \"{n}\" — register it via SimulationBuilder::influence() or register_influence_name()")
            }),
            None => self.default_influence.unwrap_or_else(|| {
                panic!("no default influence kind set — call set_default_influence() or SimulationBuilder::default_influence()")
            }),
        }
    }

    /// Fallible version of `resolve_influence`.
    fn try_resolve_influence(&self, name: Option<&str>) -> Result<graph_core::InfluenceKindId, IngestError> {
        match name {
            Some(n) => self.influence_kind_names.get(n).copied().ok_or_else(|| IngestError::UnknownInfluenceKind { name: n.to_owned() }),
            None => self.default_influence.ok_or(IngestError::NoDefaultInfluence),
        }
    }

    /// Ingest using string kind names. Uses the default influence kind.
    pub fn ingest_named(
        &mut self,
        name: &str,
        kind: &str,
        properties: graph_core::Properties,
    ) -> graph_core::LocusId {
        let kind_id = self.resolve_locus_kind(kind);
        let influence = self.resolve_influence(None);
        self.ingest(name, kind_id, influence, properties)
    }

    /// Ingest using string kind names with a specific influence kind.
    pub fn ingest_named_with(
        &mut self,
        name: &str,
        kind: &str,
        influence: &str,
        properties: graph_core::Properties,
    ) -> graph_core::LocusId {
        let kind_id = self.resolve_locus_kind(kind);
        let influence_id = self.resolve_influence(Some(influence));
        self.ingest(name, kind_id, influence_id, properties)
    }

    /// Ingest a batch of co-occurring entities using string kind names.
    pub fn ingest_batch_named(
        &mut self,
        entries: Vec<(&str, &str, graph_core::Properties)>,
    ) -> Vec<graph_core::LocusId> {
        let influence = self.resolve_influence(None);
        let resolved: Vec<(&str, graph_core::LocusKindId, graph_core::Properties)> = entries
            .into_iter()
            .map(|(name, kind, props)| (name, self.resolve_locus_kind(kind), props))
            .collect();
        self.ingest_batch(resolved, influence)
    }

    // ── Result-returning variants ─────────────────────────────────────────────

    /// Like [`ingest_named`](Self::ingest_named) but returns an error instead
    /// of panicking when a kind name is unregistered.
    pub fn try_ingest_named(
        &mut self,
        name: &str,
        kind: &str,
        properties: graph_core::Properties,
    ) -> Result<graph_core::LocusId, IngestError> {
        let kind_id = self.try_resolve_locus_kind(kind)?;
        let influence = self.try_resolve_influence(None)?;
        Ok(self.ingest(name, kind_id, influence, properties))
    }

    /// Like [`ingest_named_with`](Self::ingest_named_with) but returns an
    /// error instead of panicking.
    pub fn try_ingest_named_with(
        &mut self,
        name: &str,
        kind: &str,
        influence: &str,
        properties: graph_core::Properties,
    ) -> Result<graph_core::LocusId, IngestError> {
        let kind_id = self.try_resolve_locus_kind(kind)?;
        let influence_id = self.try_resolve_influence(Some(influence))?;
        Ok(self.ingest(name, kind_id, influence_id, properties))
    }

    /// Like [`ingest_batch_named`](Self::ingest_batch_named) but returns an
    /// error (without ingesting anything) if any kind name is unregistered.
    ///
    /// All names are validated before any locus is created, so the world
    /// stays consistent on failure.
    pub fn try_ingest_batch_named(
        &mut self,
        entries: Vec<(&str, &str, graph_core::Properties)>,
    ) -> Result<Vec<graph_core::LocusId>, IngestError> {
        let influence = self.try_resolve_influence(None)?;
        // Validate all kind names up-front so we don't partially mutate the world.
        let resolved: Vec<(&str, graph_core::LocusKindId, graph_core::Properties)> = entries
            .into_iter()
            .map(|(name, kind, props)| {
                self.try_resolve_locus_kind(kind).map(|kid| (name, kid, props))
            })
            .collect::<Result<_, _>>()?;
        Ok(self.ingest_batch(resolved, influence))
    }

    // ── Co-occurrence helpers ─────────────────────────────────────────────────

    /// Ingest a group of co-occurring entities and immediately step.
    ///
    /// This is the ergonomic alternative to the boilerplate required for
    /// document-style co-occurrence graphs. It:
    ///
    /// 1. Creates or merges each named entity (same as `ingest_named`).
    /// 2. Wires cross-locus predecessors between the entities so the engine's
    ///    auto-emergence logic can relate them.
    /// 3. Calls `step()` to commit the batch.
    ///
    /// ## Relationship emergence
    ///
    /// The engine emerges a relationship between two loci when both have
    /// changes that share a cross-locus predecessor in the same batch. For
    /// **new** loci (no prior changes), no relationship emerges on their
    /// first co-occurrence. Call `ingest_cooccurrence` again with the same
    /// entities to create the relationship (or call it twice if you want
    /// the relationship to appear immediately).
    ///
    /// ## Panic behaviour
    ///
    /// Panics if any kind name is unregistered (same as `ingest_named`).
    /// Use `try_ingest_cooccurrence` for a `Result`-returning variant.
    ///
    /// ## Example
    ///
    /// ```rust,ignore
    /// // First call: creates the loci.
    /// sim.ingest_cooccurrence(vec![
    ///     ("alice", "PERSON", props! { "name" => "alice" }),
    ///     ("bob",   "PERSON", props! { "name" => "bob"   }),
    /// ]);
    /// // Second call: loci already exist → cross-locus predecessors fire →
    /// // the engine emerges a relationship between alice and bob.
    /// sim.ingest_cooccurrence(vec![
    ///     ("alice", "PERSON", props! { "name" => "alice" }),
    ///     ("bob",   "PERSON", props! { "name" => "bob"   }),
    /// ]);
    /// ```
    pub fn ingest_cooccurrence(
        &mut self,
        entries: Vec<(&str, &str, graph_core::Properties)>,
    ) -> super::config::StepObservation {
        let influence = self.resolve_influence(None);
        self.ingest_cooccurrence_with_influence(entries, influence)
    }

    /// Like [`ingest_cooccurrence`](Self::ingest_cooccurrence) but uses a
    /// named influence kind instead of the default.
    pub fn ingest_cooccurrence_with(
        &mut self,
        entries: Vec<(&str, &str, graph_core::Properties)>,
        influence: &str,
    ) -> super::config::StepObservation {
        let influence_id = self.resolve_influence(Some(influence));
        self.ingest_cooccurrence_with_influence(entries, influence_id)
    }

    /// Like [`ingest_cooccurrence`](Self::ingest_cooccurrence) but returns
    /// an error instead of panicking.
    pub fn try_ingest_cooccurrence(
        &mut self,
        entries: Vec<(&str, &str, graph_core::Properties)>,
    ) -> Result<super::config::StepObservation, IngestError> {
        let influence = self.try_resolve_influence(None)?;
        // Validate all kind names before mutating the world.
        for (_, kind, _) in &entries {
            self.try_resolve_locus_kind(kind)?;
        }
        Ok(self.ingest_cooccurrence_with_influence(entries, influence))
    }

    /// Internal implementation shared by all `ingest_cooccurrence*` variants.
    fn ingest_cooccurrence_with_influence(
        &mut self,
        entries: Vec<(&str, &str, graph_core::Properties)>,
        influence: graph_core::InfluenceKindId,
    ) -> super::config::StepObservation {
        // Step 1: create/merge loci (also queues to `pending_stimuli`).
        let ids: Vec<graph_core::LocusId> = entries
            .into_iter()
            .map(|(name, kind, props)| {
                let kind_id = self.resolve_locus_kind(kind);
                self.ingest(name, kind_id, influence, props)
            })
            .collect();

        // Step 2: snapshot the most-recent committed change for each locus.
        //         These may be `None` for brand-new loci (first occurrence).
        let last_change: Vec<Option<graph_core::ChangeId>> = {
            let world = self.world.read().unwrap();
            ids.iter()
                .map(|&id| world.log().changes_to_locus(id).next().map(|c| c.id))
                .collect()
        };

        // Step 3: build stimuli with cross-locus predecessors.
        //         Each locus names all other loci's last changes as predecessors,
        //         so the engine's cross-locus detection fires and auto-emerges
        //         relationships between loci that already existed.
        let stimuli: Vec<ProposedChange> = ids
            .iter()
            .enumerate()
            .map(|(i, &target_id)| {
                let predecessors: Vec<graph_core::ChangeId> = last_change
                    .iter()
                    .enumerate()
                    .filter_map(|(j, cid)| if j != i { *cid } else { None })
                    .collect();
                ProposedChange::activation(target_id, influence, 1.0)
                    .with_extra_predecessors(predecessors)
            })
            .collect();

        self.step(stimuli)
    }

    /// Encode properties to a StateVector using the encoder registered
    /// for `kind`, falling back to `PassthroughEncoder`.
    pub(crate) fn encode(
        &self,
        kind: graph_core::LocusKindId,
        properties: &graph_core::Properties,
    ) -> graph_core::StateVector {
        static PASSTHROUGH: graph_core::PassthroughEncoder = graph_core::PassthroughEncoder;
        let encoder = self.loci.encoder(kind).unwrap_or(&PASSTHROUGH);
        encoder.encode(properties)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use graph_core::props;
    use crate::simulation::builder::SimulationBuilder;
    use crate::registry::InfluenceKindConfig;
    use graph_core::{Change, Locus, LocusContext, LocusProgram, ProposedChange};

    struct NoopProgram;
    impl LocusProgram for NoopProgram {
        fn process(&self, _: &Locus, _: &[&Change], _: &dyn LocusContext) -> Vec<ProposedChange> { vec![] }
    }

    fn make_sim() -> crate::simulation::Simulation {
        SimulationBuilder::new()
            .locus_kind("NODE", NoopProgram)
            .influence("sig", |cfg: InfluenceKindConfig| cfg.with_decay(0.9).symmetric())
            .default_influence("sig")
            .build()
    }

    #[test]
    fn has_locus_kind_returns_correct() {
        let sim = make_sim();
        assert!(sim.has_locus_kind("NODE"));
        assert!(!sim.has_locus_kind("MISSING"));
    }

    #[test]
    fn has_influence_returns_correct() {
        let sim = make_sim();
        assert!(sim.has_influence("sig"));
        assert!(!sim.has_influence("nope"));
    }

    #[test]
    fn try_ingest_named_unknown_kind_returns_error() {
        let mut sim = make_sim();
        let err = sim.try_ingest_named("alice", "TYPO", props! {}).unwrap_err();
        assert_eq!(err, IngestError::UnknownLocusKind { name: "TYPO".to_owned() });
    }

    #[test]
    fn try_ingest_batch_named_validates_all_before_mutating() {
        let mut sim = make_sim();
        // "TYPO" is the second entry — the whole batch must fail without creating
        // the first locus.
        let err = sim.try_ingest_batch_named(vec![
            ("alice", "NODE", props! {}),
            ("bob",   "TYPO", props! {}),
        ]).unwrap_err();
        assert_eq!(err, IngestError::UnknownLocusKind { name: "TYPO".to_owned() });
        assert_eq!(sim.world().loci().len(), 0, "no loci should be created on error");
    }

    #[test]
    fn ingest_cooccurrence_creates_relationship_on_second_call() {
        let mut sim = make_sim();
        sim.ingest_cooccurrence(vec![
            ("alice", "NODE", props! {}),
            ("bob",   "NODE", props! {}),
        ]);
        assert_eq!(sim.world().relationships().len(), 0);

        sim.ingest_cooccurrence(vec![
            ("alice", "NODE", props! {}),
            ("bob",   "NODE", props! {}),
        ]);
        assert_eq!(sim.world().relationships().len(), 1);
    }

    #[test]
    fn try_ingest_cooccurrence_returns_error_on_unknown_kind() {
        let mut sim = make_sim();
        let err = sim.try_ingest_cooccurrence(vec![
            ("alice", "NODE",  props! {}),
            ("bob",   "TYPO",  props! {}),
        ]).unwrap_err();
        assert_eq!(err, IngestError::UnknownLocusKind { name: "TYPO".to_owned() });
        assert_eq!(sim.world().loci().len(), 0);
    }

    fn make_sim_with_capacity(cap: usize, policy: BackpressurePolicy) -> crate::simulation::Simulation {
        SimulationBuilder::new()
            .locus_kind("NODE", NoopProgram)
            .influence("sig", |c: InfluenceKindConfig| c.with_decay(0.9).symmetric())
            .default_influence("sig")
            .backpressure(cap, policy)
            .build()
    }

    #[test]
    fn backpressure_reject_drops_excess_stimuli() {
        let mut sim = make_sim_with_capacity(2, BackpressurePolicy::Reject);
        // First two ingests fit; third is rejected.
        sim.ingest_named("a", "NODE", props! {});
        sim.ingest_named("b", "NODE", props! {});
        sim.ingest_named("c", "NODE", props! {});
        assert_eq!(sim.pending_stimuli.len(), 2);
    }

    #[test]
    fn backpressure_drop_newest_same_as_reject() {
        let mut sim = make_sim_with_capacity(2, BackpressurePolicy::DropNewest);
        sim.ingest_named("a", "NODE", props! {});
        sim.ingest_named("b", "NODE", props! {});
        sim.ingest_named("c", "NODE", props! {});
        assert_eq!(sim.pending_stimuli.len(), 2);
    }

    #[test]
    fn backpressure_drop_oldest_rotates_queue() {
        let mut sim = make_sim_with_capacity(2, BackpressurePolicy::DropOldest);
        let id_a = sim.ingest_named("a", "NODE", props! {});
        let _id_b = sim.ingest_named("b", "NODE", props! {});
        let id_c = sim.ingest_named("c", "NODE", props! {});
        // Queue should now have b, c (a was dropped).
        assert_eq!(sim.pending_stimuli.len(), 2);
        assert!(
            sim.pending_stimuli.iter().any(|s| {
                if let graph_core::ChangeSubject::Locus(lid) = s.subject { lid == id_c } else { false }
            }),
            "newest stimulus (c) should be in the queue"
        );
        assert!(
            !sim.pending_stimuli.iter().any(|s| {
                if let graph_core::ChangeSubject::Locus(lid) = s.subject { lid == id_a } else { false }
            }),
            "oldest stimulus (a) should have been dropped"
        );
    }

    #[test]
    fn unbounded_capacity_never_drops() {
        let mut sim = make_sim(); // capacity = 0 = unbounded
        for i in 0..100 {
            sim.ingest_named(&format!("n{i}"), "NODE", props! {});
        }
        assert_eq!(sim.pending_stimuli.len(), 100);
    }
}
