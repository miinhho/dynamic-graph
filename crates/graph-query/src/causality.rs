//! Causal DAG queries over the change log.

mod dag;
mod latest;
mod temporal;
mod trends;
mod types;

pub use dag::{
    causal_ancestors, causal_coarse_trail, causal_depth, causal_descendants, common_ancestors,
    is_ancestor_of, root_stimuli, root_stimuli_for_relationship,
};
pub use latest::{last_change_to_locus, last_change_to_relationship};
pub use temporal::{
    changes_to_locus_in_range, changes_to_relationship_in_range, committed_batches,
    loci_changed_in_batch, relationships_changed_in_batch,
};
pub use trends::{
    relationship_activity_trend, relationship_activity_trend_with_threshold,
    relationship_volatility, relationship_volatility_all, relationship_weight_delta,
    relationship_weight_trend, relationship_weight_trend_delta,
    relationship_weight_trend_with_threshold,
};
pub use types::{CoarseTrail, Trend};

pub(crate) use trends::ols_activity_slope;

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

    fn chain_world() -> World {
        let mut w = World::new();
        push_change(&mut w, 0, 0, vec![], 0);
        push_change(&mut w, 1, 1, vec![0], 1);
        push_change(&mut w, 2, 2, vec![1], 2);
        w
    }

    #[test]
    fn causal_ancestors_returns_all_predecessors() {
        let w = chain_world();
        let mut ancestors = causal_ancestors(&w, ChangeId(2));
        ancestors.sort();
        assert_eq!(ancestors, vec![ChangeId(0), ChangeId(1)]);
    }

    #[test]
    fn causal_ancestors_of_root_is_empty() {
        let w = chain_world();
        assert!(causal_ancestors(&w, ChangeId(0)).is_empty());
    }

    #[test]
    fn is_ancestor_of_detects_true_and_false() {
        let w = chain_world();
        assert!(is_ancestor_of(&w, ChangeId(0), ChangeId(2)));
        assert!(is_ancestor_of(&w, ChangeId(1), ChangeId(2)));
        assert!(!is_ancestor_of(&w, ChangeId(2), ChangeId(1)));
        assert!(!is_ancestor_of(&w, ChangeId(99), ChangeId(2)));
    }

    #[test]
    fn common_ancestors_intersects_two_walks() {
        let mut w = World::new();
        push_change(&mut w, 0, 0, vec![], 0);
        push_change(&mut w, 1, 1, vec![0], 1);
        push_change(&mut w, 2, 2, vec![0], 1);
        push_change(&mut w, 3, 3, vec![1], 2);
        push_change(&mut w, 4, 4, vec![2], 2);

        let mut shared = common_ancestors(&w, ChangeId(3), ChangeId(4));
        shared.sort();
        assert_eq!(shared, vec![ChangeId(0)]);
    }

    #[test]
    fn root_stimuli_finds_only_leaves() {
        let w = chain_world();
        assert_eq!(root_stimuli(&w, ChangeId(2)), vec![ChangeId(0)]);
    }

    #[test]
    fn causal_depth_tracks_longest_chain() {
        let w = chain_world();
        assert_eq!(causal_depth(&w, ChangeId(0)), 0);
        assert_eq!(causal_depth(&w, ChangeId(1)), 1);
        assert_eq!(causal_depth(&w, ChangeId(2)), 2);
    }

    #[test]
    fn descendants_walk_forward_dual() {
        let w = chain_world();
        let mut desc = causal_descendants(&w, ChangeId(0));
        desc.sort();
        assert_eq!(desc, vec![ChangeId(1), ChangeId(2)]);
    }

    #[test]
    fn latest_change_helpers_use_reverse_indices() {
        let w = chain_world();
        assert_eq!(
            last_change_to_locus(&w, LocusId(2)).map(|c| c.id),
            Some(ChangeId(2))
        );
        assert_eq!(last_change_to_locus(&w, LocusId(999)).map(|c| c.id), None);
    }
}
