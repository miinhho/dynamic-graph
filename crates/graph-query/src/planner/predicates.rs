use graph_core::{LocusId, RelationshipKindId};

use crate::query_api::{LocusPredicate, RelationshipPredicate};

pub(crate) struct RelPlan<'a> {
    pub seed_locus: Option<SeedKind>,
    pub predicates_ordered: Vec<&'a RelationshipPredicate>,
}

pub(crate) enum SeedKind {
    From(LocusId),
    To(LocusId),
    Touching(LocusId),
    DirectLookup {
        from: LocusId,
        to: LocusId,
        kind: RelationshipKindId,
    },
    Between {
        a: LocusId,
        b: LocusId,
    },
}

pub(crate) fn plan_rel_predicates(predicates: &[RelationshipPredicate]) -> RelPlan<'_> {
    let seed_context = RelationshipSeedContext::from_predicates(predicates);
    let (seed_locus, consumed) = seed_context.select_seed();
    let predicates_ordered = ordered_relationship_predicates(predicates, &consumed);
    RelPlan {
        seed_locus,
        predicates_ordered,
    }
}

pub(crate) fn plan_loci_predicates(predicates: &[LocusPredicate]) -> Vec<&LocusPredicate> {
    let mut ranked: Vec<(&LocusPredicate, u8)> = predicates
        .iter()
        .map(|p| (p, locus_pred_priority(p)))
        .collect();
    ranked.sort_unstable_by_key(|(_, p)| *p);
    ranked.into_iter().map(|(p, _)| p).collect()
}

struct RelationshipSeedContext {
    first_from: Option<(usize, LocusId)>,
    first_to: Option<(usize, LocusId)>,
    first_touching: Option<(usize, LocusId)>,
    first_of_kind: Option<(usize, RelationshipKindId)>,
}

impl RelationshipSeedContext {
    fn from_predicates(predicates: &[RelationshipPredicate]) -> Self {
        let mut context = Self {
            first_from: None,
            first_to: None,
            first_touching: None,
            first_of_kind: None,
        };
        for (index, predicate) in predicates.iter().enumerate() {
            context.observe(index, predicate);
        }
        context
    }

    fn observe(&mut self, index: usize, predicate: &RelationshipPredicate) {
        match predicate {
            RelationshipPredicate::From(id) if self.first_from.is_none() => {
                self.first_from = Some((index, *id));
            }
            RelationshipPredicate::To(id) if self.first_to.is_none() => {
                self.first_to = Some((index, *id));
            }
            RelationshipPredicate::Touching(id) if self.first_touching.is_none() => {
                self.first_touching = Some((index, *id));
            }
            RelationshipPredicate::OfKind(kind) if self.first_of_kind.is_none() => {
                self.first_of_kind = Some((index, *kind));
            }
            _ => {}
        }
    }

    fn select_seed(&self) -> (Option<SeedKind>, Vec<usize>) {
        if let Some((seed, consumed)) = self.direct_lookup_seed() {
            return (Some(seed), consumed);
        }
        if let Some((seed, consumed)) = self.between_seed() {
            return (Some(seed), consumed);
        }
        if let Some((seed, consumed)) = self.adjacency_seed() {
            return (Some(seed), consumed);
        }
        (None, Vec::new())
    }

    fn direct_lookup_seed(&self) -> Option<(SeedKind, Vec<usize>)> {
        match (self.first_from, self.first_to, self.first_of_kind) {
            (Some((fi, from)), Some((ti, to)), Some((ki, kind))) => {
                Some((SeedKind::DirectLookup { from, to, kind }, vec![fi, ti, ki]))
            }
            _ => None,
        }
    }

    fn between_seed(&self) -> Option<(SeedKind, Vec<usize>)> {
        match (self.first_from, self.first_to) {
            (Some((fi, a)), Some((ti, b))) => Some((SeedKind::Between { a, b }, vec![fi, ti])),
            _ => None,
        }
    }

    fn adjacency_seed(&self) -> Option<(SeedKind, Vec<usize>)> {
        self.first_from
            .map(|(index, id)| (SeedKind::From(id), vec![index]))
            .or_else(|| {
                self.first_to
                    .map(|(index, id)| (SeedKind::To(id), vec![index]))
            })
            .or_else(|| {
                self.first_touching
                    .map(|(index, id)| (SeedKind::Touching(id), vec![index]))
            })
    }
}

fn ordered_relationship_predicates<'a>(
    predicates: &'a [RelationshipPredicate],
    consumed: &[usize],
) -> Vec<&'a RelationshipPredicate> {
    let mut rest: Vec<(&RelationshipPredicate, u8)> = predicates
        .iter()
        .enumerate()
        .filter(|(index, _)| !consumed.contains(index))
        .map(|(_, predicate)| (predicate, rel_pred_priority(predicate)))
        .collect();
    rest.sort_unstable_by_key(|(_, priority)| *priority);
    rest.into_iter().map(|(predicate, _)| predicate).collect()
}

fn rel_pred_priority(pred: &RelationshipPredicate) -> u8 {
    match pred {
        RelationshipPredicate::From(_)
        | RelationshipPredicate::To(_)
        | RelationshipPredicate::Touching(_) => 5,
        RelationshipPredicate::OfKind(_) => 10,
        RelationshipPredicate::ActivityAbove(_)
        | RelationshipPredicate::StrengthAbove(_)
        | RelationshipPredicate::SlotAbove { .. }
        | RelationshipPredicate::MinChangeCount(_) => 20,
        RelationshipPredicate::CreatedInRange { .. } | RelationshipPredicate::OlderThan { .. } => {
            30
        }
    }
}

fn locus_pred_priority(pred: &LocusPredicate) -> u8 {
    match pred {
        LocusPredicate::OfKind(_) => 10,
        LocusPredicate::StateAbove { .. } | LocusPredicate::StateBelow { .. } => 20,
        LocusPredicate::F64PropertyAbove { .. } | LocusPredicate::StrPropertyEq { .. } => 30,
        LocusPredicate::MinDegree(_) => 40,
        LocusPredicate::ReachableFromActive { .. }
        | LocusPredicate::DownstreamOfActive { .. }
        | LocusPredicate::UpstreamOfActive { .. } => 85,
        LocusPredicate::ReachableFrom { .. }
        | LocusPredicate::DownstreamOf { .. }
        | LocusPredicate::UpstreamOf { .. } => 90,
    }
}
