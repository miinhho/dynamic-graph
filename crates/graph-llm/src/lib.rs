//! LLM-assisted ingestion and narration for `dynamic-graph-db`.
//!
//! ## Quick start — [`GraphLlm`] facade
//!
//! The simplest way to use this crate: construct [`GraphLlm`] once and call
//! the methods you need. All query pre-computation and name-resolution is
//! handled internally.
//!
//! ```ignore
//! use graph_llm::{GraphLlm, OllamaClient};
//!
//! let client = OllamaClient::new("llama3:8b");
//! let g      = GraphLlm::new(&client, &world);
//!
//! let answer = g.ask("Who is Alice connected to?")?;
//! let prose  = g.narrate_counterfactual(&root_changes)?;
//! let prose  = g.narrate_entity_deviations(baseline)?;
//! let prose  = g.narrate_schema_tension(&schema)?;
//! let nodes  = g.ingest("Marie Curie worked at...", &["PERSON", "ORG"])?;
//! ```
//!
//! ## Low-level free functions
//!
//! All capabilities are also available as free functions for cases where you
//! need more control over pre-computation.
//!
//! ### Text → Graph ingestion
//!
//! [`TextIngestor`] sends a natural-language document to an LLM and receives
//! back a list of named entities ([`ExtractedNode`]). Feed the result directly
//! into [`Simulation::ingest_cooccurrence`].
//!
//! ```no_run
//! # use graph_llm::{AnthropicClient, TextIngestor};
//! let client   = AnthropicClient::from_env().unwrap();
//! let ingestor = TextIngestor::new(&client);
//! let nodes    = ingestor.extract("Alice and Bob met at MIT.", &["PERSON", "ORG"]).unwrap();
//! ```
//!
//! ### Causal narration
//!
//! [`narrate_counterfactual`] and [`narrate_entity_deviations`] accept the
//! structured output of `graph-query` functions and return a prose explanation
//! for end-users.
//!
//! ```no_run
//! # use graph_llm::{AnthropicClient, narrate_counterfactual};
//! let pairs  = vec![("Alice".to_owned(), "Bob".to_owned())];
//! let client = AnthropicClient::from_env().unwrap();
//! let prose  = narrate_counterfactual(&client, &pairs).unwrap();
//! ```
//!
//! ### Schema-tension narration
//!
//! [`narrate_prescriptions`] translates [`graph_boundary::BoundaryAction`]
//! proposals into plain-language recommendations.
//!
//! ```no_run
//! # use graph_llm::{AnthropicClient, narrate_prescriptions};
//! # use graph_schema::SchemaWorld;
//! # use graph_query::NameMap;
//! # use graph_boundary::BoundaryAction;
//! let client  = AnthropicClient::from_env().unwrap();
//! let schema  = SchemaWorld::new();
//! let names   = NameMap::default();
//! let actions: Vec<BoundaryAction> = vec![];
//! let prose   = narrate_prescriptions(&client, &actions, &schema, &names).unwrap();
//! ```
//!
//! ## LLM backends
//!
//! All capabilities accept a `&dyn LlmClient`. The `anthropic` feature
//! (enabled by default) provides [`AnthropicClient`]. The `ollama` feature
//! provides [`OllamaClient`] for local models. Disable both to compile
//! without `reqwest` and supply your own client via the [`LlmClient`] trait.
//! [`MockLlmClient`] is always available for tests.

mod client;
mod error;
mod facade;
mod ingest;
mod narrate;
mod rag;
mod tension;

pub use client::{LlmClient, MockLlmClient};
pub use error::LlmError;
pub use facade::GraphLlm;
pub use ingest::{ExtractedNode, TextIngestor};
pub use narrate::{narrate_counterfactual, narrate_entity_deviations};
pub use rag::answer_with_graph;
pub use tension::narrate_prescriptions;

#[cfg(feature = "anthropic")]
pub use client::AnthropicClient;

#[cfg(feature = "ollama")]
pub use client::OllamaClient;
