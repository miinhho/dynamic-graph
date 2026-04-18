mod adapter;
#[path = "profile/bundle.rs"]
mod bundle;
#[path = "profile/metrics.rs"]
mod metrics;
#[path = "profile/terminals.rs"]
mod terminals;

pub use bundle::{RelationshipBundle, relationship_profile};

#[cfg(test)]
mod tests {
    use super::*;
    use graph_core::{Endpoints, InfluenceKindId, LocusId, StateVector};
    use graph_world::World;

    fn make_world_ab() -> (World, LocusId, LocusId) {
        let a = LocusId(0);
        let b = LocusId(1);
        let mut w = World::new();
        let k1 = InfluenceKindId(1);
        let k2 = InfluenceKindId(2);
        // A→B: k1 activity=2.0, k2 activity=-1.0
        w.add_relationship(
            Endpoints::directed(a, b),
            k1,
            StateVector::from_slice(&[2.0, 0.0]),
        );
        w.add_relationship(
            Endpoints::directed(a, b),
            k2,
            StateVector::from_slice(&[-1.0, 0.0]),
        );
        // B→A: k1 activity=0.5
        w.add_relationship(
            Endpoints::directed(b, a),
            k1,
            StateVector::from_slice(&[0.5, 0.0]),
        );
        // Unrelated edge A→C
        w.add_relationship(
            Endpoints::directed(a, LocusId(2)),
            k1,
            StateVector::from_slice(&[3.0, 0.0]),
        );
        (w, a, b)
    }

    #[test]
    fn profile_collects_both_directions() {
        let (w, a, b) = make_world_ab();
        let p = relationship_profile(&w, a, b);
        assert_eq!(p.len(), 3); // A→B k1, A→B k2, B→A k1
    }

    #[test]
    fn profile_excludes_unrelated_edges() {
        let (w, a, b) = make_world_ab();
        let p = relationship_profile(&w, a, b);
        assert!(p.relationships.iter().all(|r| r.endpoints.involves(b)));
    }

    #[test]
    fn net_activity_sums_correctly() {
        let (w, a, b) = make_world_ab();
        let net = relationship_profile(&w, a, b).net_activity();
        assert!((net - 1.5).abs() < 1e-5, "expected 1.5, got {net}");
    }

    #[test]
    fn activity_by_kind_merges_both_directions() {
        let (w, a, b) = make_world_ab();
        let pairs = relationship_profile(&w, a, b).activity_by_kind();
        let k1_sum = pairs
            .iter()
            .find(|(k, _)| *k == InfluenceKindId(1))
            .map(|(_, v)| *v);
        // k1: 2.0 (A→B) + 0.5 (B→A) = 2.5
        assert!((k1_sum.unwrap() - 2.5).abs() < 1e-5);
    }

    #[test]
    fn dominant_kind_is_highest_activity() {
        let (w, a, b) = make_world_ab();
        assert_eq!(
            relationship_profile(&w, a, b).dominant_kind(),
            Some(InfluenceKindId(1))
        );
    }

    #[test]
    fn empty_profile_when_no_edges() {
        let w = World::new();
        let p = relationship_profile(&w, LocusId(0), LocusId(1));
        assert!(p.is_empty());
        assert_eq!(p.net_activity(), 0.0);
        assert!(p.dominant_kind().is_none());
    }

    #[test]
    fn is_excitatory_inhibitory() {
        let (w, a, b) = make_world_ab();
        let p = relationship_profile(&w, a, b);
        assert!(p.is_excitatory());
        assert!(!p.is_inhibitory());
    }

    #[test]
    fn profile_similarity_identical_profiles() {
        let (w, a, b) = make_world_ab();
        let p = relationship_profile(&w, a, b);
        // Similarity with itself = 1.0
        assert!((p.profile_similarity(&p) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn profile_similarity_orthogonal_kinds() {
        // Profile AB has only k1; profile CD has only k2 → orthogonal → 0.0
        let k1 = InfluenceKindId(1);
        let k2 = InfluenceKindId(2);
        let mut w = World::new();
        w.add_relationship(
            Endpoints::directed(LocusId(0), LocusId(1)),
            k1,
            StateVector::from_slice(&[1.0, 0.0]),
        );
        w.add_relationship(
            Endpoints::directed(LocusId(2), LocusId(3)),
            k2,
            StateVector::from_slice(&[1.0, 0.0]),
        );
        let pab = relationship_profile(&w, LocusId(0), LocusId(1));
        let pcd = relationship_profile(&w, LocusId(2), LocusId(3));
        assert!((pab.profile_similarity(&pcd)).abs() < 1e-5);
    }
}
