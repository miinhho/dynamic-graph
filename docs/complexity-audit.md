# Complexity Audit

Living document tracking the cognitive/tuning surface of the engine and the
findings that motivate simplification decisions.

This is not a design doc. It is evidence: what users must decide, and what
we've empirically learned about which decisions matter.

---

## Finding 1 — DefaultEmergencePerspective does not generalise across graph densities

**Date**: 2026-04-18
**Evidence**: `crates/graph-engine/tests/davis_women.rs`

The default `min_activity_threshold = 0.1` separates communities correctly
on karate_club (34 nodes, 78 edges, ~14% density) but collapses every node
into one community on Davis Southern Women (18 women, 139 co-attendance
pairs active, ~91% density).

Tuning the threshold to 3.0 (static) or 5.0 (dynamic) recovers the
Freeman-2003 consensus partition on Davis. But a fixed default cannot
satisfy both datasets — the appropriate cutoff depends on the activity
distribution the kind produces.

**Implication**: the "default" is a karate-tuned constant, not a universal
prior. Users of dense graphs must tune it manually.

**Candidate remediations**:
- Distribution-aware threshold: compute a knee point on the sorted
  activity histogram each time `recognize_entities` runs
- Bipartite-aware perspective: detect event-like loci and cluster women by
  shared-event signatures instead of co-attendance edges
- Document the tuning requirement explicitly in the perspective's docstring

Do not adopt a fix yet; LFR and SocioPatterns datasets are scheduled next
and will produce more evidence about which axis (threshold, algorithm,
weighting) is the right place to intervene.

---

## Finding 2 — LayerTransition usage is skewed; overlap_threshold 0.5 rejects legitimate splits

**Date**: 2026-04-18
**Evidence**: `crates/graph-engine/tests/lfr_dynamic.rs`

Greene-style dynamic benchmark (60 nodes, 4 planted communities,
Born/Split/Merge/Dormant schedule). 4 isolated scenarios + 1 composite
protocol run scored at proposal level (absorbed-side `Merged` layers
and offspring `Born` layers are filtered in `collect_transitions` so the
numbers reflect what the engine *decided*, not how many records it wrote).

### 2a — Which transitions actually fire

Raw layer counts on the full benchmark (all 6 test functions):

| Transition       | Fires under planted schedule?            | Notes |
|------------------|------------------------------------------|-------|
| `Born`           | Yes — 1 per newly recognized community   | Load-bearing |
| `Split`          | Yes — 1 per split, gated by overlap knob | Load-bearing (see 2b) |
| `Merged`         | Yes — 1 per merge proposal               | Load-bearing (see 2c) |
| `BecameDormant`  | Yes — 1 per community that goes silent   | Load-bearing |
| `MembershipDelta`| 1 deposit total across all 6 tests       | Essentially dead weight |
| `CoherenceShift` | **Never fires**                          | No test trips the 0.05 drift gate |
| `Revived`        | **Never fires**                          | Benchmark schedule has no dormant-then-revived path |

**Implication**: of the 7 variants, 4 are load-bearing. `CoherenceShift`
and `Revived` did not fire once. `MembershipDelta` fired once (in the
default-threshold composite only, when overlap-gating kept an entity
alive with a mutated member set). Before demoting them to an `advanced`
module, the SocioPatterns/Enron datasets — which have longer, noisier
regimes — should be checked for the drift-heavy cases these three
variants are designed for.

### 2b — `overlap_threshold = 0.5` is a karate-tuned default too

At `overlap_threshold = 0.5` (default), a 15-member entity A splitting
into offspring of size 8 and 7 cannot produce a `Split` transition:
Jaccard(A2=7, A=15) = 7/15 ≈ 0.467 < 0.5. A becomes Dormant and A2 is
reborn as a fresh entity, losing the causal link.

`composite_greene_protocol_default_threshold_recovers_most` confirms:
Born/Merge/Dormant recover at default threshold, but Split recall = 0.
Tuning to `overlap_threshold = 0.4` recovers Split recall = 1.0.

**Implication**: the 0.5 cutoff is a second karate-tuned constant (like
Finding 1's 0.1). It silently kills Split lineage on any asymmetric
partition. Candidate remediation: distribution-aware threshold, OR
accept a Split if the offspring partition *covers* the source above an
area-based bar (rather than per-offspring Jaccard).

### 2c — Post-Split source re-matched children (engine invariant) — **Resolved 2026-04-18**

**Before fix**: after `EmergenceProposal::Split` fired, the source
entity A was left with `status = Active` and `current.members =
original members`. On the next `recognize_entities` pass, components
containing A1/A2 members still Jaccard-matched A above the tuned
`overlap_threshold = 0.4`, producing spurious `Merge` proposals folding
A with its own children. Proposal-level Merge precision in the
composite tuned run was 0.33 (3 detected vs 1 planted: 1 legitimate CD
merge, 2 A re-absorbing its own split offspring).

**Fix** (`crates/graph-engine/src/engine/world_ops.rs` Split branch):
after depositing the `Split` layer on the source, flip its status to
`Dormant`. The `Split` layer itself records the lifecycle transition;
no separate `BecameDormant` layer is added. The source's identity
continues through `lineage.children`, which was already being
populated.

**Post-fix numbers** (`composite_greene_protocol_tuned`): all four
planted transitions recover at precision 1.0 / recall 1.0.
`MembershipDelta = 0`, `CoherenceShift = 0`, `Revived = 0`.

The fix does not affect karate/davis/partition_determinism suites —
none of them relied on the pre-fix post-Split Active behaviour.

---

## Aggressive reduction campaign (2026-04-18)

Active sweep across the 47-knob surface, in phases. Each phase removes knobs
backed by benchmark evidence of irrelevance or algorithmic redundancy.

### Phase 1 — `overlap_threshold` removed

Replaced Jaccard-based component/entity reconciliation with **locus-flow
analysis**: the engine partitions each active entity's members by which
component they landed in, deriving Split/Dormant/Continuation/Merge from
the bucket distribution without any overlap threshold. Subset-attack
guard: `unassigned > bucket → Dormant`. Only internal constant is
`MIN_SIGNIFICANT_BUCKET = 2` (noise tolerance for 1-locus drift).

Killed Finding 2b entirely. Killed Finding 2c (post-Split dormancy fix
merged in the same pass).

### Phase 2 — `min_activity_threshold` / `min_bridge_activity` → `Option<f32>`

`None` is the new default. Thresholds auto-compute from the activity
distribution:
- Emergence: largest relative gap in the lower half of sorted activities,
  applied only if the gap exceeds 2× (clear bimodal signal-vs-noise split).
  Otherwise no filter — label-propagation's weighted voting handles it.
- Cohere: median of nonzero bridge activities.

Killed Finding 1 (Davis collapse): the gap-detector finds Davis's
intra/cross co-attendance gap automatically.

### Phase 3 — Plasticity 6 → 3

Removed STDP (`stdp` bool, `ltd_rate`) and BCM (`bcm_tau`, `World::bcm_thresholds`
storage). Plain Hebbian (`learning_rate`, `weight_decay`, `max_weight`) is
the only rule. No benchmark demonstrated BCM/STDP improving outcomes.
Deleted 4 unit tests (stdp_anticausal_{weakens,clamps}, bcm_{ltp,ltd}) and
2 karate_club BCM tests.

### Phase 4 — Stabilization 3 → 1

Removed `saturation` (None/Tanh/Clip — no benchmark required non-None) and
`trust_region` (no benchmark used it). `alpha` is the only remaining
blend parameter. `SaturationMode` enum kept as a stub (only `None` variant)
for call-site stability.

### Running total — sweep completed 2026-04-18

| Phase | Knobs removed | Running count | Findings resolved |
|-------|---------------|---------------|-------------------|
| Start | — | 47 | — |
| 1 | `overlap_threshold` | 46 | 2b, 2c |
| 2 | (2 knobs become auto-default) | 46 (effective 44) | 1 |
| 3 | `stdp`, `ltd_rate`, `bcm_tau` | 43 | — |
| 4 | `saturation`, `trust_region` | 41 | — |
| 5 | `min_emerge_activity`, `max_activity`, `prune_activity_threshold`, `prune_weight_threshold` | 37 | — |
| 6 | `recent_window`, `compression_age`, `removal_age`, `preserved_transitions` | 33 | — |
| 7 | `quiescent_threshold`, `diverge_threshold`, `limit_cycle_tolerance`, `min_scale`, `max_scale`, `shrink_factor`, `recovery_factor` | 26 | — |
| 8 | `history_window`, `change_retention_batches`, `cold_relationship_threshold`, `cold_relationship_min_idle_batches`, `auto_weather_every_ticks`, `event_history_len`, `pending_stimuli_capacity`, `backpressure_policy` | **18** | — |

**Final surface: ~16 load-bearing knobs** (810 workspace tests passing).

### Phase 5 — Per-kind dynamics (4 removed)

Removed `min_emerge_activity`, `max_activity`, `prune_activity_threshold`,
`prune_weight_threshold`. The first two had benchmark evidence only in
`celegans.rs` (an example, not a correctness test); the latter two had
zero non-default usage. Auto-pruning is gone — callers who want
cleanup issue explicit `StructuralProposal::DeleteRelationship`.

### Phase 6 — Entity weathering (4 hard-coded)

`DefaultEntityWeathering` became a ZST. `recent_window`/`compression_age`/
`removal_age` are internal `const` values (50 / 200 / 1000). Custom
policies remain possible via `impl EntityWeatheringPolicy`.

### Phase 7 — Regime & adaptive (7 hard-coded)

`DefaultRegimeClassifier` and `AdaptiveConfig` became ZSTs. The seven
thresholds/factors are internal `const`. Call sites passing
`AdaptiveConfig::default()` continue to compile because the struct is
retained as a unit marker.

### Phase 9 scouting — plasticity auto-tuning deferred (2026-04-18)

Scoped as: `PlasticityConfig.learning_rate` and `.weight_decay` → `Option<f32>`
with `None` = observation-based auto. Rolled back after design surface
revealed a missing prerequisite.

**Why deferred**: auto-tuning needs a target for the self-supervised
feedback loop — traditional ML lr auto assumes a loss function. Our
engine has none. Heuristic surrogates ("keep weight variance in a band")
require either:
- a hard-coded target band (violates Principle 1 — override) or
- a new "objective" knob declared by the user (contradicts the
  reduction goal — knob moves from lr to objective).

Phase 2 threshold auto worked because "cluster formation" provides a
local objective (bimodal gap detection) — a property no `plasticity.*`
knob has.

**Reopen condition**: (a) SocioPatterns or Enron benchmark introduces a
supervised metric (e.g. "next-week interaction prediction accuracy"), or
(b) a `PlasticityObjective::*` API is added and accepted as a domain
declaration, not a tuning knob.

Rollback was clean — 810 tests still pass. `PlasticityConfig` unchanged
from Phase 3 shape (3 fields: learning_rate, weight_decay, max_weight).

### Phase 8 — Simulation lifecycle (8 hard-coded)

Removed `history_window`, `change_retention_batches`,
`cold_relationship_threshold`, `cold_relationship_min_idle_batches`,
`auto_weather_every_ticks`, `event_history_len`,
`pending_stimuli_capacity`, `backpressure_policy`. None had
non-default usage in benchmarks or tests. `SimulationBuilder::{history_window,
backpressure, auto_weather, auto_weather_with}` methods kept as no-ops
for call-site stability — they do nothing now.

### Remaining surface (16 knobs)

- **Per-InfluenceKind (10)**: `name`, `decay_per_batch`, `activity_contribution`,
  `parent`, `symmetric`, `applies_between`, `extra_slots`, `demotion_policy`,
  `stabilization.alpha`, `plasticity` (struct with 3 fields)
- **Per-LocusKind (2)**: `refractory_batches`, `max_proposals_per_dispatch`
- **Perspective auto-knobs (2)**: `min_activity_threshold`, `min_bridge_activity`
  — `Option<f32>`, default `None` = distribution-based auto
- **Engine (1)**: `engine.max_batches_per_tick`
- **Simulation (1)**: `auto_commit` (storage feature)

### Post-sweep additions (2026-04-18 afternoon)

**Phase 8 후속 3-agent 병렬 작업** 완료:

1. **SocioPatterns benchmark added** — `crates/graph-engine/tests/sociopatterns.rs`
   (4 tests, 40 students × 5 classes × 60 blocks). Full write-up:
   `docs/sociopatterns-finding.md`. Workspace tally: **814 tests, 0 failures**.
2. **Gap-detector window widened 50% → 75%** in `emergence/default.rs::auto_activity_threshold`.
   SocioPatterns showed the old window missed the bimodal cut in
   noise-heavy streams (noise floor at activity≈2.0, true cut at p75≈7.64).
   Karate/Davis/LFR regressions: 0.
3. **`Learnable` trait framework** introduced in `regime/adaptive.rs`.
   `AdaptiveGuardRail` is now a newtype over `PerKindLearnable<RegimeAlphaScale>`.
   Public API unchanged; the 7 existing adaptive tests still pass.
   Next auto-tuning work plugs in a new `impl Learnable` without touching
   atomic-state or registration scaffolding.

### Phase 9 reopen condition (a) — **MET**

SocioPatterns test `next_block_prediction_accuracy` supplies the
supervised metric Phase 9 scouting was missing:

- `precision@20 = 1.000` (hits 20/20, 2.11× lift vs base rate 0.473)
- `precision@50 = 1.000`
- `precision@100 = 0.970`
- `recall = 0.894` (330 / 369 test-block pairs)

Suggested plasticity objective (from `sociopatterns-finding.md`):
minimise `(1 − precision@K) + λ · (1 − recall)` on a rolling held-out
tail. `K` and `λ` are **user-declared domain knobs** (how many
predictions you need, how much you weight coverage vs. precision) —
domain declarations, not tuning knobs.

Phase 9 work is now an independent design decision (the
`PlasticityObjective::*` API shape), not a missing-data blocker.

### Additional reduction candidates (from Agent B docstring sweep)

While writing the uniform "**Override when**" block on the 16 survivors,
three knobs turned out to have *no concrete override reason* — they're
next-sweep candidates:

1. **`demotion_policy`** — three variants (ActivityFloor / IdleBatches /
   LruCapacity) but no benchmark distinguishes them. Collapse to one.
2. **`PlasticityConfig.weight_decay`** — user-facing, but no evidence
   non-default values help. Tentative internal const after Phase 9
   objectives-based auto-tune probes it.
3. **`min_activity_threshold` / `min_bridge_activity`** — the auto
   path is strong enough across karate/Davis/LFR/SocioPatterns that
   there's no concrete scenario where overriding is the right answer.
   Candidate for demotion to internal const once the Enron benchmark
   confirms or refutes. **Exception**: the SocioPatterns 50% → 75%
   finding shows the auto heuristic itself still needed tuning — don't
   demote before the heuristic is locked in.

Projected reduction if all three are removed: **16 → ~13**. Defer until
Enron benchmark data is in.

### Tests removed (documented for future archaeology)

- `composite_greene_protocol_default_threshold_recovers_most` (Finding 2b obsoleted)
- `dynamic_clique_emergence_default_threshold_merges` (Finding 1 obsoleted)
- `low_activity_edge_excluded_from_clustering` (hard-coded 0.1 assumption)
- `weak_bridge_below_threshold_suppressed` (hard-coded 0.3 assumption)
- `bcm_enhanced_faction_detection`, `bcm_cross_faction_suppression` (BCM gone)
- `bcm_ltp_when_post_above_threshold`, `bcm_ltd_when_post_below_threshold`
- `stdp_anticausal_weakens`, `stdp_anticausal_clamps_at_zero`
- `negative_prune_threshold_panics` (prune knobs gone)
- `backpressure_reject_drops_excess_stimuli`, `backpressure_drop_newest_same_as_reject`,
  `backpressure_drop_oldest_rotates_queue` (queue now permanently unbounded)

---

## Tuning Surface Inventory (2026-04-18)

47 user-tunable knobs surface in the current API. Grouped by locality:

### Per-InfluenceKind (21 knobs, multiplied by number of kinds)

**Dynamics (8)**
- `decay_per_batch` — activity decay per batch
- `activity_contribution` — per-touch signed contribution
- `min_emerge_activity` — creation gate
- `max_activity` — clamp cap
- `prune_activity_threshold` — auto-prune after decay
- `prune_weight_threshold` — auto-prune after Hebbian
- `symmetric` — structural (bool)
- `applies_between` — endpoint kind whitelist

**Plasticity (6)** — *three mutually non-exclusive rules*
- `plasticity.learning_rate` (LTP rate; 0 = off)
- `plasticity.ltd_rate` (asymmetric LTD rate; 0 = use learning_rate)
- `plasticity.weight_decay` (per-batch multiplier)
- `plasticity.max_weight` (clamp)
- `plasticity.stdp` (bool — timing-dependent mode)
- `plasticity.bcm_tau` (Option — BCM threshold adaptation)

**Stabilization guard-rail (3)**
- `stabilization.alpha` (state blend)
- `stabilization.saturation` (None/Tanh/Clip)
- `stabilization.trust_region` (L2 radius)

**Hierarchy & topology (4)**
- `parent` (kind inheritance)
- `extra_slots` (list of custom slots, each with default + decay)
- `demotion_policy` (ActivityFloor / IdleBatches / LruCapacity)
- `interaction_effects` (between kind pairs: Synergistic / Antagonistic / Neutral)

### Per-LocusKind (2 knobs)

- `refractory_batches`
- `max_proposals_per_dispatch`

### Emergence perspective (2 knobs)

- `min_activity_threshold` ← **Finding 1**
- `overlap_threshold` (Jaccard cutoff for entity reconciliation)

### Cohere perspective (1 knob)

- `min_bridge_activity`

### Entity weathering (4 knobs)

- `recent_window` (Preserved age)
- `compression_age` (Compress age)
- `removal_age` (Remove age)
- `preserved_transitions` (which LayerTransition kinds resist removal)

### Regime classifier (3 knobs)

- `quiescent_threshold`
- `diverge_threshold`
- `limit_cycle_tolerance`

### Adaptive guard rail (4 knobs)

- `min_scale`
- `max_scale`
- `shrink_factor`
- `recovery_factor`

### Simulation lifecycle (10 knobs)

- `engine.max_batches_per_tick`
- `history_window`
- `change_retention_batches`
- `cold_relationship_threshold`
- `cold_relationship_min_idle_batches`
- `auto_weather_every_ticks`
- `auto_commit`
- `event_history_len`
- `pending_stimuli_capacity`
- `backpressure_policy`

---

## Redundancy suspects

Axes where the current surface has 2+ alternatives without empirical
evidence of when each is needed:

| Category | Alternatives | Evidence basis |
|---|---|---|
| Plasticity rule | plain Hebbian / STDP / BCM | None of karate/davis/celegans require more than plain Hebbian |
| Subscription scope | Specific / AllOfKind / TouchingLocus | karate uses no subscriptions; davis uses none; celegans unknown |
| Demotion policy | ActivityFloor / IdleBatches / LruCapacity | No test differentiates them |
| Saturation mode | None / Tanh / Clip | No test requires non-None |
| Weathering effects | Preserved / Compress / Skeleton / Remove | No test has deep-enough sediment to exercise transitions |
| Stabilization blend | alpha + saturation + trust_region | Three distinct knobs for "don't let it diverge"; could collapse |
| `LayerTransition` variants | Born / Split / Merged / BecameDormant / MembershipDelta / CoherenceShift / Revived | LFR dynamic (Finding 2a): only Born/Split/Merged/BecameDormant are load-bearing; MembershipDelta fires ≤1×, CoherenceShift/Revived never fire |

---

## Simplification hypothesis

After N dataset benchmarks produce usage evidence:

1. **Keep** knobs that at least one benchmark demonstrably needs
2. **Hide** knobs not used by any benchmark behind an `advanced` module
3. **Merge** knobs whose effects are observationally indistinguishable
4. **Replace** karate-tuned constants with distribution-aware heuristics

This list is static until LFR, SocioPatterns, and Enron datasets produce
results. Revisit after each.

---

## Dataset queue

| Dataset | Purpose | Status |
|---|---|---|
| Karate Club | Baseline community detection | Done — in tree |
| Davis Southern Women | Event-stream emergence, bipartite noise | Done — 2026-04-18 |
| LFR dynamic benchmark | Sediment transition precision/recall | Done — 2026-04-18 |
| SocioPatterns primary school | Scale + temporal resolution | Done — 2026-04-18 (see `sociopatterns-finding.md`) |
| Enron email | Real-world scale, natural Merge/Dormant | Planned |
