//! Shared regime concepts used by classification and adaptive control.

use crate::regime::BatchHistory;

/// Observed dynamical regime of the system at a given point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DynamicsRegime {
    /// Not enough history to classify yet.
    Initializing,
    /// Energy is decreasing monotonically toward zero.
    Settling,
    /// Energy is near zero; nothing significant is happening.
    Quiescent,
    /// Energy oscillates. This is a valid observation mode.
    Oscillating,
    /// Same period-2 (or longer) pattern repeats.
    LimitCycleSuspect,
    /// Energy is increasing without bound.
    Diverging,
}

impl DynamicsRegime {
    pub fn to_tag(self) -> graph_core::RegimeTag {
        match self {
            DynamicsRegime::Initializing => graph_core::RegimeTag::Initializing,
            DynamicsRegime::Settling => graph_core::RegimeTag::Settling,
            DynamicsRegime::Quiescent => graph_core::RegimeTag::Quiescent,
            DynamicsRegime::Oscillating => graph_core::RegimeTag::Oscillating,
            DynamicsRegime::LimitCycleSuspect => graph_core::RegimeTag::LimitCycleSuspect,
            DynamicsRegime::Diverging => graph_core::RegimeTag::Diverging,
        }
    }
}

impl std::fmt::Display for DynamicsRegime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DynamicsRegime::Initializing => write!(f, "Initializing"),
            DynamicsRegime::Settling => write!(f, "Settling"),
            DynamicsRegime::Quiescent => write!(f, "Quiescent"),
            DynamicsRegime::Oscillating => write!(f, "Oscillating"),
            DynamicsRegime::LimitCycleSuspect => write!(f, "LimitCycleSuspect"),
            DynamicsRegime::Diverging => write!(f, "Diverging"),
        }
    }
}

/// Classifies the dynamical regime from a `BatchHistory`.
pub trait RegimeClassifier: Send + Sync {
    fn classify(&self, history: &BatchHistory) -> DynamicsRegime;
}
