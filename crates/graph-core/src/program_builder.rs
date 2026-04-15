//! Declarative builder for [`LocusProgram`] implementations.
//!
//! Writing a full `LocusProgram` impl for simple forwarding or accumulation
//! patterns requires boilerplate. [`ProgramBuilder`] lets you assemble a
//! program from named high-level rules and raw closures instead:
//!
//! ```rust,ignore
//! use graph_core::{InfluenceKindId, LocusId, RelationshipId, ProgramBuilder};
//!
//! const FIRE: InfluenceKindId = InfluenceKindId(1);
//! const REL: RelationshipId   = RelationshipId(3);
//!
//! let program = ProgramBuilder::new()
//!     .forward(LocusId(5), FIRE, 0.8)          // scale and forward to locus 5
//!     .accumulate(FIRE, 0.1)                   // also self-accumulate
//!     .subscribe_initial(&[REL])               // watch this relationship from batch 0
//!     .build();
//! ```
//!
//! The resulting [`ComposedProgram`] implements [`LocusProgram`]. Named rules
//! apply a noise floor of `0.001` — outputs below that threshold are not
//! proposed, preventing spurious tiny activations from cascading.

use crate::change::Change;
use crate::ids::{InfluenceKindId, LocusId};
use crate::locus::Locus;
use crate::program::{LocusContext, LocusProgram, ProposedChange, StructuralProposal};
use crate::relationship::RelationshipId;

/// Minimum absolute output below which named rule results are suppressed.
const NOISE_FLOOR: f32 = 0.001;

// Type aliases for the closure shapes stored inside `ComposedProgram`.
type ProcessFn =
    Box<dyn Fn(&Locus, &[&Change], &dyn LocusContext) -> Vec<ProposedChange> + Send + Sync>;
type StructuralFn =
    Box<dyn Fn(&Locus, &[&Change], &dyn LocusContext) -> Vec<StructuralProposal> + Send + Sync>;

// ─── ComposedProgram ─────────────────────────────────────────────────────────

/// A [`LocusProgram`] assembled from zero or more process and structural rules.
///
/// Created via [`ProgramBuilder::build`]. The `process` and
/// `structural_proposals` methods iterate each rule in insertion order and
/// flatten the results. Rules are independent — they do not see each other's
/// outputs within the same batch.
pub struct ComposedProgram {
    process_rules: Vec<ProcessFn>,
    structural_rules: Vec<StructuralFn>,
    initial_subs: Vec<RelationshipId>,
}

impl LocusProgram for ComposedProgram {
    fn process(
        &self,
        locus: &Locus,
        incoming: &[&Change],
        ctx: &dyn LocusContext,
    ) -> Vec<ProposedChange> {
        let mut out = Vec::new();
        for rule in &self.process_rules {
            out.extend(rule(locus, incoming, ctx));
        }
        out
    }

    fn structural_proposals(
        &self,
        locus: &Locus,
        incoming: &[&Change],
        ctx: &dyn LocusContext,
    ) -> Vec<StructuralProposal> {
        let mut out = Vec::new();
        for rule in &self.structural_rules {
            out.extend(rule(locus, incoming, ctx));
        }
        out
    }

    fn initial_subscriptions(&self, _locus: &Locus) -> Vec<RelationshipId> {
        self.initial_subs.clone()
    }
}

// ─── ProgramBuilder ──────────────────────────────────────────────────────────

/// Builder for [`ComposedProgram`].
///
/// Rules are stored in insertion order and executed in that order. Prefer the
/// named convenience methods (`forward`, `broadcast`, `accumulate`) for common
/// patterns; use `on_process` / `on_structural` for custom logic.
pub struct ProgramBuilder {
    process_rules: Vec<ProcessFn>,
    structural_rules: Vec<StructuralFn>,
    initial_subs: Vec<RelationshipId>,
}

impl Default for ProgramBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl ProgramBuilder {
    /// Create a new empty builder.
    pub fn new() -> Self {
        Self {
            process_rules: Vec::new(),
            structural_rules: Vec::new(),
            initial_subs: Vec::new(),
        }
    }

    /// Add a raw process rule.
    ///
    /// `rule` receives the same `(locus, incoming, ctx)` arguments as
    /// [`LocusProgram::process`] and returns zero or more [`ProposedChange`]s.
    /// Use the named convenience methods (`forward`, `broadcast`, `accumulate`)
    /// for the common patterns.
    pub fn on_process<F>(mut self, rule: F) -> Self
    where
        F: Fn(&Locus, &[&Change], &dyn LocusContext) -> Vec<ProposedChange>
            + Send
            + Sync
            + 'static,
    {
        self.process_rules.push(Box::new(rule));
        self
    }

    /// Add a raw structural rule.
    ///
    /// `rule` receives the same `(locus, incoming, ctx)` arguments as
    /// [`LocusProgram::structural_proposals`] and returns zero or more
    /// [`StructuralProposal`]s.
    pub fn on_structural<F>(mut self, rule: F) -> Self
    where
        F: Fn(&Locus, &[&Change], &dyn LocusContext) -> Vec<StructuralProposal>
            + Send
            + Sync
            + 'static,
    {
        self.structural_rules.push(Box::new(rule));
        self
    }

    /// Forward the locus's slot-0 state, scaled by `gain`, to `to` using
    /// influence kind `kind`.
    ///
    /// The scaled value is proposed as an [`activation`][ProposedChange::activation]
    /// only when its absolute value exceeds the noise floor (0.001). Mirrors
    /// the semantics of `graph_testkit::ForwardProgram`.
    pub fn forward(self, to: LocusId, kind: InfluenceKindId, gain: f32) -> Self {
        self.on_process(move |locus, _incoming, _ctx| {
            let signal = locus.state.as_slice().first().copied().unwrap_or(0.0) * gain;
            if signal.abs() < NOISE_FLOOR {
                return Vec::new();
            }
            vec![ProposedChange::activation(to, kind, signal)]
        })
    }

    /// Fan the locus's slot-0 state, scaled by `gain`, out to every locus in
    /// `targets` using influence kind `kind`.
    ///
    /// Each target receives an independent [`activation`][ProposedChange::activation]
    /// proposal. Targets whose scaled signal falls below the noise floor are
    /// silently skipped. Mirrors `graph_testkit::BroadcastProgram`.
    pub fn broadcast(self, targets: Vec<LocusId>, kind: InfluenceKindId, gain: f32) -> Self {
        self.on_process(move |locus, _incoming, _ctx| {
            let signal = locus.state.as_slice().first().copied().unwrap_or(0.0) * gain;
            if signal.abs() < NOISE_FLOOR {
                return Vec::new();
            }
            targets
                .iter()
                .map(|&t| ProposedChange::activation(t, kind, signal))
                .collect()
        })
    }

    /// Self-accumulate the locus's slot-0 state, scaled by `gain`.
    ///
    /// Proposes an activation back to the locus itself — the engine will add
    /// `current * gain` on top of the locus's existing state. Subject to the
    /// noise floor. Mirrors `graph_testkit::AccumulatorProgram`.
    pub fn accumulate(self, kind: InfluenceKindId, gain: f32) -> Self {
        self.on_process(move |locus, _incoming, _ctx| {
            let current = locus.state.as_slice().first().copied().unwrap_or(0.0);
            let signal = current * gain;
            if signal.abs() < NOISE_FLOOR {
                return Vec::new();
            }
            vec![ProposedChange::activation(locus.id, kind, signal)]
        })
    }

    /// Declare relationships this locus should subscribe to from the very
    /// first batch, before any changes have been committed.
    ///
    /// Equivalent to overriding [`LocusProgram::initial_subscriptions`].
    /// Multiple calls accumulate rather than replace the subscription list.
    pub fn subscribe_initial(mut self, rel_ids: &[RelationshipId]) -> Self {
        self.initial_subs.extend_from_slice(rel_ids);
        self
    }

    /// Consume the builder and produce a [`ComposedProgram`].
    pub fn build(self) -> ComposedProgram {
        ComposedProgram {
            process_rules: self.process_rules,
            structural_rules: self.structural_rules,
            initial_subs: self.initial_subs,
        }
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::change::ChangeSubject;
    use crate::ids::{InfluenceKindId, LocusId, LocusKindId};
    use crate::relationship::RelationshipId;
    use crate::locus::Locus;
    use crate::relationship::Relationship;
    use crate::state::StateVector;

    // Minimal no-op LocusContext for tests.
    struct NullCtx;

    impl LocusContext for NullCtx {
        fn locus(&self, _id: LocusId) -> Option<&Locus> {
            None
        }

        fn relationships_for<'a>(
            &'a self,
            _locus: LocusId,
        ) -> Box<dyn Iterator<Item = &'a Relationship> + 'a> {
            Box::new(std::iter::empty())
        }
    }

    fn make_locus(id: u64, state0: f32) -> Locus {
        Locus::new(LocusId(id), LocusKindId(0), StateVector::from_slice(&[state0]))
    }

    const KIND: InfluenceKindId = InfluenceKindId(1);

    // ── empty ──────────────────────────────────────────────────────────────────

    #[test]
    fn empty_program_produces_nothing() {
        let prog = ProgramBuilder::new().build();
        let locus = make_locus(0, 0.5);
        assert!(prog.process(&locus, &[], &NullCtx).is_empty());
        assert!(prog.structural_proposals(&locus, &[], &NullCtx).is_empty());
        assert!(prog.initial_subscriptions(&locus).is_empty());
    }

    // ── forward ───────────────────────────────────────────────────────────────

    #[test]
    fn forward_scales_and_targets_correctly() {
        let prog = ProgramBuilder::new().forward(LocusId(7), KIND, 2.0).build();
        let locus = make_locus(0, 0.5);
        let changes = prog.process(&locus, &[], &NullCtx);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].subject, ChangeSubject::Locus(LocusId(7)));
        let out = changes[0].after.as_slice()[0];
        assert!((out - 1.0).abs() < 1e-5, "expected 1.0, got {out}");
    }

    #[test]
    fn forward_suppresses_below_noise_floor() {
        let prog = ProgramBuilder::new().forward(LocusId(7), KIND, 1.0).build();
        // 0.0001 * 1.0 = 0.0001 < 0.001
        let locus = make_locus(0, 0.0001);
        assert!(prog.process(&locus, &[], &NullCtx).is_empty());
    }

    // ── broadcast ─────────────────────────────────────────────────────────────

    #[test]
    fn broadcast_fans_out_to_all_targets() {
        let targets = vec![LocusId(1), LocusId(2), LocusId(3)];
        let prog = ProgramBuilder::new().broadcast(targets, KIND, 1.0).build();
        let locus = make_locus(0, 0.5);
        let changes = prog.process(&locus, &[], &NullCtx);
        assert_eq!(changes.len(), 3);
        let subjects: Vec<_> = changes.iter().map(|c| c.subject.clone()).collect();
        assert!(subjects.contains(&ChangeSubject::Locus(LocusId(1))));
        assert!(subjects.contains(&ChangeSubject::Locus(LocusId(2))));
        assert!(subjects.contains(&ChangeSubject::Locus(LocusId(3))));
    }

    #[test]
    fn broadcast_suppresses_when_signal_below_floor() {
        let prog = ProgramBuilder::new()
            .broadcast(vec![LocusId(1), LocusId(2)], KIND, 1.0)
            .build();
        let locus = make_locus(0, 0.0001);
        assert!(prog.process(&locus, &[], &NullCtx).is_empty());
    }

    // ── accumulate ────────────────────────────────────────────────────────────

    #[test]
    fn accumulate_targets_self() {
        let prog = ProgramBuilder::new().accumulate(KIND, 0.1).build();
        let locus = make_locus(42, 0.8);
        let changes = prog.process(&locus, &[], &NullCtx);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].subject, ChangeSubject::Locus(LocusId(42)));
        let out = changes[0].after.as_slice()[0];
        assert!((out - 0.08).abs() < 1e-5, "expected 0.08, got {out}");
    }

    #[test]
    fn accumulate_suppresses_below_floor() {
        let prog = ProgramBuilder::new().accumulate(KIND, 0.001).build();
        // 0.0001 * 0.001 = 1e-7 < 0.001
        let locus = make_locus(0, 0.0001);
        assert!(prog.process(&locus, &[], &NullCtx).is_empty());
    }

    // ── composition ───────────────────────────────────────────────────────────

    #[test]
    fn multiple_rules_flatten_in_insertion_order() {
        let prog = ProgramBuilder::new()
            .forward(LocusId(1), KIND, 1.0)
            .forward(LocusId(2), KIND, 1.0)
            .build();
        let locus = make_locus(0, 0.5);
        let changes = prog.process(&locus, &[], &NullCtx);
        assert_eq!(changes.len(), 2);
        assert_eq!(changes[0].subject, ChangeSubject::Locus(LocusId(1)));
        assert_eq!(changes[1].subject, ChangeSubject::Locus(LocusId(2)));
    }

    // ── on_process / on_structural ────────────────────────────────────────────

    #[test]
    fn on_process_closure_is_called() {
        let prog = ProgramBuilder::new()
            .on_process(|_l, _i, _c| {
                vec![ProposedChange::activation(LocusId(99), KIND, 0.5)]
            })
            .build();
        let locus = make_locus(0, 0.0);
        let changes = prog.process(&locus, &[], &NullCtx);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].subject, ChangeSubject::Locus(LocusId(99)));
    }

    #[test]
    fn on_structural_closure_is_called() {
        let prog = ProgramBuilder::new()
            .on_structural(|_l, _i, _c| {
                vec![StructuralProposal::create_directed(
                    LocusId(0),
                    LocusId(1),
                    InfluenceKindId(1),
                )]
            })
            .build();
        let locus = make_locus(0, 0.0);
        let proposals = prog.structural_proposals(&locus, &[], &NullCtx);
        assert_eq!(proposals.len(), 1);
    }

    // ── initial subscriptions ─────────────────────────────────────────────────

    #[test]
    fn subscribe_initial_accumulates_across_calls() {
        let prog = ProgramBuilder::new()
            .subscribe_initial(&[RelationshipId(10), RelationshipId(20)])
            .subscribe_initial(&[RelationshipId(30)])
            .build();
        let locus = make_locus(0, 0.0);
        let subs = prog.initial_subscriptions(&locus);
        assert_eq!(
            subs,
            vec![RelationshipId(10), RelationshipId(20), RelationshipId(30)]
        );
    }
}
