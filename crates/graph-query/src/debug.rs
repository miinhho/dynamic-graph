//! Causal trace and debug utilities.
//!
//! [`CausalTrace`] provides a structured, human-readable walk of the
//! predecessor DAG for a specific locus at a given batch. Use it for
//! debugging why a locus reached its current state — seeing not just
//! *what* changed but *what caused* each change.
//!
//! ## Example
//!
//! ```rust,ignore
//! let trace = graph_query::causal_trace(&world, locus_id, world.current_batch());
//! println!("{trace}");
//!
//! // Or check the structured data:
//! for step in &trace.steps {
//!     println!("  change {:?} @ depth {} via kind {:?}", step.change_id, step.depth, step.kind);
//! }
//! if trace.truncated {
//!     eprintln!("warning: trace stopped at trimmed log boundary");
//! }
//! ```

use std::collections::VecDeque;
use std::fmt;

use rustc_hash::FxHashSet;

use graph_core::{BatchId, ChangeId, ChangeSubject, InfluenceKindId, LocusId, StateVector};
use graph_world::World;

// ─── CausalStep ──────────────────────────────────────────────────────────────

/// One step in a causal trace.
///
/// Captures all information from a single [`Change`][graph_core::Change] that
/// is relevant for debugging: what was touched, which kind drove the change,
/// what the state looked like before and after, and which earlier changes are
/// the direct predecessors of this one.
#[derive(Debug, Clone)]
pub struct CausalStep {
    /// BFS depth from the trace's starting change.
    /// Depth 0 = the most recent change to the target locus.
    /// Depth N = N hops back through the predecessor DAG.
    pub depth: usize,
    /// The ID of this change.
    pub change_id: ChangeId,
    /// What this change was about (locus or relationship).
    pub subject: ChangeSubject,
    /// Influence kind that drove this change.
    pub kind: InfluenceKindId,
    /// State of the subject *before* this change fired.
    pub before: StateVector,
    /// State of the subject *after* this change fired.
    pub after: StateVector,
    /// Batch in which this change was committed.
    pub batch: BatchId,
    /// Direct predecessor change IDs. Empty for root stimuli.
    pub predecessor_ids: Vec<ChangeId>,
}

// ─── CausalTrace ─────────────────────────────────────────────────────────────

/// A structured walk of the causal predecessor DAG for a locus at a batch.
///
/// Produced by [`causal_trace`]. Steps are ordered breadth-first from the most
/// recent change back toward root stimuli. The first step (if any) is always
/// the most recent change to the target locus at or before `batch`.
///
/// ## Display
///
/// `CausalTrace` implements `Display`. The output is a multi-line ASCII
/// representation suitable for logging or printing:
///
/// ```text
/// CausalTrace for locus 3 @ batch 7 (4 steps)
///   [0] change #12  batch=7  kind=1  Locus(3)  0.00→0.72  preds=[#9, #10]
///   [1] change #9   batch=6  kind=1  Locus(1)  0.00→0.50  preds=[]  (root)
///   [1] change #10  batch=6  kind=2  Locus(2)  0.00→0.30  preds=[#5]
///   [2] change #5   batch=5  kind=1  Locus(2)  0.00→0.80  preds=[]  (root)
/// ```
///
/// When `truncated` is `true`, the display appends a warning line.
#[derive(Debug, Clone)]
pub struct CausalTrace {
    /// The target locus this trace was computed for.
    pub target: LocusId,
    /// The batch at (or before which) the trace starts.
    pub batch: BatchId,
    /// Steps in the trace, breadth-first from newest to oldest.
    pub steps: Vec<CausalStep>,
    /// `true` when the walk stopped because a predecessor pointed into a
    /// trimmed portion of the log. The trace is incomplete.
    pub truncated: bool,
}

impl fmt::Display for CausalTrace {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(
            f,
            "CausalTrace for locus {} @ batch {} ({} step{})",
            self.target.0,
            self.batch.0,
            self.steps.len(),
            if self.steps.len() == 1 { "" } else { "s" },
        )?;

        if self.steps.is_empty() {
            writeln!(f, "  (no changes found)")?;
        }

        for step in &self.steps {
            let subject_str = match step.subject {
                ChangeSubject::Locus(id) => format!("Locus({})", id.0),
                ChangeSubject::Relationship(id) => format!("Rel({})", id.0),
            };

            let before0 = step.before.as_slice().first().copied().unwrap_or(0.0);
            let after0 = step.after.as_slice().first().copied().unwrap_or(0.0);

            let preds_str = if step.predecessor_ids.is_empty() {
                String::from("[]  (root)")
            } else {
                let ids: Vec<String> = step
                    .predecessor_ids
                    .iter()
                    .map(|c| format!("#{}", c.0))
                    .collect();
                format!("[{}]", ids.join(", "))
            };

            writeln!(
                f,
                "  [{}] change #{:<4}  batch={:<4}  kind={:<3}  {:<12}  {:.2}→{:.2}  preds={}",
                step.depth,
                step.change_id.0,
                step.batch.0,
                step.kind.0,
                subject_str,
                before0,
                after0,
                preds_str,
            )?;
        }

        if self.truncated {
            writeln!(
                f,
                "  *** trace truncated: predecessor(s) point into trimmed log ***"
            )?;
        }

        Ok(())
    }
}

// ─── causal_trace ─────────────────────────────────────────────────────────────

/// Compute a [`CausalTrace`] for `locus` at `batch`.
///
/// Starting from the most recent change to `locus` committed at or before
/// `batch`, the trace walks backward through the predecessor DAG in
/// breadth-first order, following predecessors until root stimuli (changes
/// with no predecessors) are reached.
///
/// Each step in the trace is annotated with the step's BFS depth (distance
/// from the starting change). The trace is deduplicated: each `ChangeId`
/// appears at most once, at the shallowest depth it was first discovered.
///
/// When a predecessor `ChangeId` is not present in the log (e.g. it was
/// trimmed by `trim_before_batch`), the trace stops at that node and sets
/// [`CausalTrace::truncated`] to `true`.
///
/// Returns an empty trace (no steps) when:
/// - The locus has no changes at or before `batch`.
/// - The log was fully trimmed for this locus.
pub fn causal_trace(world: &World, locus: LocusId, batch: BatchId) -> CausalTrace {
    // Find the most recent change to the locus at or before `batch`.
    let start = world
        .changes_to_locus(locus)
        .find(|c| c.batch.0 <= batch.0);

    let Some(start_change) = start else {
        return CausalTrace {
            target: locus,
            batch,
            steps: Vec::new(),
            truncated: false,
        };
    };

    // BFS: (change_id, depth)
    let mut queue: VecDeque<(ChangeId, usize)> = VecDeque::new();
    let mut visited: FxHashSet<ChangeId> = FxHashSet::default();
    let mut steps: Vec<CausalStep> = Vec::new();
    let mut truncated = false;

    queue.push_back((start_change.id, 0));
    visited.insert(start_change.id);

    while let Some((cid, depth)) = queue.pop_front() {
        let Some(change) = world.log().get(cid) else {
            // This ID was referenced as a predecessor but is not in the log —
            // it was trimmed.
            truncated = true;
            continue;
        };

        // Check whether any predecessors point into trimmed territory before
        // we add them to the queue, so we can flag truncation early.
        for &pred in &change.predecessors {
            if world.log().get(pred).is_none() {
                truncated = true;
            }
        }

        steps.push(CausalStep {
            depth,
            change_id: change.id,
            subject: change.subject.clone(),
            kind: change.kind,
            before: change.before.clone(),
            after: change.after.clone(),
            batch: change.batch,
            predecessor_ids: change.predecessors.clone(),
        });

        // Enqueue unvisited predecessors.
        for &pred in &change.predecessors {
            if visited.insert(pred) {
                queue.push_back((pred, depth + 1));
            }
        }
    }

    CausalTrace { target: locus, batch, steps, truncated }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use graph_core::{
        BatchId, Change, ChangeId, ChangeSubject, InfluenceKindId, LocusId, StateVector,
    };
    use graph_world::World;

    fn push_change(
        world: &mut World,
        id: u64,
        locus: u64,
        preds: Vec<u64>,
        batch: u64,
    ) -> ChangeId {
        let cid = ChangeId(id);
        world.log_mut().append(Change {
            id: cid,
            subject: ChangeSubject::Locus(LocusId(locus)),
            kind: InfluenceKindId(1),
            predecessors: preds.into_iter().map(ChangeId).collect(),
            before: StateVector::zeros(1),
            after: StateVector::from_slice(&[0.5]),
            batch: BatchId(batch),
            wall_time: None,
            metadata: None,
        });
        cid
    }

    // ── Empty / no-history cases ─────────────────────────────────────────────

    #[test]
    fn trace_empty_for_locus_with_no_changes() {
        let w = World::new();
        let trace = causal_trace(&w, LocusId(0), BatchId(5));
        assert!(trace.steps.is_empty());
        assert!(!trace.truncated);
        assert_eq!(trace.target, LocusId(0));
    }

    #[test]
    fn trace_empty_when_all_changes_after_batch() {
        let mut w = World::new();
        push_change(&mut w, 0, 0, vec![], 10);
        // Ask for batch 5, but the only change is at batch 10.
        let trace = causal_trace(&w, LocusId(0), BatchId(5));
        assert!(trace.steps.is_empty());
    }

    // ── Single root stimulus ─────────────────────────────────────────────────

    #[test]
    fn trace_single_root_step() {
        let mut w = World::new();
        push_change(&mut w, 0, 0, vec![], 1);
        let trace = causal_trace(&w, LocusId(0), BatchId(5));
        assert_eq!(trace.steps.len(), 1);
        assert_eq!(trace.steps[0].depth, 0);
        assert_eq!(trace.steps[0].change_id, ChangeId(0));
        assert!(trace.steps[0].predecessor_ids.is_empty());
        assert!(!trace.truncated);
    }

    // ── Linear chain ─────────────────────────────────────────────────────────

    #[test]
    fn trace_linear_chain_walks_predecessors() {
        // c0 (root, locus 1) → c1 (locus 1) → c2 (locus 0)
        let mut w = World::new();
        push_change(&mut w, 0, 1, vec![], 0);
        push_change(&mut w, 1, 1, vec![0], 1);
        push_change(&mut w, 2, 0, vec![1], 2);

        let trace = causal_trace(&w, LocusId(0), BatchId(5));
        // Steps: c2 (depth 0) → c1 (depth 1) → c0 (depth 2)
        assert_eq!(trace.steps.len(), 3);
        assert_eq!(trace.steps[0].change_id, ChangeId(2));
        assert_eq!(trace.steps[0].depth, 0);
        assert_eq!(trace.steps[1].change_id, ChangeId(1));
        assert_eq!(trace.steps[1].depth, 1);
        assert_eq!(trace.steps[2].change_id, ChangeId(0));
        assert_eq!(trace.steps[2].depth, 2);
        assert!(!trace.truncated);
    }

    // ── Deduplication in diamond ─────────────────────────────────────────────

    #[test]
    fn trace_deduplicates_diamond_predecessors() {
        // c0 (root) → c1, c2 → c3 (locus 0)
        let mut w = World::new();
        push_change(&mut w, 0, 1, vec![], 0);
        push_change(&mut w, 1, 2, vec![0], 1);
        push_change(&mut w, 2, 3, vec![0], 1);
        push_change(&mut w, 3, 0, vec![1, 2], 2);

        let trace = causal_trace(&w, LocusId(0), BatchId(5));
        // c0 should appear exactly once even though both c1 and c2 reference it.
        assert_eq!(
            trace.steps.iter().filter(|s| s.change_id == ChangeId(0)).count(),
            1
        );
        // 4 unique changes total.
        assert_eq!(trace.steps.len(), 4);
        assert!(!trace.truncated);
    }

    // ── Batch filter ─────────────────────────────────────────────────────────

    #[test]
    fn trace_starts_from_most_recent_change_at_or_before_batch() {
        // c0 at batch 1, c1 at batch 5 — asking for batch 3 should start from c0.
        let mut w = World::new();
        push_change(&mut w, 0, 0, vec![], 1);
        push_change(&mut w, 1, 0, vec![0], 5);

        let trace = causal_trace(&w, LocusId(0), BatchId(3));
        assert_eq!(trace.steps.len(), 1);
        assert_eq!(trace.steps[0].change_id, ChangeId(0));
    }

    // ── Display smoke tests ──────────────────────────────────────────────────

    #[test]
    fn trace_display_contains_locus_and_batch() {
        let mut w = World::new();
        push_change(&mut w, 0, 0, vec![], 2);
        let trace = causal_trace(&w, LocusId(0), BatchId(2));
        let rendered = trace.to_string();
        assert!(rendered.contains("locus 0"), "got: {rendered}");
        assert!(rendered.contains("batch 2"), "got: {rendered}");
        assert!(rendered.contains("(root)"), "got: {rendered}");
    }

    #[test]
    fn trace_display_truncated_note() {
        let trace = CausalTrace {
            target: LocusId(0),
            batch: BatchId(1),
            steps: vec![],
            truncated: true,
        };
        let rendered = trace.to_string();
        assert!(rendered.contains("truncated"), "got: {rendered}");
    }

    #[test]
    fn trace_display_predecessor_ids_listed() {
        let mut w = World::new();
        push_change(&mut w, 0, 1, vec![], 0);
        push_change(&mut w, 1, 0, vec![0], 1);

        let trace = causal_trace(&w, LocusId(0), BatchId(5));
        let rendered = trace.to_string();
        // The first step (depth 0) should show pred #0.
        assert!(rendered.contains("#0"), "got: {rendered}");
    }
}
