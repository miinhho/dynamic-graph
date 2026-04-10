//! Regime classification and adaptive guard rail.
//!
//! `classifier` observes the engine's dynamical state and labels it as
//! one of the `DynamicsRegime` variants. `adaptive` uses that label to
//! scale stabilization alphas — only `Diverging` triggers tightening.

pub mod adaptive;
pub mod classifier;

pub use adaptive::{AdaptiveConfig, AdaptiveGuardRail};
pub use classifier::{
    BatchHistory, BatchMetrics, DefaultRegimeClassifier, DynamicsRegime, RegimeClassifier,
};
