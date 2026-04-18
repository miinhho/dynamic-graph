//! Regime classification and adaptive guard rail.
//!
//! `classifier` observes the engine's dynamical state and labels it as
//! one of the `DynamicsRegime` variants. `adaptive` uses that label to
//! scale stabilization alphas, while the generic learnable framework is
//! re-exported here for other per-kind controllers.

pub mod adaptive;
pub mod classifier;
mod types;

pub use adaptive::{AdaptiveConfig, AdaptiveGuardRail, Learnable, PerKindLearnable};
pub use classifier::{BatchHistory, BatchMetrics, DefaultRegimeClassifier};
pub use types::{DynamicsRegime, RegimeClassifier};
