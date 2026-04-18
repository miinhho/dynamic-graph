//! Analytical centrality functions: Burt's structural constraint, structural
//! balance (Cartwright & Harary), Newman–Girvan modularity, betweenness
//! centrality (Brandes), and harmonic closeness centrality.
//!
//! All functions take `&World` and are read-only.

mod balance;
mod brandes;
mod brokerage;
mod community;
mod indexed;
mod modularity;
mod rankings;
mod traversal;

pub use self::balance::{
    TriangleBalance, all_triangles, balance_index, triangle_balance, unstable_triangles,
};
pub use self::brokerage::{all_constraints, effective_network_size, structural_constraint};
pub use self::community::{louvain, louvain_with_resolution};
pub use self::modularity::modularity;
pub use self::rankings::{
    all_betweenness, all_closeness, betweenness_centrality, closeness_centrality, pagerank,
    pagerank_centrality,
};

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use graph_core::{
        BatchId, Endpoints, InfluenceKindId, KindObservation, Locus, LocusId, LocusKindId,
        Relationship, RelationshipLineage, StateVector,
    };
    use graph_world::World;
    use smallvec::smallvec;

    const LK: LocusKindId = LocusKindId(1);
    const RK: InfluenceKindId = InfluenceKindId(1);

    fn make_locus(id: u64) -> Locus {
        Locus::new(LocusId(id), LK, StateVector::zeros(1))
    }

    /// Insert a symmetric (undirected) relationship with given activity+weight.
    fn add_sym_rel(world: &mut World, a: u64, b: u64, activity: f32, weight: f32) {
        let id = world.relationships_mut().mint_id();
        world.relationships_mut().insert(Relationship {
            id,
            kind: RK,
            endpoints: Endpoints::Symmetric {
                a: LocusId(a),
                b: LocusId(b),
            },
            state: StateVector::from_slice(&[activity, weight]),
            lineage: RelationshipLineage {
                created_by: None,
                last_touched_by: None,
                change_count: 1,
                kinds_observed: smallvec![KindObservation::synthetic(RK)],
            },
            created_batch: BatchId(0),
            last_decayed_batch: 0,
            metadata: None,
        });
    }

    // ── Star graph ────────────────────────────────────────────────────────────

    /// Star: hub=0, spokes=1,2,3.
    /// Hub bridges all spokes → low constraint.
    /// Spokes only connect to hub → high constraint.
    fn star_world() -> World {
        let mut w = World::new();
        for i in 0u64..4 {
            w.insert_locus(make_locus(i));
        }
        // All edges uniform strength 1.0 (activity=0.5, weight=0.5)
        for spoke in 1u64..4 {
            add_sym_rel(&mut w, 0, spoke, 0.5, 0.5);
        }
        w
    }

    #[test]
    fn star_hub_has_lower_constraint_than_spokes() {
        let w = star_world();
        let c_hub = structural_constraint(&w, LocusId(0)).expect("hub has rels");
        let c_spoke = structural_constraint(&w, LocusId(1)).expect("spoke has rels");
        assert!(
            c_hub < c_spoke,
            "hub constraint {c_hub} should be < spoke constraint {c_spoke}"
        );
    }

    #[test]
    fn star_spoke_no_rels_returns_none() {
        let mut w = World::new();
        w.insert_locus(make_locus(99));
        assert!(structural_constraint(&w, LocusId(99)).is_none());
    }

    #[test]
    fn all_constraints_sorted_ascending() {
        let w = star_world();
        let cs = all_constraints(&w);
        assert!(!cs.is_empty());
        for pair in cs.windows(2) {
            assert!(pair[0].1 <= pair[1].1, "not sorted: {:?}", cs);
        }
        // Hub should appear first (lowest constraint)
        assert_eq!(cs[0].0, LocusId(0));
    }

    #[test]
    fn star_effective_network_size_hub_near_three() {
        // Hub has 3 non-redundant contacts (spokes don't interconnect)
        let w = star_world();
        let ens = effective_network_size(&w, LocusId(0));
        // Should be close to 3.0 (each spoke is independent of the others)
        assert!(ens > 2.5 && ens <= 3.0, "expected ~3.0, got {ens}");
    }

    // ── Clique ────────────────────────────────────────────────────────────────

    /// Fully connected K4 clique: nodes 0,1,2,3 all connected.
    fn clique_world() -> World {
        let mut w = World::new();
        for i in 0u64..4 {
            w.insert_locus(make_locus(i));
        }
        for a in 0u64..4 {
            for b in (a + 1)..4 {
                add_sym_rel(&mut w, a, b, 0.5, 0.5);
            }
        }
        w
    }

    #[test]
    fn clique_all_nodes_high_constraint() {
        let w = clique_world();
        // In a clique all contacts are connected to each other → high constraint
        // Theoretical max for 3 contacts is 1.0; for 4-node clique it's ~1.125
        for i in 0u64..4 {
            let c = structural_constraint(&w, LocusId(i)).expect("clique node has rels");
            assert!(
                c > 0.5,
                "clique node {i} should have high constraint, got {c}"
            );
        }
    }

    #[test]
    fn clique_hub_has_higher_constraint_than_star_hub() {
        let star = star_world();
        let clique = clique_world();
        let c_star = structural_constraint(&star, LocusId(0)).unwrap();
        let c_clique = structural_constraint(&clique, LocusId(0)).unwrap();
        assert!(
            c_clique > c_star,
            "clique hub {c_clique} should be more constrained than star hub {c_star}"
        );
    }

    // ── Triangle balance ──────────────────────────────────────────────────────

    fn triangle_world(strength_ab: f32, strength_bc: f32, strength_ac: f32) -> World {
        let mut w = World::new();
        for i in 0u64..3 {
            w.insert_locus(make_locus(i));
        }
        add_sym_rel(&mut w, 0, 1, strength_ab / 2.0, strength_ab / 2.0);
        add_sym_rel(&mut w, 1, 2, strength_bc / 2.0, strength_bc / 2.0);
        add_sym_rel(&mut w, 0, 2, strength_ac / 2.0, strength_ac / 2.0);
        w
    }

    #[test]
    fn triangle_all_positive_is_balanced() {
        // +++ → product = +1 → balanced
        let w = triangle_world(1.0, 1.0, 1.0);
        let bal = triangle_balance(&w, LocusId(0), LocusId(1), LocusId(2), 0.0);
        assert_eq!(bal, Some(TriangleBalance::Balanced));
    }

    #[test]
    fn triangle_two_neg_one_pos_is_balanced() {
        // +-- → product = +1 → balanced
        let w = triangle_world(1.0, -0.5, -0.5);
        let bal = triangle_balance(&w, LocusId(0), LocusId(1), LocusId(2), 0.0);
        assert_eq!(bal, Some(TriangleBalance::Balanced));
    }

    #[test]
    fn triangle_one_neg_two_pos_is_unstable() {
        // ++- → product = -1 → unstable
        let w = triangle_world(1.0, 1.0, -0.5);
        let bal = triangle_balance(&w, LocusId(0), LocusId(1), LocusId(2), 0.0);
        assert_eq!(bal, Some(TriangleBalance::Unstable));
    }

    #[test]
    fn triangle_all_negative_is_unstable() {
        // --- → product = -1 → unstable
        let w = triangle_world(-1.0, -1.0, -1.0);
        let bal = triangle_balance(&w, LocusId(0), LocusId(1), LocusId(2), 0.0);
        assert_eq!(bal, Some(TriangleBalance::Unstable));
    }

    #[test]
    fn triangle_missing_edge_returns_none() {
        let mut w = World::new();
        for i in 0u64..3 {
            w.insert_locus(make_locus(i));
        }
        add_sym_rel(&mut w, 0, 1, 0.5, 0.5);
        add_sym_rel(&mut w, 1, 2, 0.5, 0.5);
        // No edge 0-2
        let bal = triangle_balance(&w, LocusId(0), LocusId(1), LocusId(2), 0.0);
        assert_eq!(bal, None);
    }

    #[test]
    fn all_triangles_clique_returns_four_triangles() {
        let w = clique_world();
        // K4 has C(4,3)=4 triangles
        let ts = all_triangles(&w);
        assert_eq!(ts.len(), 4, "K4 has 4 triangles, got: {:?}", ts);
    }

    #[test]
    fn all_triangles_dedup_sorted() {
        let w = clique_world();
        let ts = all_triangles(&w);
        for t in &ts {
            assert!(t.0 < t.1 && t.1 < t.2, "triangle not sorted: {:?}", t);
        }
        let mut sorted = ts.clone();
        sorted.sort();
        assert_eq!(ts, sorted, "triangles not in sorted order");
    }

    #[test]
    fn balance_index_all_positive_clique() {
        // All edges positive (strength 1.0 > threshold 0.0) → +++ for every triangle → 1.0
        let w = clique_world();
        let bi = balance_index(&w, 0.0);
        assert!((bi - 1.0).abs() < 1e-5, "expected 1.0, got {bi}");
    }

    #[test]
    fn balance_index_no_triangles_returns_zero() {
        let w = star_world(); // star has no triangles
        let bi = balance_index(&w, 0.0);
        assert_eq!(bi, 0.0);
    }

    #[test]
    fn unstable_triangles_detects_unstable() {
        // Build a triangle where one edge is negative (++-)
        let w = triangle_world(1.0, 1.0, -0.5);
        let us = unstable_triangles(&w, 0.0);
        assert_eq!(us.len(), 1);
    }

    // ── Modularity ────────────────────────────────────────────────────────────

    /// Two disconnected components: {0,1} and {2,3}, each fully connected.
    fn two_component_world() -> World {
        let mut w = World::new();
        for i in 0u64..4 {
            w.insert_locus(make_locus(i));
        }
        add_sym_rel(&mut w, 0, 1, 0.5, 0.5); // component A
        add_sym_rel(&mut w, 2, 3, 0.5, 0.5); // component B
        w
    }

    #[test]
    fn modularity_perfect_partition_near_one() {
        let w = two_component_world();
        let partition = vec![vec![LocusId(0), LocusId(1)], vec![LocusId(2), LocusId(3)]];
        let q = modularity(&w, &partition);
        // For two disconnected equal components, Q = 0.5
        assert!(q > 0.4, "expected Q near 0.5, got {q}");
    }

    #[test]
    fn modularity_single_community_near_zero() {
        let w = two_component_world();
        // Put all nodes in one group
        let partition = vec![vec![LocusId(0), LocusId(1), LocusId(2), LocusId(3)]];
        let q = modularity(&w, &partition);
        // Q ≈ 0 for a single-community partition
        assert!(q.abs() < 0.1, "expected Q near 0, got {q}");
    }

    #[test]
    fn modularity_empty_partition_returns_zero() {
        let w = two_component_world();
        let q = modularity(&w, &[]);
        assert_eq!(q, 0.0);
    }

    #[test]
    fn modularity_no_edges_returns_zero() {
        let mut w = World::new();
        for i in 0u64..4 {
            w.insert_locus(make_locus(i));
        }
        let partition = vec![vec![LocusId(0), LocusId(1)], vec![LocusId(2), LocusId(3)]];
        let q = modularity(&w, &partition);
        assert_eq!(q, 0.0);
    }

    #[test]
    fn modularity_wrong_partition_lower_than_correct() {
        let w = two_component_world();
        let correct = vec![vec![LocusId(0), LocusId(1)], vec![LocusId(2), LocusId(3)]];
        let wrong = vec![vec![LocusId(0), LocusId(2)], vec![LocusId(1), LocusId(3)]];
        let q_correct = modularity(&w, &correct);
        let q_wrong = modularity(&w, &wrong);
        assert!(
            q_correct > q_wrong,
            "correct partition Q={q_correct} should be > wrong Q={q_wrong}"
        );
    }

    // ── Betweenness centrality ─────────────────────────────────────────────────

    /// Path graph 0–1–2–3: locus 1 and 2 are on all shortest paths between the
    /// two halves → higher betweenness than the endpoints 0 and 3.
    fn path4_world() -> World {
        let mut w = World::new();
        for i in 0u64..4 {
            w.insert_locus(make_locus(i));
        }
        // Undirected chain: 0-1-2-3
        add_sym_rel(&mut w, 0, 1, 0.5, 0.5);
        add_sym_rel(&mut w, 1, 2, 0.5, 0.5);
        add_sym_rel(&mut w, 2, 3, 0.5, 0.5);
        w
    }

    #[test]
    fn betweenness_endpoints_are_zero() {
        let w = path4_world();
        let b0 = betweenness_centrality(&w, LocusId(0));
        let b3 = betweenness_centrality(&w, LocusId(3));
        assert_eq!(b0, 0.0, "endpoint 0 should have 0 betweenness");
        assert_eq!(b3, 0.0, "endpoint 3 should have 0 betweenness");
    }

    #[test]
    fn betweenness_inner_nodes_higher_than_endpoints() {
        let w = path4_world();
        let b1 = betweenness_centrality(&w, LocusId(1));
        let b2 = betweenness_centrality(&w, LocusId(2));
        let b0 = betweenness_centrality(&w, LocusId(0));
        assert!(b1 > b0, "node 1: {b1} should beat endpoint 0: {b0}");
        assert!(b2 > b0, "node 2: {b2} should beat endpoint 0: {b0}");
    }

    #[test]
    fn all_betweenness_sorted_descending() {
        let w = path4_world();
        let scores = all_betweenness(&w);
        assert_eq!(scores.len(), 4);
        for pair in scores.windows(2) {
            assert!(pair[0].1 >= pair[1].1, "not sorted desc: {:?}", scores);
        }
    }

    #[test]
    fn betweenness_star_hub_highest() {
        // In a star, the hub lies on every shortest path between spokes.
        let w = star_world();
        let b_hub = betweenness_centrality(&w, LocusId(0));
        for spoke in 1u64..4 {
            let b_spoke = betweenness_centrality(&w, LocusId(spoke));
            assert!(b_hub > b_spoke, "hub {b_hub} should beat spoke {b_spoke}");
        }
    }

    #[test]
    fn betweenness_small_world_returns_zero_for_missing_locus() {
        let mut w = World::new();
        w.insert_locus(make_locus(0));
        // < 3 loci → everyone is 0.0
        assert_eq!(betweenness_centrality(&w, LocusId(0)), 0.0);
    }

    // ── Closeness centrality ──────────────────────────────────────────────────

    #[test]
    fn closeness_single_locus_returns_none() {
        let mut w = World::new();
        w.insert_locus(make_locus(0));
        assert!(closeness_centrality(&w, LocusId(0)).is_none());
    }

    #[test]
    fn closeness_isolated_locus_is_zero() {
        let mut w = World::new();
        w.insert_locus(make_locus(0));
        w.insert_locus(make_locus(1));
        // No edges → harmonic sum = 0
        let c = closeness_centrality(&w, LocusId(0)).unwrap();
        assert_eq!(c, 0.0);
    }

    #[test]
    fn closeness_hub_higher_than_spoke_in_star() {
        let w = star_world();
        let c_hub = closeness_centrality(&w, LocusId(0)).unwrap();
        let c_spoke = closeness_centrality(&w, LocusId(1)).unwrap();
        assert!(c_hub > c_spoke, "hub {c_hub} should beat spoke {c_spoke}");
    }

    #[test]
    fn all_closeness_sorted_descending() {
        let w = star_world();
        let scores = all_closeness(&w);
        assert_eq!(scores.len(), 4);
        for pair in scores.windows(2) {
            assert!(pair[0].1 >= pair[1].1, "not sorted desc: {:?}", scores);
        }
        // Hub should appear first
        assert_eq!(scores[0].0, LocusId(0));
    }

    #[test]
    fn all_closeness_empty_for_one_locus() {
        let mut w = World::new();
        w.insert_locus(make_locus(0));
        assert!(all_closeness(&w).is_empty());
    }

    // ── PageRank ──────────────────────────────────────────────────────────────

    fn add_dir_rel(world: &mut World, from: u64, to: u64, activity: f32) {
        let id = world.relationships_mut().mint_id();
        world.relationships_mut().insert(Relationship {
            id,
            kind: RK,
            endpoints: graph_core::Endpoints::Directed {
                from: LocusId(from),
                to: LocusId(to),
            },
            state: StateVector::from_slice(&[activity, 0.5]),
            lineage: RelationshipLineage {
                created_by: None,
                last_touched_by: None,
                change_count: 1,
                kinds_observed: smallvec![KindObservation::synthetic(RK)],
            },
            created_batch: BatchId(0),
            last_decayed_batch: 0,
            metadata: None,
        });
    }

    #[test]
    fn pagerank_empty_world_returns_empty() {
        assert!(pagerank(&World::new(), 0.85, 100, 1e-6).is_empty());
    }

    #[test]
    fn pagerank_scores_sum_to_one() {
        let w = star_world(); // undirected star → symmetric edges
        let scores = pagerank(&w, 0.85, 100, 1e-6);
        let total: f32 = scores.iter().map(|(_, v)| v).sum();
        assert!(
            (total - 1.0).abs() < 1e-4,
            "scores should sum to 1, got {total}"
        );
    }

    #[test]
    fn pagerank_hub_ranks_first_in_star() {
        let w = star_world();
        let scores = pagerank(&w, 0.85, 100, 1e-6);
        // Hub (locus 0) should have the highest PageRank in a star graph.
        assert_eq!(scores[0].0, LocusId(0), "hub should rank first: {scores:?}");
    }

    #[test]
    fn pagerank_sorted_descending() {
        let w = star_world();
        let scores = pagerank(&w, 0.85, 100, 1e-6);
        for pair in scores.windows(2) {
            assert!(pair[0].1 >= pair[1].1, "not sorted desc: {:?}", scores);
        }
    }

    #[test]
    fn pagerank_sink_accumulates_more_rank() {
        // Chain 0→1→2→3: locus 3 receives flow from all upstream nodes.
        let mut w = World::new();
        for i in 0u64..4 {
            w.insert_locus(make_locus(i));
        }
        for i in 0u64..3 {
            add_dir_rel(&mut w, i, i + 1, 1.0);
        }
        let scores = pagerank(&w, 0.85, 100, 1e-6);
        // Sink node 3 should rank higher than source node 0.
        let pr0 = scores.iter().find(|(id, _)| *id == LocusId(0)).unwrap().1;
        let pr3 = scores.iter().find(|(id, _)| *id == LocusId(3)).unwrap().1;
        assert!(pr3 > pr0, "sink {pr3} should beat source {pr0}");
    }

    #[test]
    fn pagerank_centrality_single_locus_default_params() {
        let w = star_world();
        let pr_hub = pagerank_centrality(&w, LocusId(0));
        let pr_spoke = pagerank_centrality(&w, LocusId(1));
        assert!(pr_hub > pr_spoke, "hub {pr_hub} > spoke {pr_spoke}");
    }

    #[test]
    fn pagerank_centrality_missing_locus_returns_zero() {
        let w = star_world();
        assert_eq!(pagerank_centrality(&w, LocusId(99)), 0.0);
    }

    // ── Louvain community detection ───────────────────────────────────────────

    /// Two disconnected cliques: {0,1,2} and {3,4,5}.
    /// Louvain should recover both communities.
    fn two_clique_world() -> World {
        let mut w = World::new();
        for i in 0u64..6 {
            w.insert_locus(make_locus(i));
        }
        // Clique A: 0-1, 1-2, 0-2
        for (a, b) in [(0u64, 1), (1, 2), (0, 2)] {
            add_sym_rel(&mut w, a, b, 0.5, 0.5);
        }
        // Clique B: 3-4, 4-5, 3-5
        for (a, b) in [(3u64, 4), (4, 5), (3, 5)] {
            add_sym_rel(&mut w, a, b, 0.5, 0.5);
        }
        w
    }

    #[test]
    fn louvain_empty_world_returns_empty() {
        assert!(louvain(&World::new()).is_empty());
    }

    #[test]
    fn louvain_no_edges_each_node_is_own_community() {
        let mut w = World::new();
        for i in 0u64..4 {
            w.insert_locus(make_locus(i));
        }
        let comms = louvain(&w);
        assert_eq!(comms.len(), 4, "4 isolated nodes → 4 singleton communities");
        for c in &comms {
            assert_eq!(c.len(), 1);
        }
    }

    #[test]
    fn louvain_two_cliques_recovers_both_groups() {
        let w = two_clique_world();
        let comms = louvain(&w);
        // Should find exactly 2 communities of size 3 each.
        assert_eq!(comms.len(), 2, "expected 2 communities, got {comms:?}");
        let mut sizes: Vec<usize> = comms.iter().map(Vec::len).collect();
        sizes.sort();
        assert_eq!(sizes, vec![3, 3]);

        // Nodes within each clique should be grouped together.
        let clique_a: Vec<LocusId> = vec![LocusId(0), LocusId(1), LocusId(2)];
        let clique_b: Vec<LocusId> = vec![LocusId(3), LocusId(4), LocusId(5)];
        assert!(
            comms.iter().any(|c| c == &clique_a),
            "clique A not found in {comms:?}"
        );
        assert!(
            comms.iter().any(|c| c == &clique_b),
            "clique B not found in {comms:?}"
        );
    }

    #[test]
    fn louvain_all_nodes_covered() {
        let w = two_clique_world();
        let comms = louvain(&w);
        let mut all_nodes: Vec<LocusId> = comms.into_iter().flatten().collect();
        all_nodes.sort();
        let expected: Vec<LocusId> = (0u64..6).map(LocusId).collect();
        assert_eq!(all_nodes, expected, "every node must appear exactly once");
    }

    #[test]
    fn louvain_modularity_partition_beats_single_community() {
        let w = two_clique_world();
        let comms = louvain(&w);
        let q_louvain = modularity(&w, &comms);
        let all_together = vec![(0u64..6).map(LocusId).collect::<Vec<_>>()];
        let q_single = modularity(&w, &all_together);
        assert!(
            q_louvain > q_single,
            "Louvain Q={q_louvain} should beat single-community Q={q_single}"
        );
    }

    #[test]
    fn louvain_high_resolution_splits_more() {
        // Bridge graph: two cliques connected by a single weak bridge.
        let mut w = World::new();
        for i in 0u64..6 {
            w.insert_locus(make_locus(i));
        }
        for (a, b) in [(0u64, 1), (1, 2), (0, 2)] {
            add_sym_rel(&mut w, a, b, 1.0, 0.0);
        }
        for (a, b) in [(3u64, 4), (4, 5), (3, 5)] {
            add_sym_rel(&mut w, a, b, 1.0, 0.0);
        }
        // Weak bridge between the two cliques
        add_sym_rel(&mut w, 2, 3, 0.1, 0.0);

        let comms_default = louvain(&w);
        let comms_high_res = louvain_with_resolution(&w, 3.0);
        // Higher resolution should find at least as many communities.
        assert!(
            comms_high_res.len() >= comms_default.len(),
            "high resolution should not merge more: default={} high_res={}",
            comms_default.len(),
            comms_high_res.len(),
        );
    }
}
