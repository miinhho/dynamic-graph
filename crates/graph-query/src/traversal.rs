//! BFS-based graph traversal: shortest path, reachability, and connected
//! components. All operations treat the relationship graph as **undirected**
//! unless a directed or activity-aware variant is explicitly used.

mod activity;
mod analysis;
mod directed;
mod neighbors;
mod primitives;
mod topology;
mod undirected;

#[cfg(test)]
use graph_core::LocusId;
#[cfg(test)]
use graph_world::World;

pub use self::activity::{
    downstream_of_active, path_between_active, reachable_from_active, upstream_of_active,
};
pub use self::analysis::{TransitiveRule, has_cycle, infer_transitive};
pub use self::directed::{
    directed_path, directed_path_of_kind, downstream_of, downstream_of_kind, upstream_of,
    upstream_of_kind,
};
pub use self::topology::{
    hub_loci, isolated_loci, neighbors_of, neighbors_of_kind, reciprocal_of, reciprocal_pairs,
    sink_loci, source_loci,
};
pub use self::undirected::{
    connected_components, connected_components_of_kind, path_between, path_between_of_kind,
    reachable_from, reachable_from_of_kind, reachable_matching, strongest_path,
};

#[cfg(test)]
mod tests {
    use super::*;
    use graph_core::{
        Endpoints, InfluenceKindId, KindObservation, Locus, LocusKindId, Relationship,
        RelationshipKindId, RelationshipLineage, StateVector,
    };

    fn chain_world(n: u64) -> World {
        let kind = LocusKindId(1);
        let rk: RelationshipKindId = InfluenceKindId(1);
        let mut w = World::new();
        for i in 0..n {
            w.insert_locus(Locus::new(LocusId(i), kind, StateVector::zeros(1)));
        }
        for i in 0..(n - 1) {
            let id = w.relationships_mut().mint_id();
            w.relationships_mut().insert(Relationship {
                id,
                kind: rk,
                endpoints: Endpoints::Directed {
                    from: LocusId(i),
                    to: LocusId(i + 1),
                },
                state: StateVector::from_slice(&[1.0, 0.0]),
                lineage: RelationshipLineage {
                    created_by: None,
                    last_touched_by: None,
                    change_count: 1,
                    kinds_observed: smallvec::smallvec![KindObservation::synthetic(rk)],
                },
                created_batch: graph_core::BatchId(0),
                last_decayed_batch: 0,
                metadata: None,
            });
        }
        w
    }

    fn two_chain_world() -> World {
        let kind = LocusKindId(1);
        let rk: RelationshipKindId = InfluenceKindId(1);
        let mut w = World::new();
        for id in [0u64, 1, 2, 10, 11] {
            w.insert_locus(Locus::new(LocusId(id), kind, StateVector::zeros(1)));
        }
        for (from, to) in [(0u64, 1), (1, 2), (10, 11)] {
            let id = w.relationships_mut().mint_id();
            w.relationships_mut().insert(Relationship {
                id,
                kind: rk,
                endpoints: Endpoints::Directed {
                    from: LocusId(from),
                    to: LocusId(to),
                },
                state: StateVector::from_slice(&[1.0, 0.0]),
                lineage: RelationshipLineage {
                    created_by: None,
                    last_touched_by: None,
                    change_count: 1,
                    kinds_observed: smallvec::smallvec![KindObservation::synthetic(rk)],
                },
                created_batch: graph_core::BatchId(0),
                last_decayed_batch: 0,
                metadata: None,
            });
        }
        w
    }

    #[test]
    fn path_between_same_locus_returns_singleton() {
        let w = chain_world(4);
        assert_eq!(
            path_between(&w, LocusId(2), LocusId(2)),
            Some(vec![LocusId(2)])
        );
    }

    #[test]
    fn path_between_finds_shortest_path() {
        let w = chain_world(5);
        let path = path_between(&w, LocusId(0), LocusId(4)).unwrap();
        assert_eq!(
            path,
            vec![LocusId(0), LocusId(1), LocusId(2), LocusId(3), LocusId(4)]
        );
    }

    #[test]
    fn path_between_returns_none_for_disconnected_loci() {
        let mut w = chain_world(3);
        w.insert_locus(Locus::new(
            LocusId(99),
            LocusKindId(1),
            StateVector::zeros(1),
        ));
        assert!(path_between(&w, LocusId(0), LocusId(99)).is_none());
    }

    #[test]
    fn reachable_from_depth_1_returns_direct_neighbors() {
        let w = chain_world(5);
        let mut reached = reachable_from(&w, LocusId(2), 1);
        reached.sort();
        assert_eq!(reached, vec![LocusId(1), LocusId(3)]);
    }

    #[test]
    fn reachable_from_depth_0_is_empty() {
        let w = chain_world(4);
        assert!(reachable_from(&w, LocusId(0), 0).is_empty());
    }

    #[test]
    fn connected_components_counts_correctly() {
        let w = two_chain_world();
        let comps = connected_components(&w);
        assert_eq!(comps.len(), 2);
        let mut sizes: Vec<usize> = comps.iter().map(Vec::len).collect();
        sizes.sort();
        assert_eq!(sizes, vec![2, 3]);
    }

    fn two_path_world() -> World {
        let lk = LocusKindId(1);
        let rk: RelationshipKindId = InfluenceKindId(1);
        let mut w = World::new();
        for i in 0u64..4 {
            w.insert_locus(Locus::new(LocusId(i), lk, StateVector::zeros(1)));
        }
        for (from, to, activity) in [(0u64, 1u64, 0.1f32), (1, 3, 0.1), (0, 2, 5.0), (2, 3, 5.0)] {
            let id = w.relationships_mut().mint_id();
            w.relationships_mut().insert(Relationship {
                id,
                kind: rk,
                endpoints: Endpoints::Directed {
                    from: LocusId(from),
                    to: LocusId(to),
                },
                state: StateVector::from_slice(&[activity, 0.0]),
                lineage: RelationshipLineage {
                    created_by: None,
                    last_touched_by: None,
                    change_count: 1,
                    kinds_observed: smallvec::smallvec![KindObservation::synthetic(rk)],
                },
                created_batch: graph_core::BatchId(0),
                last_decayed_batch: 0,
                metadata: None,
            });
        }
        w
    }

    #[test]
    fn strongest_path_same_locus_returns_singleton() {
        let w = chain_world(4);
        assert_eq!(
            strongest_path(&w, LocusId(1), LocusId(1)),
            Some(vec![LocusId(1)])
        );
    }

    #[test]
    fn strongest_path_returns_none_for_disconnected() {
        let mut w = chain_world(3);
        w.insert_locus(Locus::new(
            LocusId(99),
            LocusKindId(1),
            StateVector::zeros(1),
        ));
        assert!(strongest_path(&w, LocusId(0), LocusId(99)).is_none());
    }

    #[test]
    fn strongest_path_prefers_high_activity_over_short_hops() {
        let w = two_path_world();
        let path = strongest_path(&w, LocusId(0), LocusId(3)).unwrap();
        assert_eq!(path, vec![LocusId(0), LocusId(2), LocusId(3)]);
    }

    #[test]
    fn path_between_chooses_short_path_over_strong_path() {
        let w = two_path_world();
        let path = path_between(&w, LocusId(0), LocusId(3)).unwrap();
        assert_eq!(path, vec![LocusId(0), LocusId(1), LocusId(3)]);
    }

    #[test]
    fn connected_components_of_kind_filters_by_kind() {
        let kind_a: RelationshipKindId = InfluenceKindId(1);
        let kind_b: RelationshipKindId = InfluenceKindId(2);
        let lk = LocusKindId(1);
        let mut w = World::new();
        for i in 0u64..4 {
            w.insert_locus(Locus::new(LocusId(i), lk, StateVector::zeros(1)));
        }
        for (from, to, kind) in [(0u64, 1, kind_a), (2, 3, kind_b)] {
            let id = w.relationships_mut().mint_id();
            w.relationships_mut().insert(Relationship {
                id,
                kind,
                endpoints: Endpoints::Directed {
                    from: LocusId(from),
                    to: LocusId(to),
                },
                state: StateVector::from_slice(&[1.0, 0.0]),
                lineage: RelationshipLineage {
                    created_by: None,
                    last_touched_by: None,
                    change_count: 1,
                    kinds_observed: smallvec::smallvec![KindObservation::synthetic(kind)],
                },
                created_batch: graph_core::BatchId(0),
                last_decayed_batch: 0,
                metadata: None,
            });
        }
        let comps_a = connected_components_of_kind(&w, kind_a);
        let mut sizes_a: Vec<usize> = comps_a.iter().map(Vec::len).collect();
        sizes_a.sort();
        assert_eq!(sizes_a, vec![1, 1, 2]);

        let comps_b = connected_components_of_kind(&w, kind_b);
        let mut sizes_b: Vec<usize> = comps_b.iter().map(Vec::len).collect();
        sizes_b.sort();
        assert_eq!(sizes_b, vec![1, 1, 2]);
    }

    fn diamond_world() -> World {
        let lk = LocusKindId(1);
        let rk: RelationshipKindId = InfluenceKindId(1);
        let mut w = World::new();
        for i in 0u64..4 {
            w.insert_locus(Locus::new(LocusId(i), lk, StateVector::zeros(1)));
        }
        for (from, to) in [(0u64, 1), (0, 2), (1, 3), (2, 3)] {
            let id = w.relationships_mut().mint_id();
            w.relationships_mut().insert(Relationship {
                id,
                kind: rk,
                endpoints: Endpoints::Directed {
                    from: LocusId(from),
                    to: LocusId(to),
                },
                state: StateVector::from_slice(&[1.0, 0.0]),
                lineage: RelationshipLineage {
                    created_by: None,
                    last_touched_by: None,
                    change_count: 1,
                    kinds_observed: smallvec::smallvec![KindObservation::synthetic(rk)],
                },
                created_batch: graph_core::BatchId(0),
                last_decayed_batch: 0,
                metadata: None,
            });
        }
        w
    }

    #[test]
    fn downstream_of_follows_directed_edges_forward() {
        let w = diamond_world();
        let mut reach = downstream_of(&w, LocusId(0), 3);
        reach.sort();
        assert_eq!(reach, vec![LocusId(1), LocusId(2), LocusId(3)]);
    }

    #[test]
    fn downstream_of_does_not_traverse_reverse_directed_edges() {
        let w = diamond_world();
        assert!(downstream_of(&w, LocusId(3), 3).is_empty());
    }

    #[test]
    fn upstream_of_follows_directed_edges_backward() {
        let w = diamond_world();
        let mut reach = upstream_of(&w, LocusId(3), 3);
        reach.sort();
        assert_eq!(reach, vec![LocusId(0), LocusId(1), LocusId(2)]);
    }

    #[test]
    fn upstream_of_does_not_traverse_forward_directed_edges() {
        let w = diamond_world();
        assert!(upstream_of(&w, LocusId(0), 3).is_empty());
    }

    #[test]
    fn directed_path_follows_direction() {
        let w = diamond_world();
        let path = directed_path(&w, LocusId(0), LocusId(3)).unwrap();
        assert_eq!(path.first(), Some(&LocusId(0)));
        assert_eq!(path.last(), Some(&LocusId(3)));
        assert_eq!(path.len(), 3);
    }

    #[test]
    fn directed_path_returns_none_against_direction() {
        let w = diamond_world();
        assert!(directed_path(&w, LocusId(3), LocusId(0)).is_none());
    }

    fn reciprocal_world() -> World {
        let lk = LocusKindId(1);
        let rk: RelationshipKindId = InfluenceKindId(1);
        let mut w = World::new();
        for i in 0u64..3 {
            w.insert_locus(Locus::new(LocusId(i), lk, StateVector::zeros(1)));
        }
        for (from, to) in [(0u64, 1u64), (1, 0), (0, 2)] {
            let id = w.relationships_mut().mint_id();
            w.relationships_mut().insert(Relationship {
                id,
                kind: rk,
                endpoints: Endpoints::Directed {
                    from: LocusId(from),
                    to: LocusId(to),
                },
                state: StateVector::from_slice(&[1.0, 0.0]),
                lineage: RelationshipLineage {
                    created_by: None,
                    last_touched_by: None,
                    change_count: 1,
                    kinds_observed: smallvec::smallvec![KindObservation::synthetic(rk)],
                },
                created_batch: graph_core::BatchId(0),
                last_decayed_batch: 0,
                metadata: None,
            });
        }
        w
    }

    #[test]
    fn reciprocal_of_finds_reverse_directed_edge() {
        let w = reciprocal_world();
        let rel_01 = w
            .relationships()
            .relationships_from(LocusId(0))
            .find(|r| r.endpoints.target() == Some(LocusId(1)))
            .map(|r| r.id)
            .unwrap();
        let rec = reciprocal_of(&w, rel_01);
        assert!(rec.is_some());
        let rec_rel = w.relationships().get(rec.unwrap()).unwrap();
        assert_eq!(rec_rel.endpoints.source(), Some(LocusId(1)));
        assert_eq!(rec_rel.endpoints.target(), Some(LocusId(0)));
    }

    #[test]
    fn reciprocal_of_returns_none_for_one_way_edge() {
        let w = reciprocal_world();
        let rel_02 = w
            .relationships()
            .relationships_from(LocusId(0))
            .find(|r| r.endpoints.target() == Some(LocusId(2)))
            .map(|r| r.id)
            .unwrap();
        assert!(reciprocal_of(&w, rel_02).is_none());
    }

    #[test]
    fn reciprocal_of_returns_none_for_symmetric() {
        let lk = LocusKindId(1);
        let rk: RelationshipKindId = InfluenceKindId(1);
        let mut w = World::new();
        for i in 0u64..2 {
            w.insert_locus(Locus::new(LocusId(i), lk, StateVector::zeros(1)));
        }
        let id = w.relationships_mut().mint_id();
        w.relationships_mut().insert(Relationship {
            id,
            kind: rk,
            endpoints: Endpoints::Symmetric {
                a: LocusId(0),
                b: LocusId(1),
            },
            state: StateVector::from_slice(&[1.0, 0.0]),
            lineage: RelationshipLineage {
                created_by: None,
                last_touched_by: None,
                change_count: 1,
                kinds_observed: smallvec::smallvec![KindObservation::synthetic(rk)],
            },
            created_batch: graph_core::BatchId(0),
            last_decayed_batch: 0,
            metadata: None,
        });
        assert!(reciprocal_of(&w, id).is_none());
    }

    #[test]
    fn reciprocal_pairs_finds_mutual_pair() {
        let w = reciprocal_world();
        let pairs = reciprocal_pairs(&w);
        assert_eq!(pairs.len(), 1);
        let (a, b) = pairs[0];
        let rel_a = w.relationships().get(a).unwrap();
        let rel_b = w.relationships().get(b).unwrap();
        assert!(rel_a.endpoints.involves(LocusId(0)));
        assert!(rel_a.endpoints.involves(LocusId(1)));
        assert!(rel_b.endpoints.involves(LocusId(0)));
        assert!(rel_b.endpoints.involves(LocusId(1)));
    }

    #[test]
    fn hub_loci_filters_by_degree() {
        let w = reciprocal_world();
        let hubs = hub_loci(&w, 3);
        assert_eq!(hubs, vec![LocusId(0)]);
        let all_connected = hub_loci(&w, 1);
        assert_eq!(all_connected.len(), 3);
        let none = hub_loci(&w, 10);
        assert!(none.is_empty());
    }

    #[test]
    fn isolated_loci_returns_loci_with_no_edges() {
        let lk = LocusKindId(1);
        let rk: RelationshipKindId = InfluenceKindId(1);
        let mut w = World::new();
        for i in 0u64..4 {
            w.insert_locus(Locus::new(LocusId(i), lk, StateVector::zeros(1)));
        }
        let id = w.relationships_mut().mint_id();
        w.relationships_mut().insert(Relationship {
            id,
            kind: rk,
            endpoints: Endpoints::Directed {
                from: LocusId(0),
                to: LocusId(1),
            },
            state: StateVector::from_slice(&[1.0, 0.0]),
            lineage: RelationshipLineage {
                created_by: None,
                last_touched_by: None,
                change_count: 1,
                kinds_observed: smallvec::smallvec![KindObservation::synthetic(rk)],
            },
            created_batch: graph_core::BatchId(0),
            last_decayed_batch: 0,
            metadata: None,
        });
        let mut iso = isolated_loci(&w);
        iso.sort();
        assert_eq!(iso, vec![LocusId(2), LocusId(3)]);
    }

    #[test]
    fn symmetric_edges_count_in_directed_traversal_both_ways() {
        let lk = LocusKindId(1);
        let rk: RelationshipKindId = InfluenceKindId(1);
        let mut w = World::new();
        for i in 0u64..3 {
            w.insert_locus(Locus::new(LocusId(i), lk, StateVector::zeros(1)));
        }
        for (a, b, sym) in [(0u64, 1u64, true), (1, 2, false)] {
            let id = w.relationships_mut().mint_id();
            let endpoints = if sym {
                Endpoints::Symmetric {
                    a: LocusId(a),
                    b: LocusId(b),
                }
            } else {
                Endpoints::Directed {
                    from: LocusId(a),
                    to: LocusId(b),
                }
            };
            w.relationships_mut().insert(Relationship {
                id,
                kind: rk,
                endpoints,
                state: StateVector::from_slice(&[1.0, 0.0]),
                lineage: RelationshipLineage {
                    created_by: None,
                    last_touched_by: None,
                    change_count: 1,
                    kinds_observed: smallvec::smallvec![KindObservation::synthetic(rk)],
                },
                created_batch: graph_core::BatchId(0),
                last_decayed_batch: 0,
                metadata: None,
            });
        }
        let mut ds = downstream_of(&w, LocusId(0), 3);
        ds.sort();
        assert_eq!(ds, vec![LocusId(1), LocusId(2)]);
        let mut us = upstream_of(&w, LocusId(2), 3);
        us.sort();
        assert_eq!(us, vec![LocusId(0), LocusId(1)]);
    }

    #[test]
    fn neighbors_of_returns_immediate_undirected_neighbors() {
        let w = chain_world(5);
        let mut nbrs = neighbors_of(&w, LocusId(2));
        nbrs.sort();
        assert_eq!(nbrs, vec![LocusId(1), LocusId(3)]);
        let nbrs0 = neighbors_of(&w, LocusId(0));
        assert_eq!(nbrs0, vec![LocusId(1)]);
    }

    #[test]
    fn neighbors_of_kind_filters_to_specific_kind() {
        let lk = LocusKindId(1);
        let mut w = World::new();
        for i in 0u64..4 {
            w.insert_locus(Locus::new(LocusId(i), lk, StateVector::zeros(1)));
        }
        let rk1: RelationshipKindId = InfluenceKindId(1);
        let rk2: RelationshipKindId = InfluenceKindId(2);
        let id1 = w.relationships_mut().mint_id();
        w.relationships_mut().insert(Relationship {
            id: id1,
            kind: rk1,
            endpoints: Endpoints::Directed {
                from: LocusId(0),
                to: LocusId(1),
            },
            state: StateVector::from_slice(&[1.0, 0.0]),
            lineage: RelationshipLineage {
                created_by: None,
                last_touched_by: None,
                change_count: 1,
                kinds_observed: smallvec::smallvec![KindObservation::synthetic(rk1)],
            },
            created_batch: graph_core::BatchId(0),
            last_decayed_batch: 0,
            metadata: None,
        });
        let id2 = w.relationships_mut().mint_id();
        w.relationships_mut().insert(Relationship {
            id: id2,
            kind: rk2,
            endpoints: Endpoints::Directed {
                from: LocusId(0),
                to: LocusId(2),
            },
            state: StateVector::from_slice(&[1.0, 0.0]),
            lineage: RelationshipLineage {
                created_by: None,
                last_touched_by: None,
                change_count: 1,
                kinds_observed: smallvec::smallvec![KindObservation::synthetic(rk2)],
            },
            created_batch: graph_core::BatchId(0),
            last_decayed_batch: 0,
            metadata: None,
        });
        assert_eq!(neighbors_of_kind(&w, LocusId(0), rk1), vec![LocusId(1)]);
        assert_eq!(neighbors_of_kind(&w, LocusId(0), rk2), vec![LocusId(2)]);
        let rk3: RelationshipKindId = InfluenceKindId(3);
        assert!(neighbors_of_kind(&w, LocusId(0), rk3).is_empty());
    }

    fn trust_chain_world() -> World {
        use graph_core::{
            Endpoints, InfluenceKindId, LocusKindId, Relationship, RelationshipKindId,
            RelationshipLineage, StateVector,
        };
        let lk = LocusKindId(1);
        let trust: RelationshipKindId = InfluenceKindId(10);
        let mut w = World::new();
        for i in 0u64..3 {
            w.insert_locus(Locus::new(LocusId(i), lk, StateVector::zeros(1)));
        }
        let id1 = w.relationships_mut().mint_id();
        w.relationships_mut().insert(Relationship {
            id: id1,
            kind: trust,
            endpoints: Endpoints::Directed {
                from: LocusId(0),
                to: LocusId(1),
            },
            state: StateVector::from_slice(&[0.8, 0.0]),
            lineage: RelationshipLineage {
                created_by: None,
                last_touched_by: None,
                change_count: 1,
                kinds_observed: smallvec::smallvec![KindObservation::synthetic(trust)],
            },
            created_batch: graph_core::BatchId(0),
            last_decayed_batch: 0,
            metadata: None,
        });
        let id2 = w.relationships_mut().mint_id();
        w.relationships_mut().insert(Relationship {
            id: id2,
            kind: trust,
            endpoints: Endpoints::Directed {
                from: LocusId(1),
                to: LocusId(2),
            },
            state: StateVector::from_slice(&[0.7, 0.0]),
            lineage: RelationshipLineage {
                created_by: None,
                last_touched_by: None,
                change_count: 1,
                kinds_observed: smallvec::smallvec![KindObservation::synthetic(trust)],
            },
            created_batch: graph_core::BatchId(0),
            last_decayed_batch: 0,
            metadata: None,
        });
        w
    }

    #[test]
    fn infer_transitive_product_weakens_with_hops() {
        use graph_core::InfluenceKindId;
        let w = trust_chain_world();
        let trust: graph_core::RelationshipKindId = InfluenceKindId(10);
        let result = infer_transitive(&w, LocusId(0), LocusId(2), trust, TransitiveRule::Product);
        assert!(result.is_some());
        assert!((result.unwrap() - 0.8 * 0.7).abs() < 1e-5);
    }

    #[test]
    fn infer_transitive_min_is_weakest_link() {
        use graph_core::InfluenceKindId;
        let w = trust_chain_world();
        let trust: graph_core::RelationshipKindId = InfluenceKindId(10);
        let result = infer_transitive(&w, LocusId(0), LocusId(2), trust, TransitiveRule::Min);
        assert!((result.unwrap() - 0.7).abs() < 1e-5);
    }

    #[test]
    fn infer_transitive_mean_averages_edges() {
        use graph_core::InfluenceKindId;
        let w = trust_chain_world();
        let trust: graph_core::RelationshipKindId = InfluenceKindId(10);
        let result = infer_transitive(&w, LocusId(0), LocusId(2), trust, TransitiveRule::Mean);
        assert!((result.unwrap() - 0.75).abs() < 1e-5);
    }

    #[test]
    fn infer_transitive_no_path_returns_none() {
        use graph_core::InfluenceKindId;
        let w = trust_chain_world();
        let trust: graph_core::RelationshipKindId = InfluenceKindId(10);
        assert!(
            infer_transitive(&w, LocusId(2), LocusId(0), trust, TransitiveRule::Product).is_none()
        );
    }

    #[test]
    fn infer_transitive_same_locus_returns_none() {
        use graph_core::InfluenceKindId;
        let w = trust_chain_world();
        let trust: graph_core::RelationshipKindId = InfluenceKindId(10);
        assert!(
            infer_transitive(&w, LocusId(0), LocusId(0), trust, TransitiveRule::Product).is_none()
        );
    }

    #[test]
    fn has_cycle_returns_false_for_dag() {
        let w = diamond_world();
        assert!(!has_cycle(&w));
    }

    #[test]
    fn has_cycle_returns_true_for_simple_cycle() {
        let lk = LocusKindId(1);
        let rk: RelationshipKindId = InfluenceKindId(1);
        let mut w = World::new();
        for i in 0u64..3 {
            w.insert_locus(Locus::new(LocusId(i), lk, StateVector::zeros(1)));
        }
        for (from, to) in [(0u64, 1), (1, 2), (2, 0)] {
            let id = w.relationships_mut().mint_id();
            w.relationships_mut().insert(Relationship {
                id,
                kind: rk,
                endpoints: Endpoints::Directed {
                    from: LocusId(from),
                    to: LocusId(to),
                },
                state: StateVector::from_slice(&[1.0, 0.0]),
                lineage: RelationshipLineage {
                    created_by: None,
                    last_touched_by: None,
                    change_count: 1,
                    kinds_observed: smallvec::smallvec![KindObservation::synthetic(rk)],
                },
                created_batch: graph_core::BatchId(0),
                last_decayed_batch: 0,
                metadata: None,
            });
        }
        assert!(has_cycle(&w));
    }

    #[test]
    fn has_cycle_ignores_symmetric_edges() {
        let lk = LocusKindId(1);
        let rk: RelationshipKindId = InfluenceKindId(1);
        let mut w = World::new();
        for i in 0u64..2 {
            w.insert_locus(Locus::new(LocusId(i), lk, StateVector::zeros(1)));
        }
        let id = w.relationships_mut().mint_id();
        w.relationships_mut().insert(Relationship {
            id,
            kind: rk,
            endpoints: Endpoints::Symmetric {
                a: LocusId(0),
                b: LocusId(1),
            },
            state: StateVector::from_slice(&[1.0, 0.0]),
            lineage: RelationshipLineage {
                created_by: None,
                last_touched_by: None,
                change_count: 1,
                kinds_observed: smallvec::smallvec![KindObservation::synthetic(rk)],
            },
            created_batch: graph_core::BatchId(0),
            last_decayed_batch: 0,
            metadata: None,
        });
        assert!(!has_cycle(&w));
    }

    #[test]
    fn has_cycle_empty_world_returns_false() {
        assert!(!has_cycle(&World::new()));
    }

    #[test]
    fn source_loci_in_chain() {
        let w = chain_world(4);
        let mut sources = source_loci(&w);
        sources.sort();
        assert_eq!(sources, vec![LocusId(0)]);
    }

    #[test]
    fn sink_loci_in_chain() {
        let w = chain_world(4);
        let mut sinks = sink_loci(&w);
        sinks.sort();
        assert_eq!(sinks, vec![LocusId(3)]);
    }

    #[test]
    fn source_and_sink_empty_for_cycle() {
        let lk = LocusKindId(1);
        let rk: RelationshipKindId = InfluenceKindId(1);
        let mut w = World::new();
        for i in 0u64..3 {
            w.insert_locus(Locus::new(LocusId(i), lk, StateVector::zeros(1)));
        }
        for (from, to) in [(0u64, 1), (1, 2), (2, 0)] {
            let id = w.relationships_mut().mint_id();
            w.relationships_mut().insert(Relationship {
                id,
                kind: rk,
                endpoints: Endpoints::Directed {
                    from: LocusId(from),
                    to: LocusId(to),
                },
                state: StateVector::from_slice(&[1.0, 0.0]),
                lineage: RelationshipLineage {
                    created_by: None,
                    last_touched_by: None,
                    change_count: 1,
                    kinds_observed: smallvec::smallvec![KindObservation::synthetic(rk)],
                },
                created_batch: graph_core::BatchId(0),
                last_decayed_batch: 0,
                metadata: None,
            });
        }
        assert!(source_loci(&w).is_empty());
        assert!(sink_loci(&w).is_empty());
    }

    fn activity_chain_world() -> World {
        let lk = LocusKindId(1);
        let rk: RelationshipKindId = InfluenceKindId(1);
        let mut w = World::new();
        for i in 0u64..4 {
            w.insert_locus(Locus::new(LocusId(i), lk, StateVector::zeros(1)));
        }
        for (from, to, activity) in [(0u64, 1u64, 0.8f32), (1, 2, 0.1), (2, 3, 0.8)] {
            let id = w.relationships_mut().mint_id();
            w.relationships_mut().insert(Relationship {
                id,
                kind: rk,
                endpoints: Endpoints::Directed {
                    from: LocusId(from),
                    to: LocusId(to),
                },
                state: StateVector::from_slice(&[activity, 0.0]),
                lineage: RelationshipLineage {
                    created_by: None,
                    last_touched_by: None,
                    change_count: 1,
                    kinds_observed: smallvec::smallvec![KindObservation::synthetic(rk)],
                },
                created_batch: graph_core::BatchId(0),
                last_decayed_batch: 0,
                metadata: None,
            });
        }
        w
    }

    #[test]
    fn reachable_from_active_skips_dormant_edges() {
        let w = activity_chain_world();
        let mut reach = reachable_from_active(&w, LocusId(0), 10, 0.5);
        reach.sort();
        assert_eq!(reach, vec![LocusId(1)]);
    }

    #[test]
    fn reachable_from_active_depth_zero_is_empty() {
        let w = activity_chain_world();
        assert!(reachable_from_active(&w, LocusId(0), 0, 0.5).is_empty());
    }

    #[test]
    fn reachable_from_active_zero_threshold_equals_reachable_from() {
        let w = activity_chain_world();
        let mut active = reachable_from_active(&w, LocusId(0), 10, 0.0);
        let mut standard = reachable_from(&w, LocusId(0), 10);
        active.sort();
        standard.sort();
        assert_eq!(active, standard);
    }

    #[test]
    fn downstream_of_active_skips_dormant_forward_edges() {
        let w = activity_chain_world();
        let mut ds = downstream_of_active(&w, LocusId(0), 10, 0.5);
        ds.sort();
        assert_eq!(ds, vec![LocusId(1)]);
    }

    #[test]
    fn upstream_of_active_skips_dormant_backward_edges() {
        let w = activity_chain_world();
        let mut us = upstream_of_active(&w, LocusId(3), 10, 0.5);
        us.sort();
        assert_eq!(us, vec![LocusId(2)]);
    }

    #[test]
    fn path_between_active_blocked_by_dormant_edge() {
        let w = activity_chain_world();
        assert!(path_between_active(&w, LocusId(0), LocusId(3), 0.5).is_none());
    }

    #[test]
    fn path_between_active_finds_path_at_zero_threshold() {
        let w = activity_chain_world();
        let path = path_between_active(&w, LocusId(0), LocusId(3), 0.0).unwrap();
        assert_eq!(path.first(), Some(&LocusId(0)));
        assert_eq!(path.last(), Some(&LocusId(3)));
    }

    #[test]
    fn path_between_active_same_locus_returns_singleton() {
        let w = activity_chain_world();
        assert_eq!(
            path_between_active(&w, LocusId(1), LocusId(1), 0.9),
            Some(vec![LocusId(1)])
        );
    }

    #[test]
    fn symmetric_edges_not_counted_as_directed_degree() {
        let lk = LocusKindId(1);
        let rk: RelationshipKindId = InfluenceKindId(1);
        let mut w = World::new();
        for i in 0u64..2 {
            w.insert_locus(Locus::new(LocusId(i), lk, StateVector::zeros(1)));
        }
        let id = w.relationships_mut().mint_id();
        w.relationships_mut().insert(Relationship {
            id,
            kind: rk,
            endpoints: Endpoints::Symmetric {
                a: LocusId(0),
                b: LocusId(1),
            },
            state: StateVector::from_slice(&[1.0, 0.0]),
            lineage: RelationshipLineage {
                created_by: None,
                last_touched_by: None,
                change_count: 1,
                kinds_observed: smallvec::smallvec![KindObservation::synthetic(rk)],
            },
            created_batch: graph_core::BatchId(0),
            last_decayed_batch: 0,
            metadata: None,
        });
        assert!(source_loci(&w).is_empty());
        assert!(sink_loci(&w).is_empty());
    }
}
