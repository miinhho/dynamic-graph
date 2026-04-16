//! graph-boundary: tension analysis between the static and dynamic graph layers.
//!
//! The dynamic graph (`graph-world`) surfaces *observed* structure: which loci
//! actually influence each other, at what strength, and how that has changed
//! over time.
//!
//! The static graph (`graph-schema`) surfaces *declared* structure: what the
//! user *says* the world looks like, independent of observed behavior.
//!
//! The gap between them is where information lives.
//!
//! ## Four quadrants
//!
//! ```text
//!                  │  Dynamic active  │  Dynamic dormant / absent
//! ─────────────────┼──────────────────┼───────────────────────────
//! Declared (static)│  CONFIRMED       │  GHOST
//!   absent (static)│  SHADOW          │  NULL  (not reported)
//! ```
//!
//! - **Confirmed**: declared fact AND active dynamic relationship between the
//!   same loci. The world behaves as declared.
//! - **Ghost**: declared fact, but the dynamic relationship is absent or
//!   below the activity threshold. Declared structure is not behaviorally
//!   expressed.
//! - **Shadow**: active dynamic relationship with no declared counterpart.
//!   Undeclared influence — potentially important but invisible to the schema.
//!
//! ## Tension score
//!
//! `tension` is a scalar in `[0.0, 1.0]` summarising the overall divergence:
//!
//! ```text
//! tension = (ghosts + shadows) / (confirmed + ghosts + shadows).max(1)
//! ```
//!
//! A score of 0.0 means the static and dynamic worlds are perfectly aligned;
//! 1.0 means no overlap at all.

pub mod report;
pub mod analysis;
pub mod prescribe;
pub mod layer;

pub use report::{BoundaryEdge, BoundaryReport};
pub use analysis::{analyze_boundary, analyze_boundary_with_mode, SignalMode};
pub use prescribe::{BoundaryAction, RetractReason, PrescriptionConfig, prescribe_updates, apply_prescriptions};
pub use layer::{LayerTension, LayerReport, layer_tension};
