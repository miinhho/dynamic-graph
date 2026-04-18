mod render;
mod trace;
mod types;

pub use trace::causal_trace;
pub use types::{CausalStep, CausalTrace};

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
            trace
                .steps
                .iter()
                .filter(|s| s.change_id == ChangeId(0))
                .count(),
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
