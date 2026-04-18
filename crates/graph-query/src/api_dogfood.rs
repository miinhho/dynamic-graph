//! Dogfood: 실제 예시 도메인 패턴을 serializable API로 재현하는 테스트.
//!
//! 이전에 발견한 문제들이 해결됐는지 확인한다.

#[cfg(test)]
mod dogfood {
    use graph_core::{
        BatchId, Endpoints, InfluenceKindId, KindObservation, Locus, LocusId, LocusKindId,
        Relationship, RelationshipLineage, StateVector,
    };
    use graph_world::World;
    use smallvec::smallvec;

    use crate::query_api::{
        EntityPredicate, EntitySort, LocusPredicate, LocusSort, Query, QueryResult, RelSort,
        RelationshipPredicate, execute,
    };

    const KIND_SUPPLY: InfluenceKindId = InfluenceKindId(2);
    const KIND_ORDER: InfluenceKindId = InfluenceKindId(1);

    const SUPPLIER_A: LocusId = LocusId(1);
    const SUPPLIER_B: LocusId = LocusId(2);
    const FACTORY: LocusId = LocusId(3);
    const WAREHOUSE: LocusId = LocusId(4);

    fn supply_world() -> World {
        let mut w = World::new();
        let lkind = LocusKindId(1);
        for (id, stock) in [
            (SUPPLIER_A, 0.9f32),
            (SUPPLIER_B, 0.3f32),
            (FACTORY, 0.5f32),
            (WAREHOUSE, 1.4f32),
        ] {
            w.insert_locus(Locus::new(id, lkind, StateVector::from_slice(&[stock])));
        }
        let edges = [
            (SUPPLIER_A, FACTORY, KIND_SUPPLY, 0.80f32, 0.60f32, 6u64),
            (SUPPLIER_B, FACTORY, KIND_SUPPLY, 0.20f32, 0.15f32, 2u64),
            (FACTORY, WAREHOUSE, KIND_SUPPLY, 0.70f32, 0.55f32, 5u64),
            (FACTORY, SUPPLIER_A, KIND_ORDER, 0.30f32, 0.20f32, 3u64),
        ];
        for (from, to, kind, activity, weight, touches) in edges {
            let id = w.relationships_mut().mint_id();
            w.relationships_mut().insert(Relationship {
                id,
                kind,
                endpoints: Endpoints::directed(from, to),
                state: StateVector::from_slice(&[activity, weight]),
                lineage: RelationshipLineage {
                    created_by: None,
                    last_touched_by: None,
                    change_count: touches,
                    kinds_observed: smallvec![KindObservation::synthetic(kind)],
                },
                created_batch: BatchId(0),
                last_decayed_batch: 0,
                metadata: None,
            });
        }
        w
    }

    // ══════════════════════════════════════════════════════════════════════════
    // ✅ P0 해결: Top-N + 정렬 — celegans 상위 10개 synapse 패턴
    // ══════════════════════════════════════════════════════════════════════════

    #[test]
    fn top_n_active_rels_now_sortable() {
        let w = supply_world();

        // celegans 예시 패턴: "activity 기준 상위 10개 관계"
        let result = execute(
            &w,
            &Query::FindRelationships {
                predicates: vec![],
                sort_by: Some(RelSort::ActivityDesc),
                limit: Some(3),
            },
        );
        match result {
            QueryResult::RelationshipSummaries(rows) => {
                assert_eq!(rows.len(), 3);
                // 내림차순 정렬 검증
                assert!(rows[0].activity >= rows[1].activity);
                assert!(rows[1].activity >= rows[2].activity);
                // 상위 1등은 SUPPLIER_A→FACTORY (0.80)
                assert_eq!(rows[0].from, SUPPLIER_A);
                assert_eq!(rows[0].to, FACTORY);
            }
            _ => panic!("expected RelationshipSummaries"),
        }
    }

    #[test]
    fn top_n_loci_by_state_sortable() {
        let w = supply_world();

        // celegans 예시: membrane potential 상위 2개 뉴런
        let result = execute(
            &w,
            &Query::FindLoci {
                predicates: vec![],
                sort_by: Some(LocusSort::StateDesc(0)),
                limit: Some(2),
            },
        );
        match result {
            QueryResult::LocusSummaries(rows) => {
                assert_eq!(rows.len(), 2);
                // SUPPLIER_A(0.9) > WAREHOUSE(1.4) — 아, WAREHOUSE가 더 높다
                // WAREHOUSE(1.4) > SUPPLIER_A(0.9)
                assert_eq!(rows[0].id, WAREHOUSE); // 1.4
                assert_eq!(rows[1].id, SUPPLIER_A); // 0.9
            }
            _ => panic!("expected LocusSummaries"),
        }
    }

    // ══════════════════════════════════════════════════════════════════════════
    // ✅ P0 해결: 결과에 세부 정보 포함 — 2단계 룩업 불필요
    // ══════════════════════════════════════════════════════════════════════════

    #[test]
    fn find_rels_result_has_all_fields_no_second_lookup() {
        let w = supply_world();

        // celegans 라인 473-484: activity, from, to, kind 가 모두 필요
        let result = execute(
            &w,
            &Query::FindRelationships {
                predicates: vec![RelationshipPredicate::ActivityAbove(0.5)],
                sort_by: Some(RelSort::ActivityDesc),
                limit: None,
            },
        );
        match result {
            QueryResult::RelationshipSummaries(rows) => {
                for r in &rows {
                    // 모든 필드가 결과에 있다 — world.relationships().get() 불필요
                    let _ = (
                        r.from,
                        r.to,
                        r.kind,
                        r.activity,
                        r.weight,
                        r.change_count,
                        r.directed,
                    );
                    // supply_chain 예시의 루프와 동일한 패턴
                    let kind_str = if r.kind == KIND_SUPPLY {
                        "supply"
                    } else {
                        "order"
                    };
                    let _ = format!(
                        "L{}→L{}  kind={}  activity={:.3}  weight={:.4}  touches={}",
                        r.from.0, r.to.0, kind_str, r.activity, r.weight, r.change_count
                    );
                }
            }
            _ => panic!("expected RelationshipSummaries"),
        }
    }

    #[test]
    fn find_loci_result_has_state_no_second_lookup() {
        let w = supply_world();

        // supply_chain: warehouse stock을 알기 위한 패턴
        let result = execute(
            &w,
            &Query::FindLoci {
                predicates: vec![LocusPredicate::OfKind(LocusKindId(1))],
                sort_by: None,
                limit: None,
            },
        );
        match result {
            QueryResult::LocusSummaries(rows) => {
                let warehouse = rows.iter().find(|r| r.id == WAREHOUSE).unwrap();
                // state 슬롯 값이 결과에 있다 — world.locus(id) 불필요
                assert!((warehouse.state[0] - 1.4).abs() < 1e-5);
            }
            _ => panic!("expected LocusSummaries"),
        }
    }

    // ══════════════════════════════════════════════════════════════════════════
    // ✅ P1 해결: LocusStateSlot — supply_chain 라인 402/428/445 패턴
    // ══════════════════════════════════════════════════════════════════════════

    #[test]
    fn locus_state_slot_replaces_direct_world_access() {
        let w = supply_world();

        // 이전: world.locus(WAREHOUSE).map(|l| l.state[0]).unwrap_or(0.0)
        // 이후:
        let result = execute(
            &w,
            &Query::LocusStateSlot {
                locus: WAREHOUSE,
                slot: 0,
            },
        );
        match result {
            QueryResult::MaybeScore(Some(stock)) => {
                assert!((stock - 1.4).abs() < 1e-5);
            }
            _ => panic!("expected MaybeScore(Some)"),
        }
    }

    // ══════════════════════════════════════════════════════════════════════════
    // ✅ P1 해결: RelationshipProfile에 dominant_kind, activity_by_kind 추가
    //    supply_chain 라인 538-541 패턴
    // ══════════════════════════════════════════════════════════════════════════

    #[test]
    fn relationship_profile_has_dominant_kind_and_breakdown() {
        let w = supply_world();

        let result = execute(
            &w,
            &Query::RelationshipProfile {
                from: SUPPLIER_A,
                to: FACTORY,
            },
        );
        match result {
            QueryResult::RelationshipProfile(p) => {
                // supply_chain 라인 534: "dominant_kind=SUPPLY_KIND"
                assert_eq!(p.dominant_kind, Some(KIND_SUPPLY));

                // supply_chain 라인 539: "for (kind, act) in bundle.activity_by_kind()"
                assert!(!p.activity_by_kind.is_empty());
                // 첫 번째가 가장 높은 activity
                let supply_entry = p.activity_by_kind.iter().find(|(k, _)| *k == KIND_SUPPLY);
                assert!(supply_entry.is_some());

                // net_influence: SUPPLIER_A→FACTORY(0.80) - FACTORY→SUPPLIER_A(0.30) = 0.50
                assert!((p.net_influence - 0.50).abs() < 1e-4);
            }
            _ => panic!("expected RelationshipProfile"),
        }
    }

    // ══════════════════════════════════════════════════════════════════════════
    // ✅ P1 해결: ActivityTrend — supply_chain 라인 554-563 패턴
    //    (log 데이터 없으면 Insufficient 반환)
    // ══════════════════════════════════════════════════════════════════════════

    #[test]
    fn activity_trend_insufficient_without_log_data() {
        let w = supply_world();
        // log 데이터 없음 → Insufficient (데이터 없이 트렌드 계산 불가)
        let rel_id = w.relationships().iter().next().map(|r| r.id).unwrap();
        let result = execute(
            &w,
            &Query::ActivityTrend {
                relationship: rel_id,
                from_batch: BatchId(0),
                to_batch: BatchId(10),
            },
        );
        match result {
            QueryResult::Trend(crate::query_api::TrendResult::Insufficient) => {}
            _ => panic!("expected Trend(Insufficient) for world with no change log"),
        }
    }

    // ══════════════════════════════════════════════════════════════════════════
    // ✅ P2 해결: Coheres — celegans 라인 449-454 패턴
    //    (cohere 없는 세계 → 빈 Vec)
    // ══════════════════════════════════════════════════════════════════════════

    #[test]
    fn coheres_returns_empty_for_no_coheres() {
        let w = supply_world();
        let result = execute(&w, &Query::Coheres);
        match result {
            QueryResult::Coheres(cs) => {
                assert_eq!(cs.len(), 0); // extract_cohere 안 했으므로 없음
            }
            _ => panic!("expected Coheres"),
        }
    }

    #[test]
    fn coheres_named_returns_empty_for_missing_key() {
        let w = supply_world();
        let result = execute(&w, &Query::CoheresNamed("touch_circuit".to_string()));
        match result {
            QueryResult::Coheres(cs) => assert_eq!(cs.len(), 0),
            _ => panic!(),
        }
    }

    // ══════════════════════════════════════════════════════════════════════════
    // ✅ P2 해결: FindEntities 정렬 — celegans entity 섹션 패턴
    // ══════════════════════════════════════════════════════════════════════════

    #[test]
    fn find_entities_sort_coherence_desc() {
        let w = supply_world(); // 엔티티 없음 — 구조 테스트만
        let result = execute(
            &w,
            &Query::FindEntities {
                predicates: vec![EntityPredicate::CoherenceAbove(0.0)],
                sort_by: Some(EntitySort::CoherenceDesc),
                limit: Some(5),
            },
        );
        match result {
            QueryResult::Entities(ids) => {
                assert_eq!(ids.len(), 0); // 엔티티 없음
            }
            _ => panic!(),
        }
    }

    // ══════════════════════════════════════════════════════════════════════════
    // ✅ AllBetweenness + limit — "top-N hub" 패턴
    // ══════════════════════════════════════════════════════════════════════════

    #[test]
    fn all_betweenness_top_n_hub_pattern() {
        let w = supply_world();

        // ring_dynamics 예시: "가장 betweenness 높은 hub 2개"
        let result = execute(&w, &Query::AllBetweenness { limit: Some(2) });
        match result {
            QueryResult::LocusScores(scores) => {
                assert_eq!(scores.len(), 2);
                // 내림차순 보장
                assert!(scores[0].1 >= scores[1].1);
                // FACTORY가 SUPPLIER_A/B와 WAREHOUSE 사이 허브
                assert_eq!(scores[0].0, FACTORY);
            }
            _ => panic!("expected LocusScores"),
        }
    }

    // ══════════════════════════════════════════════════════════════════════════
    // 복합 predicate 여전히 잘 동작하는지 회귀 테스트
    // ══════════════════════════════════════════════════════════════════════════

    #[test]
    fn compound_predicates_still_work() {
        let w = supply_world();
        let result = execute(
            &w,
            &Query::FindRelationships {
                predicates: vec![
                    RelationshipPredicate::OfKind(KIND_SUPPLY),
                    RelationshipPredicate::ActivityAbove(0.5),
                ],
                sort_by: Some(RelSort::ActivityDesc),
                limit: None,
            },
        );
        match result {
            QueryResult::RelationshipSummaries(rows) => {
                assert_eq!(rows.len(), 2); // A→FACTORY(0.80), FACTORY→WAREHOUSE(0.70)
                assert!(rows[0].activity >= rows[1].activity);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn find_loci_by_property_still_works() {
        let mut w = supply_world();
        let mut props = graph_core::Properties::new();
        props.set("role", "supplier".to_string());
        w.properties_mut().insert(SUPPLIER_A, props.clone());
        w.properties_mut().insert(SUPPLIER_B, props);

        let result = execute(
            &w,
            &Query::FindLoci {
                predicates: vec![LocusPredicate::StrPropertyEq {
                    key: "role".to_string(),
                    value: "supplier".to_string(),
                }],
                sort_by: None,
                limit: None,
            },
        );
        match result {
            QueryResult::LocusSummaries(rows) => {
                assert_eq!(rows.len(), 2);
                let ids: Vec<_> = rows.iter().map(|r| r.id).collect();
                assert!(ids.contains(&SUPPLIER_A));
                assert!(ids.contains(&SUPPLIER_B));
            }
            _ => panic!(),
        }
    }

    // ══════════════════════════════════════════════════════════════════════════
    // EntityDeviationsSince — 엔티티 없는 세계에서 빈 Vec 반환
    // ══════════════════════════════════════════════════════════════════════════

    #[test]
    fn entity_deviations_empty_for_no_entities() {
        let w = supply_world();
        let result = execute(&w, &Query::EntityDeviationsSince(BatchId(0)));
        match result {
            QueryResult::EntityDeviations(diffs) => {
                assert_eq!(diffs.len(), 0);
            }
            _ => panic!("expected EntityDeviations"),
        }
    }

    // ══════════════════════════════════════════════════════════════════════════
    // RelationshipsAbsentWithout — root_changes 없으면 빈 Vec
    // ══════════════════════════════════════════════════════════════════════════

    #[test]
    fn relationships_absent_without_empty_roots() {
        let w = supply_world();
        let result = execute(&w, &Query::RelationshipsAbsentWithout(vec![]));
        match result {
            QueryResult::RelationshipSummaries(rows) => {
                // 빈 root set → 아무 관계도 "absent" 아님
                assert_eq!(rows.len(), 0);
            }
            _ => panic!("expected RelationshipSummaries"),
        }
    }
}
