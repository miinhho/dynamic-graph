use super::*;
use graph_core::{
    Endpoints, InfluenceKindId, KindObservation, Locus, LocusKindId, Relationship,
    RelationshipKindId, RelationshipLineage, StateVector,
};
use graph_world::World;

fn make_world() -> World {
    let lk_a = LocusKindId(1);
    let lk_b = LocusKindId(2);
    let rk: RelationshipKindId = InfluenceKindId(1);
    let mut w = World::new();

    w.insert_locus(Locus::new(
        graph_core::LocusId(0),
        lk_a,
        StateVector::from_slice(&[0.9]),
    ));
    w.insert_locus(Locus::new(
        graph_core::LocusId(1),
        lk_a,
        StateVector::from_slice(&[0.3]),
    ));
    w.insert_locus(Locus::new(
        graph_core::LocusId(2),
        lk_b,
        StateVector::from_slice(&[0.7]),
    ));

    w.properties_mut().insert(
        graph_core::LocusId(0),
        graph_core::props! {
            "type" => "ORG",
            "score" => 0.9_f64,
        },
    );
    w.properties_mut().insert(
        graph_core::LocusId(1),
        graph_core::props! {
            "type" => "PERSON",
            "score" => 0.3_f64,
        },
    );

    let id = w.relationships_mut().mint_id();
    w.relationships_mut().insert(Relationship {
        id,
        kind: rk,
        endpoints: Endpoints::Directed {
            from: graph_core::LocusId(0),
            to: graph_core::LocusId(1),
        },
        state: StateVector::from_slice(&[0.8, 0.5, 0.2]),
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
    w
}

#[test]
fn loci_of_kind_filters() {
    let w = make_world();
    let kind_a = loci_of_kind(&w, LocusKindId(1));
    assert_eq!(kind_a.len(), 2);
    let kind_b = loci_of_kind(&w, LocusKindId(2));
    assert_eq!(kind_b.len(), 1);
}

#[test]
fn loci_with_state_filters_by_slot() {
    let w = make_world();
    let high = loci_with_state(&w, 0, |v| v > 0.5);
    assert_eq!(high.len(), 2);
    let low = loci_with_state(&w, 0, |v| v < 0.5);
    assert_eq!(low.len(), 1);
}

#[test]
fn loci_with_str_property_filters() {
    let w = make_world();
    let orgs = loci_with_str_property(&w, "type", |v| v == "ORG");
    assert_eq!(orgs.len(), 1);
    assert_eq!(orgs[0].id, graph_core::LocusId(0));
}

#[test]
fn loci_with_f64_property_filters() {
    let w = make_world();
    let high_score = loci_with_f64_property(&w, "score", |v| v > 0.5);
    assert_eq!(high_score.len(), 1);
}

#[test]
fn relationships_with_activity_filters() {
    let w = make_world();
    let active = relationships_with_activity(&w, |a| a > 0.5);
    assert_eq!(active.len(), 1);
    let none = relationships_with_activity(&w, |a| a > 0.9);
    assert!(none.is_empty());
}

#[test]
fn relationships_with_slot_filters_extra_slot() {
    let w = make_world();
    let found = relationships_with_slot(&w, 2, |v| v > 0.1);
    assert_eq!(found.len(), 1);
    let not_found = relationships_with_slot(&w, 2, |v| v > 0.5);
    assert!(not_found.is_empty());
}

#[test]
fn relationships_of_kind_filters() {
    let w = make_world();
    let rk = InfluenceKindId(1);
    assert_eq!(relationships_of_kind(&w, rk).len(), 1);
    assert!(relationships_of_kind(&w, InfluenceKindId(99)).is_empty());
}

fn make_world_with_entities() -> World {
    use graph_core::{BatchId, Entity, LocusId};
    let mut w = World::new();
    let lk = LocusKindId(1);
    w.insert_locus(Locus::new(LocusId(0), lk, StateVector::from_slice(&[0.5])));
    w.insert_locus(Locus::new(LocusId(1), lk, StateVector::from_slice(&[0.8])));
    w.insert_locus(Locus::new(LocusId(2), lk, StateVector::from_slice(&[0.3])));

    let e0 = w.entities_mut().mint_id();
    w.entities_mut().insert(Entity::born(
        e0,
        BatchId(1),
        graph_core::EntitySnapshot {
            members: vec![LocusId(0), LocusId(1)],
            member_relationships: vec![],
            coherence: 0.8,
        },
    ));
    let e1 = w.entities_mut().mint_id();
    w.entities_mut().insert(Entity::born(
        e1,
        BatchId(2),
        graph_core::EntitySnapshot {
            members: vec![LocusId(2)],
            member_relationships: vec![],
            coherence: 0.2,
        },
    ));
    w
}

#[test]
fn active_entities_returns_all() {
    let w = make_world_with_entities();
    assert_eq!(active_entities(&w).len(), 2);
}

#[test]
fn entities_with_member_finds_correct_entity() {
    let w = make_world_with_entities();
    use graph_core::LocusId;
    let found = entities_with_member(&w, LocusId(1));
    assert_eq!(found.len(), 1);
    assert!(found[0].current.members.contains(&LocusId(1)));

    let not_found = entities_with_member(&w, LocusId(99));
    assert!(not_found.is_empty());
}

#[test]
fn entities_with_coherence_filters() {
    let w = make_world_with_entities();
    let high = entities_with_coherence(&w, |c| c > 0.5);
    assert_eq!(high.len(), 1);
    assert!((high[0].current.coherence - 0.8).abs() < 1e-5);

    let low = entities_with_coherence(&w, |c| c <= 0.5);
    assert_eq!(low.len(), 1);
}

#[test]
fn entities_matching_custom_pred() {
    let w = make_world_with_entities();
    let large = entities_matching(&w, |e| e.current.members.len() >= 2);
    assert_eq!(large.len(), 1);
    assert_eq!(large[0].current.members.len(), 2);
}

fn directed_world() -> World {
    use graph_core::{Endpoints, LocusId};
    let mut w = World::new();
    w.add_relationship(
        Endpoints::directed(LocusId(0), LocusId(1)),
        InfluenceKindId(1),
        StateVector::from_slice(&[1.0, 0.0]),
    );
    w.add_relationship(
        Endpoints::directed(LocusId(2), LocusId(1)),
        InfluenceKindId(1),
        StateVector::from_slice(&[1.0, 0.0]),
    );
    w.add_relationship(
        Endpoints::directed(LocusId(0), LocusId(2)),
        InfluenceKindId(2),
        StateVector::from_slice(&[1.0, 0.0]),
    );
    w
}

#[test]
fn relationships_from_returns_outgoing_only() {
    use graph_core::LocusId;
    let w = directed_world();
    let out = relationships_from(&w, LocusId(0));
    assert_eq!(out.len(), 2);
    let none = relationships_from(&w, LocusId(1));
    assert!(none.is_empty());
}

#[test]
fn relationships_to_returns_incoming_only() {
    use graph_core::LocusId;
    let w = directed_world();
    let inc = relationships_to(&w, LocusId(1));
    assert_eq!(inc.len(), 2);
    let none = relationships_to(&w, LocusId(0));
    assert!(none.is_empty());
}

#[test]
fn relationships_between_returns_all_kinds() {
    use graph_core::LocusId;
    let w = directed_world();
    let between01 = relationships_between(&w, LocusId(0), LocusId(1));
    assert_eq!(between01.len(), 1);
    let between02 = relationships_between(&w, LocusId(0), LocusId(2));
    assert_eq!(between02.len(), 1);
    let between12 = relationships_between(&w, LocusId(1), LocusId(2));
    assert_eq!(between12.len(), 1);
    let none = relationships_between(&w, LocusId(0), LocusId(99));
    assert!(none.is_empty());
}

#[test]
fn relationships_between_of_kind_filters_kind() {
    use graph_core::LocusId;
    let mut w = directed_world();
    w.add_relationship(
        graph_core::Endpoints::directed(LocusId(0), LocusId(1)),
        InfluenceKindId(2),
        StateVector::from_slice(&[1.0, 0.0]),
    );
    let kind1 = relationships_between_of_kind(&w, LocusId(0), LocusId(1), InfluenceKindId(1));
    assert_eq!(kind1.len(), 1);
    let kind2 = relationships_between_of_kind(&w, LocusId(0), LocusId(1), InfluenceKindId(2));
    assert_eq!(kind2.len(), 1);
    let kind99 = relationships_between_of_kind(&w, LocusId(0), LocusId(1), InfluenceKindId(99));
    assert!(kind99.is_empty());
}

fn strength_world() -> World {
    use graph_core::{Endpoints, LocusId};
    let mut w = World::new();
    let rk = InfluenceKindId(1);
    for i in 0u64..4 {
        w.insert_locus(graph_core::Locus::new(
            LocusId(i),
            LocusKindId(1),
            StateVector::from_slice(&[0.5]),
        ));
    }
    w.add_relationship(
        Endpoints::directed(LocusId(0), LocusId(1)),
        rk,
        StateVector::from_slice(&[0.8, 0.2]),
    );
    w.add_relationship(
        Endpoints::directed(LocusId(1), LocusId(2)),
        rk,
        StateVector::from_slice(&[0.3, 0.1]),
    );
    w.add_relationship(
        Endpoints::directed(LocusId(2), LocusId(3)),
        rk,
        StateVector::from_slice(&[0.5, 0.6]),
    );
    w
}

#[test]
fn relationships_above_strength_filters() {
    let w = strength_world();
    let above = relationships_above_strength(&w, 1.0);
    assert_eq!(above.len(), 1);
    assert!((above[0].strength() - 1.1).abs() < 1e-5);

    let none = relationships_above_strength(&w, 2.0);
    assert!(none.is_empty());
}

#[test]
fn relationships_top_n_by_strength_is_sorted() {
    let w = strength_world();
    let top2 = relationships_top_n_by_strength(&w, 2);
    assert_eq!(top2.len(), 2);
    assert!(top2[0].strength() >= top2[1].strength());
    assert!((top2[0].strength() - 1.1).abs() < 1e-5);
    assert!((top2[1].strength() - 1.0).abs() < 1e-5);

    let all = relationships_top_n_by_strength(&w, 100);
    assert_eq!(all.len(), 3);

    let zero = relationships_top_n_by_strength(&w, 0);
    assert!(zero.is_empty());
}

#[test]
fn relationships_idle_for_filters_by_last_decayed_batch() {
    use graph_core::BatchId;
    let w = strength_world();
    let idle = relationships_idle_for(&w, BatchId(10), 5);
    assert_eq!(idle.len(), 3);
    let none = relationships_idle_for(&w, BatchId(10), 11);
    assert!(none.is_empty());
}

#[test]
fn lookup_loci_resolves_ids_to_references() {
    use graph_core::LocusId;
    let w = make_world();
    let ids = vec![LocusId(0), LocusId(2), LocusId(99)];
    let loci = lookup_loci(&w, &ids);
    assert_eq!(loci.len(), 2);
    assert_eq!(loci[0].id, LocusId(0));
    assert_eq!(loci[1].id, LocusId(2));
}

#[test]
fn lookup_relationships_resolves_ids() {
    use graph_core::RelationshipId;
    let w = directed_world();
    let ids: Vec<RelationshipId> = w.relationships().iter().map(|r| r.id).collect();
    let rels = lookup_relationships(&w, &ids);
    assert_eq!(rels.len(), ids.len());
    let with_bad = lookup_relationships(&w, &[RelationshipId(999)]);
    assert!(with_bad.is_empty());
}

#[test]
fn relationship_touch_rate_is_zero_for_new_relationship() {
    use graph_core::BatchId;
    let w = directed_world();
    let rel = w.relationships().iter().next().unwrap();
    assert_eq!(relationship_touch_rate(&w, rel.id, BatchId(0)), 0.0);
}

#[test]
fn relationship_touch_rate_scales_with_touches() {
    use graph_core::{BatchId, LocusId};
    let mut w = World::new();
    let rk = InfluenceKindId(1);
    w.insert_locus(graph_core::Locus::new(
        LocusId(0),
        LocusKindId(1),
        StateVector::from_slice(&[0.5]),
    ));
    w.insert_locus(graph_core::Locus::new(
        LocusId(1),
        LocusKindId(1),
        StateVector::from_slice(&[0.5]),
    ));

    let id = w.relationships_mut().mint_id();
    w.relationships_mut().insert(Relationship {
        id,
        kind: rk,
        endpoints: Endpoints::directed(LocusId(0), LocusId(1)),
        state: StateVector::from_slice(&[0.5, 0.0]),
        lineage: RelationshipLineage {
            created_by: None,
            last_touched_by: None,
            change_count: 6,
            kinds_observed: smallvec::smallvec![KindObservation::synthetic(rk)],
        },
        created_batch: BatchId(0),
        last_decayed_batch: 0,
        metadata: None,
    });
    let rate = relationship_touch_rate(&w, id, BatchId(12));
    assert!((rate - 0.5).abs() < 1e-5, "expected 0.5, got {rate}");
}

fn degree_world() -> World {
    use graph_core::{Endpoints, LocusId};
    let mut w = World::new();
    let rk = InfluenceKindId(1);
    for i in 0u64..5 {
        w.insert_locus(graph_core::Locus::new(
            LocusId(i),
            LocusKindId(1),
            StateVector::from_slice(&[0.5]),
        ));
    }
    w.add_relationship(
        Endpoints::directed(LocusId(0), LocusId(1)),
        rk,
        StateVector::from_slice(&[1.0, 0.0]),
    );
    w.add_relationship(
        Endpoints::directed(LocusId(0), LocusId(2)),
        rk,
        StateVector::from_slice(&[1.0, 0.0]),
    );
    w.add_relationship(
        Endpoints::directed(LocusId(3), LocusId(1)),
        rk,
        StateVector::from_slice(&[1.0, 0.0]),
    );
    w
}

#[test]
fn locus_degree_counts_all_edges() {
    use graph_core::LocusId;
    let w = degree_world();
    assert_eq!(locus_degree(&w, LocusId(0)), 2);
    assert_eq!(locus_degree(&w, LocusId(1)), 2);
    assert_eq!(locus_degree(&w, LocusId(4)), 0);
}

#[test]
fn locus_in_out_degree_are_directional() {
    use graph_core::LocusId;
    let w = degree_world();
    assert_eq!(locus_in_degree(&w, LocusId(0)), 0);
    assert_eq!(locus_out_degree(&w, LocusId(0)), 2);
    assert_eq!(locus_in_degree(&w, LocusId(1)), 2);
    assert_eq!(locus_out_degree(&w, LocusId(1)), 0);
}

#[test]
fn most_connected_loci_returns_top_n_by_degree() {
    use graph_core::LocusId;
    let w = degree_world();
    let top1 = most_connected_loci(&w, 1);
    assert_eq!(top1.len(), 1);
    assert!(top1[0] == LocusId(0) || top1[0] == LocusId(1));

    let top4 = most_connected_loci(&w, 4);
    assert_eq!(top4.len(), 4);

    let zero = most_connected_loci(&w, 0);
    assert!(zero.is_empty());
}

fn world_with_metadata_rels() -> World {
    use graph_core::{InfluenceKindId, LocusId, Properties};
    let mut w = World::new();
    let a = LocusId(0);
    let b = LocusId(1);
    let c = LocusId(2);
    let kind = InfluenceKindId(1);
    let rel_ab = w.add_relationship(
        Endpoints::directed(a, b),
        kind,
        StateVector::from_slice(&[1.0, 0.0]),
    );
    {
        let mut props = Properties::new();
        props.set("type", "trust");
        props.set("confidence", 0.9f64);
        w.relationships_mut().get_mut(rel_ab).unwrap().metadata = Some(props);
    }
    let rel_bc = w.add_relationship(
        Endpoints::directed(b, c),
        kind,
        StateVector::from_slice(&[1.0, 0.0]),
    );
    {
        let mut props = Properties::new();
        props.set("type", "inhibit");
        props.set("confidence", 0.4f64);
        w.relationships_mut().get_mut(rel_bc).unwrap().metadata = Some(props);
    }
    w.add_relationship(
        Endpoints::directed(a, c),
        kind,
        StateVector::from_slice(&[1.0, 0.0]),
    );
    w
}

#[test]
fn relationships_with_str_property_filters_by_type() {
    let w = world_with_metadata_rels();
    let trust = relationships_with_str_property(&w, "type", |v| v == "trust");
    assert_eq!(trust.len(), 1);
    assert_eq!(trust[0].get_str_property("type"), Some("trust"));

    let inhibit = relationships_with_str_property(&w, "type", |v| v == "inhibit");
    assert_eq!(inhibit.len(), 1);

    let all = relationships_with_str_property(&w, "type", |_| true);
    assert_eq!(all.len(), 2);
}

#[test]
fn relationships_with_f64_property_filters_by_confidence() {
    let w = world_with_metadata_rels();
    let high = relationships_with_f64_property(&w, "confidence", |v| v >= 0.8);
    assert_eq!(high.len(), 1);
    assert!((high[0].get_f64_property("confidence").unwrap() - 0.9).abs() < 1e-9);

    let low = relationships_with_f64_property(&w, "confidence", |v| v < 0.5);
    assert_eq!(low.len(), 1);
}

#[test]
fn relationships_metadata_absent_excluded() {
    let w = world_with_metadata_rels();
    let typed = relationships_with_str_property(&w, "type", |_| true);
    assert_eq!(typed.len(), 2);
}

#[test]
fn relationships_of_kinds_empty_set_returns_empty() {
    let mut w = World::new();
    w.add_relationship(
        Endpoints::directed(graph_core::LocusId(0), graph_core::LocusId(1)),
        InfluenceKindId(1),
        StateVector::from_slice(&[1.0, 0.0]),
    );
    assert!(relationships_of_kinds(&w, &[]).is_empty());
}

#[test]
fn relationships_of_kinds_matches_multiple() {
    let mut w = World::new();
    let k1 = InfluenceKindId(1);
    let k2 = InfluenceKindId(2);
    let k3 = InfluenceKindId(3);
    w.add_relationship(
        Endpoints::directed(graph_core::LocusId(0), graph_core::LocusId(1)),
        k1,
        StateVector::from_slice(&[1.0, 0.0]),
    );
    w.add_relationship(
        Endpoints::directed(graph_core::LocusId(1), graph_core::LocusId(2)),
        k2,
        StateVector::from_slice(&[1.0, 0.0]),
    );
    w.add_relationship(
        Endpoints::directed(graph_core::LocusId(2), graph_core::LocusId(3)),
        k3,
        StateVector::from_slice(&[1.0, 0.0]),
    );

    let rels = relationships_of_kinds(&w, &[k1, k2]);
    assert_eq!(rels.len(), 2);
    assert!(rels.iter().all(|r| r.kind == k1 || r.kind == k2));
}

#[test]
fn net_influence_between_no_relationship_is_zero() {
    let w = World::new();
    let net = net_influence_between(
        &w,
        graph_core::LocusId(0),
        graph_core::LocusId(1),
        |_, _| None,
    );
    assert_eq!(net, 0.0);
}

#[test]
fn net_influence_between_single_kind_sums_activity() {
    let mut w = World::new();
    let k = InfluenceKindId(1);
    w.add_relationship(
        Endpoints::directed(graph_core::LocusId(0), graph_core::LocusId(1)),
        k,
        StateVector::from_slice(&[2.0, 0.0]),
    );
    w.add_relationship(
        Endpoints::directed(graph_core::LocusId(1), graph_core::LocusId(0)),
        k,
        StateVector::from_slice(&[1.5, 0.0]),
    );

    let net = net_influence_between(
        &w,
        graph_core::LocusId(0),
        graph_core::LocusId(1),
        |_, _| None,
    );
    assert!((net - 3.5).abs() < 1e-5, "expected 3.5, got {net}");
}

#[test]
fn net_influence_between_synergistic_interaction_applies_boost() {
    let mut w = World::new();
    let excite = InfluenceKindId(1);
    let dopamine = InfluenceKindId(2);
    w.add_relationship(
        Endpoints::directed(graph_core::LocusId(0), graph_core::LocusId(1)),
        excite,
        StateVector::from_slice(&[1.0, 0.0]),
    );
    w.add_relationship(
        Endpoints::directed(graph_core::LocusId(0), graph_core::LocusId(1)),
        dopamine,
        StateVector::from_slice(&[1.0, 0.0]),
    );

    let net = net_influence_between(
        &w,
        graph_core::LocusId(0),
        graph_core::LocusId(1),
        |ka, kb| {
            if (ka == excite && kb == dopamine) || (ka == dopamine && kb == excite) {
                Some(graph_core::InteractionEffect::Synergistic { boost: 1.5 })
            } else {
                None
            }
        },
    );
    assert!((net - 3.0).abs() < 1e-5, "expected 3.0, got {net}");
}

#[test]
fn net_influence_between_antagonistic_interaction_dampens() {
    let mut w = World::new();
    let excite = InfluenceKindId(1);
    let inhibit = InfluenceKindId(2);
    w.add_relationship(
        Endpoints::directed(graph_core::LocusId(0), graph_core::LocusId(1)),
        excite,
        StateVector::from_slice(&[1.0, 0.0]),
    );
    w.add_relationship(
        Endpoints::directed(graph_core::LocusId(0), graph_core::LocusId(1)),
        inhibit,
        StateVector::from_slice(&[1.0, 0.0]),
    );

    let net = net_influence_between(
        &w,
        graph_core::LocusId(0),
        graph_core::LocusId(1),
        |_, _| Some(graph_core::InteractionEffect::Antagonistic { dampen: 0.5 }),
    );
    assert!((net - 1.0).abs() < 1e-5, "expected 1.0, got {net}");
}
