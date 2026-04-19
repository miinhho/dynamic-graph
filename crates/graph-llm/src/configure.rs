//! LLM-assisted simulation parameter inference.
//!
//! Instead of hand-tuning decay rates, plasticity learning rates, and
//! threshold values, describe the desired behaviour in plain language and let
//! the LLM suggest appropriate numbers.
//!
//! Three entry points are provided:
//!
//! - [`configure_influence`] — infer `InfluenceKindConfig` (decay, plasticity,
//!   activity contribution, etc.) from a plain-language description.
//! - [`configure_emergence`] — infer `DefaultEmergencePerspective` thresholds
//!   (activity floor, entity-overlap sensitivity).
//! - [`configure_cohere`] — infer the `DefaultCoherePerspective` bridge
//!   threshold that decides when entity clusters merge.
//!
//! # Examples
//!
//! ```ignore
//! use graph_llm::{AnthropicClient, configure_influence, configure_emergence};
//!
//! let client = AnthropicClient::from_env()?;
//!
//! // "excitatory synapse that fades moderately and strengthens through use"
//! let signal_cfg = configure_influence(&client, "dopamine", "reward signal that
//!     decays slowly, saturates at moderate levels, and slightly strengthens
//!     used pathways")?;
//!
//! let emerge_cfg = configure_emergence(&client,
//!     "detect clusters where edges are at least weakly active; be lenient
//!      when matching new components to existing entities")?;
//! ```

use graph_engine::{
    DefaultCoherePerspective, DefaultEmergencePerspective, InfluenceKindConfig, PlasticityConfig,
};

use crate::client::LlmClient;
use crate::error::LlmError;

// ── System prompts ────────────────────────────────────────────────────────────

const INFLUENCE_SYSTEM: &str = r#"You are a parameter configurator for a dynamic graph simulation engine.
Given a natural-language description of a signal or influence type, return a JSON
object describing its behaviour. Include only the fields you want to set; omit
the rest (they will use sensible defaults).

Field reference (all optional):
{
  "retention_per_batch": <0.0–1.0>
      // Fraction of activity that SURVIVES to the next tick (what is KEPT, not lost).
      // This is the survival/retention ratio — a high value means slow decay.
      // 1.0 = nothing is ever lost (permanent).
      // 0.99 = 99% survives each tick (very slow fade, effective half-life ~69 ticks).
      // 0.90 = 90% survives each tick (slow fade, half-life ~7 ticks).
      // 0.70 = 70% survives each tick (moderate fade, half-life ~2 ticks).
      // 0.50 = 50% survives each tick (fast fade, half-life = 1 tick).
      // 0.10 = only 10% survives each tick (nearly instant erasure).
      // EXAMPLES: "permanent/persistent" → 0.97–1.0, "slow fade" → 0.85–0.97,
      //   "moderate fade" → 0.6–0.85, "fast fade/quick decay" → 0.3–0.6, "instant" → 0.0–0.2.

  "activity_contribution": <finite float>
      // Activity change per event. Positive = excitatory, negative = inhibitory.
      // Typical excitatory: 0.5–2.0. Typical inhibitory: -0.5 to -2.0.

  "min_emerge_activity": <≥0.0>
      // Minimum signal magnitude needed to auto-create a new relationship.
      // 0.0 = always emerge. "only strong signals create links" → 0.3–0.8.

  "max_activity": <>0 or null>
      // Activity ceiling. null = uncapped. "saturating" → 1.0–3.0.

  "prune_activity_threshold": <≥0.0>
      // Auto-remove relationship when activity drops below this. 0 = never prune.

  "learning_rate": <≥0.0>
      // Hebbian plasticity rate. 0 = no learning. Slow: 0.01–0.05.
      // Moderate: 0.05–0.15. Fast: 0.2–0.5.

  "weight_decay": <0.0–1.0>
      // Weight retained per batch (typical: 0.95–0.99). 1.0 = no weight decay.

  "max_weight": <>0>
      // Maximum learned weight. Typical: 1.0–2.0.
}

Return ONLY valid JSON. No markdown, no explanation, no code fences."#;

const EMERGENCE_SYSTEM: &str = r#"You are a parameter configurator for entity-emergence detection in a graph simulation.
Given a natural-language description of how sensitive detection should be, return a JSON
object with these optional fields:

{
  "min_activity_threshold": <0.0–1.0>
      // Minimum relationship activity to include in community detection.
      // 0.01 = include nearly everything, 0.1 = default (ignore very weak links),
      // 0.3 = only strong links, 0.5 = only dominant links.
}

Return ONLY valid JSON. No markdown, no explanation, no code fences."#;

const COHERE_SYSTEM: &str = r#"You are a parameter configurator for entity-cluster merging in a graph simulation.
Given a natural-language description of how loosely or tightly groups should merge,
return a JSON object with this optional field:

{
  "min_bridge_activity": <0.0–1.0>
      // Cross-entity relationship activity needed for two entities to be grouped
      // into a cohere cluster. 0.1 = merge loosely connected groups,
      // 0.3 = default (moderate bridging required), 0.6 = merge only tightly linked entities.
}

Return ONLY valid JSON. No markdown, no explanation, no code fences."#;

// ── JSON extraction ───────────────────────────────────────────────────────────

/// Extract the first `{…}` JSON object from a string, stripping markdown fences
/// and leading/trailing text that LLMs sometimes add despite the prompt.
fn extract_json(raw: &str) -> Result<serde_json::Value, LlmError> {
    let start = raw
        .find('{')
        .ok_or_else(|| LlmError::ParseError(format!("no JSON object in LLM response: {raw}")))?;
    let end = raw.rfind('}').ok_or_else(|| {
        LlmError::ParseError(format!("unclosed JSON object in LLM response: {raw}"))
    })?;
    serde_json::from_str(&raw[start..=end])
        .map_err(|e| LlmError::ParseError(format!("invalid JSON from LLM: {e} — raw: {raw}")))
}

/// Read an optional finite float from `json[key]`, returning `default` if absent.
fn opt_f32(json: &serde_json::Value, key: &str, default: f32) -> Result<f32, LlmError> {
    match json.get(key) {
        None | Some(serde_json::Value::Null) => Ok(default),
        Some(v) => v
            .as_f64()
            .map(|f| f as f32)
            .ok_or_else(|| LlmError::ParseError(format!("expected float for '{key}', got: {v}"))),
    }
}

/// Read an optional nullable float (maps JSON null → `None`).
fn opt_nullable_f32(json: &serde_json::Value, key: &str) -> Result<Option<f32>, LlmError> {
    match json.get(key) {
        None => Ok(None),
        Some(serde_json::Value::Null) => Ok(None),
        Some(v) => v.as_f64().map(|f| Some(f as f32)).ok_or_else(|| {
            LlmError::ParseError(format!("expected float or null for '{key}', got: {v}"))
        }),
    }
}

/// Read an optional bool, returning `default` if absent.
fn opt_bool(json: &serde_json::Value, key: &str, default: bool) -> Result<bool, LlmError> {
    match json.get(key) {
        None => Ok(default),
        Some(v) => v
            .as_bool()
            .ok_or_else(|| LlmError::ParseError(format!("expected bool for '{key}', got: {v}"))),
    }
}

// ── Public API ────────────────────────────────────────────────────────────────

/// Infer `InfluenceKindConfig` parameters from a natural-language description.
///
/// The LLM is asked to translate the description into numeric simulation
/// parameters. Fields the LLM omits keep their default values. The result is
/// a valid `InfluenceKindConfig` ready to register with
/// `InfluenceKindRegistry::insert`.
///
/// # Errors
///
/// Returns [`LlmError::ParseError`] if the LLM response cannot be interpreted as
/// valid JSON or contains out-of-range values.
///
/// # Example
///
/// ```ignore
/// let cfg = configure_influence(&client, "dopamine",
///     "reward signal: slow decay, saturates at moderate activity,
///      strengthens repeated pathways with moderate Hebbian learning")?;
/// sim_builder.influence(DOPAMINE, cfg);
/// ```
pub fn configure_influence(
    client: &dyn LlmClient,
    name: &str,
    description: &str,
) -> Result<InfluenceKindConfig, LlmError> {
    let raw = client.complete(INFLUENCE_SYSTEM, description)?;
    let json = extract_json(&raw)?;

    // Accept both field names for compatibility: "retention_per_batch" (preferred,
    // unambiguous) and "decay_per_batch" (legacy / direct field name).
    let decay = if json.get("retention_per_batch").is_some() {
        opt_f32(&json, "retention_per_batch", 1.0)?
    } else {
        opt_f32(&json, "decay_per_batch", 1.0)?
    };
    let activity_contrib = opt_f32(&json, "activity_contribution", 1.0)?;
    // Phase 5: min_emerge_activity / max_activity / prune_activity_threshold
    // removed. JSON keys are still parsed for back-compat but ignored.
    let _min_emerge = opt_f32(&json, "min_emerge_activity", 0.0)?;
    let _max_activity = opt_nullable_f32(&json, "max_activity")?;
    let _prune_threshold = opt_f32(&json, "prune_activity_threshold", 0.0)?;
    let learning_rate = opt_f32(&json, "learning_rate", 0.0)?;
    let weight_decay = opt_f32(&json, "weight_decay", 1.0)?;
    let max_weight = opt_f32(&json, "max_weight", f32::MAX)?;
    let _stdp = opt_bool(&json, "stdp", false)?;

    validate_range("decay_per_batch", decay, 0.0, 1.0, true)?;
    validate_positive("max_weight", max_weight)?;
    validate_range("weight_decay", weight_decay, 0.0, 1.0, true)?;
    validate_non_negative("learning_rate", learning_rate)?;

    let mut cfg = InfluenceKindConfig::new(name)
        .with_decay(decay)
        .with_activity_contribution(activity_contrib);

    if learning_rate > 0.0 {
        cfg = cfg.with_plasticity(PlasticityConfig {
            learning_rate,
            weight_decay,
            max_weight,
        });
    }

    Ok(cfg)
}

/// Infer `DefaultEmergencePerspective` thresholds from a natural-language description.
///
/// Since the locus-flow rewrite the perspective has a single knob:
/// - `min_activity_threshold` — which relationships are "active enough" to count
///
/// (The former `overlap_threshold` was removed when component-to-entity
/// reconciliation moved from Jaccard overlap to direct locus-flow analysis.)
pub fn configure_emergence(
    client: &dyn LlmClient,
    description: &str,
) -> Result<DefaultEmergencePerspective, LlmError> {
    let raw = client.complete(EMERGENCE_SYSTEM, description)?;
    let json = extract_json(&raw)?;

    let min_activity = opt_f32(&json, "min_activity_threshold", 0.1)?;
    validate_range("min_activity_threshold", min_activity, 0.0, 1.0, false)?;

    Ok(DefaultEmergencePerspective::default().with_min_activity_threshold(min_activity))
}

/// Infer `DefaultCoherePerspective` bridge threshold from a natural-language description.
///
/// Controls when entity clusters are merged into a cohere group. A lower
/// threshold groups loosely connected entities; a higher threshold requires
/// strong cross-entity activity.
///
/// # Example
///
/// ```ignore
/// let perspective = configure_cohere(&client,
///     "only merge entities that are strongly and directly interacting")?;
/// handle.extract_cohere(&perspective);
/// ```
pub fn configure_cohere(
    client: &dyn LlmClient,
    description: &str,
) -> Result<DefaultCoherePerspective, LlmError> {
    let raw = client.complete(COHERE_SYSTEM, description)?;
    let json = extract_json(&raw)?;

    let bridge = opt_f32(&json, "min_bridge_activity", 0.3)?;
    validate_range("min_bridge_activity", bridge, 0.0, 1.0, false)?;

    Ok(DefaultCoherePerspective::new("llm-configured").with_min_bridge_activity(bridge))
}

// ── Validation helpers ────────────────────────────────────────────────────────

fn validate_range(
    field: &str,
    v: f32,
    lo: f32,
    hi: f32,
    exclusive_lo: bool,
) -> Result<(), LlmError> {
    let in_range = if exclusive_lo { v > lo } else { v >= lo };
    if !in_range || v > hi {
        return Err(LlmError::ParseError(format!(
            "field '{field}' = {v} is out of range ({}{}–{hi})",
            if exclusive_lo { "(" } else { "[" },
            lo,
        )));
    }
    Ok(())
}

fn validate_positive(field: &str, v: f32) -> Result<(), LlmError> {
    if !v.is_finite() || v <= 0.0 {
        return Err(LlmError::ParseError(format!(
            "field '{field}' must be > 0, got {v}"
        )));
    }
    Ok(())
}

fn validate_non_negative(field: &str, v: f32) -> Result<(), LlmError> {
    if v < 0.0 {
        return Err(LlmError::ParseError(format!(
            "field '{field}' must be >= 0, got {v}"
        )));
    }
    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::MockLlmClient;

    #[test]
    fn configure_influence_full_json() {
        let json = r#"{
            "decay_per_batch": 0.85,
            "activity_contribution": 1.5,
            "learning_rate": 0.05,
            "weight_decay": 0.99,
            "max_weight": 1.0
        }"#;
        let client = MockLlmClient::new(json);
        let cfg = configure_influence(&client, "test", "excitatory synapse").unwrap();
        assert_eq!(cfg.decay_per_batch, 0.85);
        assert_eq!(cfg.activity_contribution, 1.5);
    }

    #[test]
    fn configure_influence_partial_json_uses_defaults() {
        let client = MockLlmClient::new(r#"{"decay_per_batch": 0.7}"#);
        let cfg = configure_influence(&client, "signal", "fast-fading signal").unwrap();
        assert_eq!(cfg.decay_per_batch, 0.7);
        assert_eq!(cfg.activity_contribution, 1.0);
    }

    #[test]
    fn configure_influence_inhibitory() {
        let client =
            MockLlmClient::new(r#"{"activity_contribution": -1.0, "decay_per_batch": 0.9}"#);
        let cfg = configure_influence(&client, "inhibitory", "inhibitory interneuron").unwrap();
        assert!(cfg.activity_contribution < 0.0);
    }

    #[test]
    fn configure_influence_rejects_out_of_range_decay() {
        let client = MockLlmClient::new(r#"{"decay_per_batch": 1.5}"#);
        let err = configure_influence(&client, "bad", "").unwrap_err();
        assert!(matches!(err, LlmError::ParseError(_)));
    }

    #[test]
    fn configure_influence_strips_markdown_fences() {
        // LLMs sometimes wrap JSON in markdown despite being asked not to.
        let client = MockLlmClient::new("```json\n{\"decay_per_batch\": 0.8}\n```");
        let cfg = configure_influence(&client, "signal", "moderate decay").unwrap();
        assert_eq!(cfg.decay_per_batch, 0.8);
    }

    #[test]
    fn configure_emergence_parses_threshold() {
        let client = MockLlmClient::new(r#"{"min_activity_threshold": 0.05}"#);
        configure_emergence(&client, "lenient detection").unwrap();
    }

    #[test]
    fn configure_emergence_empty_json_uses_defaults() {
        let client = MockLlmClient::new("{}");
        configure_emergence(&client, "default").unwrap();
    }

    #[test]
    fn configure_cohere_parses_bridge_threshold() {
        let client = MockLlmClient::new(r#"{"min_bridge_activity": 0.6}"#);
        configure_cohere(&client, "tight clusters only").unwrap();
    }

    #[test]
    fn configure_cohere_rejects_out_of_range() {
        let client = MockLlmClient::new(r#"{"min_bridge_activity": -0.1}"#);
        let err = configure_cohere(&client, "").unwrap_err();
        assert!(matches!(err, LlmError::ParseError(_)));
    }
}
