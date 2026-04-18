//! High-level facade: [`GraphLlm`].
//!
//! [`GraphLlm`] bundles an LLM client and a world reference so you don't have
//! to thread them through every call. It also handles the boilerplate of
//! building a [`NameMap`] and running the intermediate query-layer steps that
//! the low-level free functions require.
//!
//! ```ignore
//! use graph_llm::{GraphLlm, OllamaClient};
//!
//! let client = OllamaClient::new("llama3:8b");
//! let g      = GraphLlm::new(&client, &world);
//!
//! // Q&A grounded in the graph
//! let answer = g.ask("Who is Alice connected to?")?;
//!
//! // Causal narration — just pass the root change IDs
//! let prose = g.narrate_counterfactual(&root_changes)?;
//!
//! // Entity deviation narration — just pass the baseline batch
//! let prose = g.narrate_entity_deviations(baseline)?;
//!
//! // Schema tension narration — just pass the schema
//! let prose = g.narrate_schema_tension(&schema)?;
//!
//! // Text → graph ingestion
//! let nodes = g.ingest("Marie Curie worked at...", &["PERSON", "ORG"])?;
//! ```

use graph_boundary::{PrescriptionConfig, analyze_boundary, prescribe_updates};
use graph_core::{BatchId, ChangeId, Endpoints};
use graph_engine::{DefaultCoherePerspective, DefaultEmergencePerspective, InfluenceKindConfig};
use graph_query::{NameMap, entity_deviations_since, relationships_absent_without};
use graph_schema::SchemaWorld;
use graph_world::World;

use crate::client::LlmClient;
use crate::configure;
use crate::error::LlmError;
use crate::ingest::{ExtractedNode, TextIngestor};

// ─── Facade ───────────────────────────────────────────────────────────────────

/// High-level entry point for LLM-assisted graph operations.
///
/// Construct once with [`GraphLlm::new`], then call the methods you need.
/// The [`NameMap`] is built from the world at construction time and reused
/// for the lifetime of the facade.
///
/// Use [`GraphLlm::with_top_k`] to control how many graph facts are injected
/// into Q&A prompts (default: 5).
pub struct GraphLlm<'a> {
    client: &'a dyn LlmClient,
    world: &'a World,
    names: NameMap,
    top_k: usize,
}

impl<'a> GraphLlm<'a> {
    /// Create a facade for `world`, using `client` for all LLM calls.
    ///
    /// The [`NameMap`] is built eagerly from the current world state.
    pub fn new(client: &'a dyn LlmClient, world: &'a World) -> Self {
        Self {
            client,
            world,
            names: NameMap::from_world(world),
            top_k: 5,
        }
    }

    /// Override the maximum number of graph facts injected into Q&A prompts.
    pub fn with_top_k(mut self, top_k: usize) -> Self {
        self.top_k = top_k;
        self
    }

    // ── Q&A ──────────────────────────────────────────────────────────────────

    /// Answer `question` using the graph as a targeted information source.
    ///
    /// Entities mentioned in the question are matched against the [`NameMap`],
    /// and their strongest `top_k` relationships are injected as context.
    pub fn ask(&self, question: &str) -> Result<String, LlmError> {
        crate::rag::answer_with_graph(self.client, question, self.world, &self.names, self.top_k)
    }

    // ── Narration ─────────────────────────────────────────────────────────────

    /// Explain which relationships would not exist without `root_changes`.
    ///
    /// Runs [`relationships_absent_without`] internally — you only need to
    /// supply the root [`ChangeId`]s (e.g. from `world.log().batch(id)`).
    pub fn narrate_counterfactual(&self, root_changes: &[ChangeId]) -> Result<String, LlmError> {
        let absent = relationships_absent_without(self.world, root_changes);
        let pairs: Vec<(String, String)> = absent
            .iter()
            .filter_map(|&id| self.world.relationships().get(id))
            .map(|rel| {
                let (a, b) = match rel.endpoints {
                    Endpoints::Symmetric { a, b } => (a, b),
                    Endpoints::Directed { from, to } => (from, to),
                };
                (self.names.name(a), self.names.name(b))
            })
            .collect();
        crate::narrate::narrate_counterfactual(self.client, &pairs)
    }

    /// Summarise how entities have changed since `baseline`.
    ///
    /// Runs [`entity_deviations_since`] internally — you only need to supply
    /// the baseline [`BatchId`].
    pub fn narrate_entity_deviations(&self, baseline: BatchId) -> Result<String, LlmError> {
        let diffs = entity_deviations_since(self.world, baseline);
        crate::narrate::narrate_entity_deviations(self.client, &diffs, &self.names)
    }

    /// Translate schema-update proposals into plain-language recommendations.
    ///
    /// Runs [`analyze_boundary`] and [`prescribe_updates`] internally — you
    /// only need to supply the [`SchemaWorld`].
    pub fn narrate_schema_tension(&self, schema: &SchemaWorld) -> Result<String, LlmError> {
        let report = analyze_boundary(self.world, schema, None);
        let actions =
            prescribe_updates(&report, schema, self.world, &PrescriptionConfig::default());
        crate::tension::narrate_prescriptions(self.client, &actions, schema, &self.names)
    }

    // ── Ingestion ─────────────────────────────────────────────────────────────

    /// Extract named entities from `text`.
    ///
    /// `kinds` filters which entity types to extract (e.g. `&["PERSON", "ORG"]`).
    /// Feed the returned [`ExtractedNode`]s into `Simulation::ingest_cooccurrence`.
    pub fn ingest(&self, text: &str, kinds: &[&str]) -> Result<Vec<ExtractedNode>, LlmError> {
        TextIngestor::new(self.client).extract(text, kinds)
    }

    // ── Parameter configuration ───────────────────────────────────────────────

    /// Infer `InfluenceKindConfig` parameters from a plain-language description.
    ///
    /// Translates natural-language descriptions of signal behaviour into
    /// numeric simulation parameters (decay rate, activity contribution,
    /// Hebbian plasticity, etc.), so you don't have to hand-tune them.
    ///
    /// ```ignore
    /// let cfg = g.configure_influence("glutamate",
    ///     "fast excitatory neurotransmitter: decays quickly, does not
    ///      saturate, and slightly strengthens repeated pathways")?;
    /// sim_builder.influence(GLUTAMATE, cfg);
    /// ```
    pub fn configure_influence(
        &self,
        name: &str,
        description: &str,
    ) -> Result<InfluenceKindConfig, LlmError> {
        configure::configure_influence(self.client, name, description)
    }

    /// Infer `DefaultEmergencePerspective` thresholds from a plain-language description.
    ///
    /// Controls the activity floor and entity-matching sensitivity used by
    /// [`EngineHandle::recognize_entities`].
    ///
    /// ```ignore
    /// let perspective = g.configure_emergence(
    ///     "detect clusters even when edges are only faintly active;
    ///      allow entities to drift considerably between ticks")?;
    /// handle.recognize_entities(&perspective);
    /// ```
    pub fn configure_emergence(
        &self,
        description: &str,
    ) -> Result<DefaultEmergencePerspective, LlmError> {
        configure::configure_emergence(self.client, description)
    }

    /// Infer `DefaultCoherePerspective` bridge threshold from a plain-language description.
    ///
    /// Controls when entity clusters are merged into a single cohere group by
    /// [`EngineHandle::extract_cohere`].
    ///
    /// ```ignore
    /// let perspective = g.configure_cohere(
    ///     "only merge entity clusters that are strongly and directly interacting")?;
    /// handle.extract_cohere(&perspective);
    /// ```
    pub fn configure_cohere(
        &self,
        description: &str,
    ) -> Result<DefaultCoherePerspective, LlmError> {
        configure::configure_cohere(self.client, description)
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::MockLlmClient;

    #[test]
    fn ask_on_empty_world_returns_llm_response() {
        let client = MockLlmClient::new("No one.");
        let world = World::new();
        let g = GraphLlm::new(&client, &world);
        assert_eq!(g.ask("Who is Alice?").unwrap(), "No one.");
    }

    #[test]
    fn narrate_entity_deviations_empty_returns_canned() {
        let client = MockLlmClient::new("should not be called");
        let world = World::new();
        let g = GraphLlm::new(&client, &world);
        let prose = g.narrate_entity_deviations(BatchId(0)).unwrap();
        assert!(prose.contains("No entity"), "{prose}");
    }

    #[test]
    fn narrate_counterfactual_empty_returns_canned() {
        let client = MockLlmClient::new("should not be called");
        let world = World::new();
        let g = GraphLlm::new(&client, &world);
        let prose = g.narrate_counterfactual(&[]).unwrap();
        assert!(prose.contains("No relationships"), "{prose}");
    }
}
