//! Entity layer weathering policy.
//!
//! Weathering controls how entity sediment layers erode over time.
//! Per `docs/identity.md` §5 (Memory and weathering), the framework
//! ships a sensible default but every layer can be replaced.
//!
//! ## Design
//!
//! The policy is a single method that maps `(layer, age_in_batches)`
//! to a `WeatheringEffect`. The engine applies the effect:
//!
//! - `Preserved` — layer is untouched.
//! - `Compress`  — layer snapshot is stripped; coherence + member count
//!   kept in a `CompressionLevel::Compressed` record.
//! - `Skeleton`  — further reduction; only transition kind + minimal
//!   stats remain (`CompressionLevel::Skeleton`).
//! - `Remove`    — layer is deleted. **The engine never removes a layer
//!   whose transition `is_significant()` (Born/Split/Merged)
//!   — it falls back to `Skeleton` for those.**
//!
//! Callers that need different semantics can implement the trait directly.

use crate::entity::{CompressedTransition, CompressionLevel, EntityLayer};

/// What the engine should do to a single entity layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WeatheringEffect {
    /// Keep the layer exactly as-is.
    Preserved,
    /// Strip the snapshot; store coherence + member count in a
    /// `Compressed` record. If the layer is already Compressed or
    /// Skeleton, this is a no-op.
    Compress,
    /// Further reduce to a `Skeleton` record. If already Skeleton,
    /// no-op.
    Skeleton,
    /// Delete the layer entirely. The engine overrides this to
    /// `Skeleton` for layers whose transition `is_significant()`.
    Remove,
}

/// Policy for deciding how to weather a single entity layer.
pub trait EntityWeatheringPolicy: Send + Sync {
    /// Return the effect to apply to `layer`, which is `age_in_batches`
    /// old (i.e. `current_batch - layer.batch`).
    fn effect(&self, layer: &EntityLayer, age_in_batches: u64) -> WeatheringEffect;
}

/// Strip a `Full` layer's snapshot and return `(coherence, member_count,
/// transition_kind)`. Shared by `apply_compress` and `apply_skeleton`.
fn strip_full(layer: &mut EntityLayer) -> (f32, u32, CompressedTransition) {
    let (coherence, member_count) = layer
        .snapshot
        .as_ref()
        .map(|s| (s.coherence, s.members.len() as u32))
        .unwrap_or((0.0, 0));
    let kind = CompressedTransition::from(&layer.transition);
    layer.snapshot = None;
    (coherence, member_count, kind)
}

/// Apply a `Compress` effect to a layer in-place.
///
/// Exposed so callers can compress on demand outside the engine's batch
/// loop. No-op if the layer is already `Compressed` or `Skeleton`.
pub fn apply_compress(layer: &mut EntityLayer) {
    match layer.compression {
        CompressionLevel::Full => {
            let (coherence, member_count, transition_kind) = strip_full(layer);
            layer.compression = CompressionLevel::Compressed {
                coherence,
                member_count,
                transition_kind,
            };
        }
        CompressionLevel::Compressed { .. } | CompressionLevel::Skeleton { .. } => {}
    }
}

/// Apply a `Skeleton` effect to a layer in-place.
///
/// Reduces a `Full` or `Compressed` layer to the minimal `Skeleton`
/// representation. No-op if already `Skeleton`.
pub fn apply_skeleton(layer: &mut EntityLayer) {
    match &layer.compression {
        CompressionLevel::Full => {
            let (coherence, member_count, transition_kind) = strip_full(layer);
            layer.compression = CompressionLevel::Skeleton {
                coherence,
                member_count,
                transition_kind,
            };
        }
        CompressionLevel::Compressed {
            coherence,
            member_count,
            transition_kind,
        } => {
            layer.compression = CompressionLevel::Skeleton {
                coherence: *coherence,
                member_count: *member_count,
                transition_kind: *transition_kind,
            };
        }
        CompressionLevel::Skeleton { .. } => {}
    }
}

// ─── Default implementation ───────────────────────────────────────────────

/// The default three-window weathering policy.
///
/// | Age range                             | Effect    |
/// |---------------------------------------|-----------|
/// | `0 .. recent_window`                  | Preserved |
/// | `recent_window .. compression_age`    | Compress  |
/// | `compression_age .. removal_age`      | Skeleton  |
/// | `>= removal_age`                      | Remove    |
///
/// Significant transitions (Born/Split/Merged) are **never** removed
/// by the engine — they fall back to `Skeleton` regardless.
///
/// Phase 6: the `recent_window` / `compression_age` / `removal_age` /
/// `preserved_transitions` knobs were collapsed into hard-coded constants.
/// No benchmark required non-default values. Custom policies are still
/// possible via `impl EntityWeatheringPolicy for MyPolicy`.
#[derive(Debug, Clone, Copy, Default)]
pub struct DefaultEntityWeathering;

const RECENT_WINDOW: u64 = 50;
const COMPRESSION_AGE: u64 = 200;
const REMOVAL_AGE: u64 = 1000;

impl EntityWeatheringPolicy for DefaultEntityWeathering {
    fn effect(&self, _layer: &EntityLayer, age_in_batches: u64) -> WeatheringEffect {
        if age_in_batches < RECENT_WINDOW {
            WeatheringEffect::Preserved
        } else if age_in_batches < COMPRESSION_AGE {
            WeatheringEffect::Compress
        } else if age_in_batches < REMOVAL_AGE {
            WeatheringEffect::Skeleton
        } else {
            WeatheringEffect::Remove
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::{EntityLayer, EntitySnapshot, LayerTransition};
    use crate::ids::BatchId;

    fn full_layer(batch: u64) -> EntityLayer {
        EntityLayer::new(
            BatchId(batch),
            EntitySnapshot {
                members: vec![crate::LocusId(1), crate::LocusId(2)],
                member_relationships: Vec::new(),
                coherence: 0.8,
            },
            LayerTransition::MembershipDelta {
                added: Vec::new(),
                removed: Vec::new(),
            },
        )
    }

    #[test]
    fn default_policy_recent_is_preserved() {
        let policy = DefaultEntityWeathering;
        let layer = full_layer(90);
        assert_eq!(policy.effect(&layer, 10), WeatheringEffect::Preserved);
    }

    #[test]
    fn default_policy_mid_age_is_compress() {
        let policy = DefaultEntityWeathering;
        let layer = full_layer(0);
        assert_eq!(policy.effect(&layer, 100), WeatheringEffect::Compress);
    }

    #[test]
    fn default_policy_old_is_skeleton() {
        let policy = DefaultEntityWeathering;
        let layer = full_layer(0);
        assert_eq!(policy.effect(&layer, 500), WeatheringEffect::Skeleton);
    }

    #[test]
    fn default_policy_ancient_is_remove() {
        let policy = DefaultEntityWeathering;
        let layer = full_layer(0);
        assert_eq!(policy.effect(&layer, 2000), WeatheringEffect::Remove);
    }

    #[test]
    fn apply_compress_strips_snapshot_keeps_stats() {
        let mut layer = full_layer(0);
        apply_compress(&mut layer);
        assert!(layer.snapshot.is_none());
        match &layer.compression {
            CompressionLevel::Compressed {
                coherence,
                member_count,
                transition_kind,
            } => {
                assert!((*coherence - 0.8).abs() < 1e-6);
                assert_eq!(*member_count, 2);
                assert_eq!(*transition_kind, CompressedTransition::MembershipDelta);
            }
            other => panic!("expected Compressed, got {other:?}"),
        }
    }

    #[test]
    fn apply_skeleton_further_reduces_compressed() {
        let mut layer = full_layer(0);
        apply_compress(&mut layer);
        apply_skeleton(&mut layer);
        assert!(matches!(
            layer.compression,
            CompressionLevel::Skeleton { .. }
        ));
    }

    #[test]
    fn apply_compress_is_idempotent_on_compressed() {
        let mut layer = full_layer(0);
        apply_compress(&mut layer);
        let before = layer.compression.clone();
        apply_compress(&mut layer);
        assert_eq!(layer.compression, before);
    }

    #[test]
    fn apply_skeleton_is_idempotent_on_skeleton() {
        let mut layer = full_layer(0);
        apply_skeleton(&mut layer);
        let before = layer.compression.clone();
        apply_skeleton(&mut layer);
        assert_eq!(layer.compression, before);
    }
}
