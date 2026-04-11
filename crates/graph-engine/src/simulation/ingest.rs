//! High-level domain-data ingestion API.
//!
//! Users work with names, properties, and relationship labels; the ingest
//! layer handles LocusId allocation, PropertyStore/NameIndex bookkeeping,
//! encoding, and stimulus generation.

use graph_core::ProposedChange;

use super::Simulation;
use super::config::StepObservation;

impl Simulation {
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

        let locus_id = if let Some(existing) = self.world.names().resolve(name) {
            if let Some(props) = self.world.properties_mut().get_mut(existing) {
                props.extend(&properties);
            }
            existing
        } else {
            let id = self.world.loci().next_id();
            let locus = graph_core::Locus::new(id, kind, state.clone());
            self.world.insert_locus(locus);
            self.world.names_mut().insert(name, id);
            self.world.properties_mut().insert(id, properties);
            id
        };

        self.pending_stimuli.push(ProposedChange::new(
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
        self.world.names().resolve(name)
    }

    /// Get the domain properties for a locus.
    pub fn properties_of(&self, id: graph_core::LocusId) -> Option<&graph_core::Properties> {
        self.world.properties().get(id)
    }

    /// Get the canonical name for a locus.
    pub fn name_of(&self, id: graph_core::LocusId) -> Option<&str> {
        self.world.names().name_of(id)
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

    /// Resolve a locus kind name to its `LocusKindId`.
    pub(crate) fn resolve_locus_kind(&self, name: &str) -> graph_core::LocusKindId {
        *self.locus_kind_names.get(name).unwrap_or_else(|| {
            panic!("unknown locus kind \"{name}\" — register it via SimulationBuilder::locus_kind() or register_locus_kind_name()")
        })
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
