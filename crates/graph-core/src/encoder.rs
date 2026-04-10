//! Encoder trait: domain properties → engine `StateVector`.
//!
//! Each `LocusKind` can have its own `Encoder` that maps `Properties`
//! (strings, numbers, metadata from NER or other pipelines) into the
//! `StateVector` consumed by the engine's dynamics (decay, stabilization,
//! plasticity).
//!
//! The user defines encoding rules; the engine handles everything after.

use crate::property::Properties;
use crate::state::StateVector;

/// Converts domain `Properties` into a `StateVector` for the engine.
///
/// Implement this trait per locus kind to control how domain data maps
/// to the numerical substrate. The engine calls `encode()` during
/// ingestion to produce the stimulus `StateVector`.
pub trait Encoder: Send + Sync {
    /// Map domain properties to a `StateVector`.
    ///
    /// Called once per ingestion event. The returned vector becomes the
    /// `after` field of the `ProposedChange` (stimulus).
    fn encode(&self, properties: &Properties) -> StateVector;

    /// Whether two co-occurring entities should form a relationship.
    ///
    /// Called for each pair of entities ingested in the same batch
    /// (e.g. extracted from the same document). Return `false` to
    /// suppress the default co-occurrence relationship.
    ///
    /// Default: always `true`.
    fn should_relate(&self, _source: &Properties, _target: &Properties) -> bool {
        true
    }
}

/// Pass-through encoder: looks for a `"_state"` property containing
/// a list of floats, or falls back to a single-slot vector using the
/// `"confidence"` property (default 0.0).
///
/// Useful for testing or when the user pre-computes state vectors
/// outside the system.
#[derive(Debug, Clone, Default)]
pub struct PassthroughEncoder;

impl Encoder for PassthroughEncoder {
    fn encode(&self, properties: &Properties) -> StateVector {
        use crate::property::PropertyValue;

        // Try explicit _state list first.
        if let Some(PropertyValue::List(items)) = properties.get("_state") {
            let vals: Vec<f32> = items
                .iter()
                .filter_map(|v| match v {
                    PropertyValue::Float(f) => Some(*f as f32),
                    PropertyValue::Int(i) => Some(*i as f32),
                    _ => None,
                })
                .collect();
            if !vals.is_empty() {
                return StateVector::from_slice(&vals);
            }
        }

        // Fallback: single confidence slot.
        let confidence = properties.get_f32("confidence").unwrap_or(0.0);
        StateVector::from_slice(&[confidence])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::property::PropertyValue;
    use crate::props;

    #[test]
    fn passthrough_uses_confidence() {
        let enc = PassthroughEncoder;
        let p = props! { "confidence" => 0.85_f64 };
        let sv = enc.encode(&p);
        assert!((sv.as_slice()[0] - 0.85).abs() < 1e-5);
    }

    #[test]
    fn passthrough_uses_explicit_state() {
        let enc = PassthroughEncoder;
        let mut p = Properties::new();
        p.set(
            "_state",
            vec![PropertyValue::Float(1.0), PropertyValue::Float(2.0)],
        );
        let sv = enc.encode(&p);
        assert_eq!(sv.as_slice(), &[1.0, 2.0]);
    }

    #[test]
    fn passthrough_defaults_to_zero() {
        let enc = PassthroughEncoder;
        let p = props! { "name" => "test" };
        let sv = enc.encode(&p);
        assert_eq!(sv.as_slice(), &[0.0]);
    }
}
