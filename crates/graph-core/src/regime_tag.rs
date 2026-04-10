//! Lightweight regime tag for cross-crate event reporting.
//!
//! The full `DynamicsRegime` with its classifier lives in graph-engine.
//! This tag is a Copy enum that graph-core can reference without
//! depending on the engine crate.

/// Tag identifying the dynamics regime. Mirrors `DynamicsRegime` in
/// graph-engine but lives in graph-core so `WorldEvent` can reference it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RegimeTag {
    Initializing,
    Quiescent,
    Settling,
    Equilibrium,
    Oscillating,
    LimitCycleSuspect,
    Diverging,
}
