# Advanced coalgebra primitives

> Status: design memo, 2026-04-28. Builds on `docs/coalgebra.md`.

`docs/coalgebra.md` mapped the existing 5-layer ontology onto the
basic coalgebraic vocabulary (Mealy coalgebra, bisimulation,
algebra/coalgebra duality at the schema/world boundary). That layer
was *coalgebra 1.0* — it gave names to patterns the codebase had
already arrived at intuitively.

This memo covers the deeper machinery actually used in current
research literature, with concrete new primitives shipped in the
substrate plus design rules for upcoming work. The order follows the
ROI ranking established in the design conversation:

1. Behavioral metrics (Kantorovich/Hausdorff lift) — **shipped**
2. Bialgebraic SOS / GSOS format — **policy only** (deferred runtime)
3. Predicate liftings & coinvariant classification — **shipped**
4. Final coalgebra projection as identity — **shipped**
5. Weighted coalgebras for evidence-style observations — **shipped**

Plus a deferred-but-watched list (§7) covering up-to congruence,
polynomial functors, fibered coalgebras, generic determinization, and
coinductive specification logics.

---

## 1. Behavioral metrics — `graph-core::metric`, `graph-query::metric`

### Theory

Replaces ad-hoc thresholds (`min_bridge_activity`,
`min_activity_threshold=0.1` in `DefaultEmergencePerspective`, the
Davis/Karate-tuned constants) with a real-valued bisimulation metric.

Following Desharnais–Edalat–Panangaden (TCS 2004) and Bonchi–König–
Petrişan (CONCUR 2018), the metric is the unique fixed point of

```text
d_{k+1}(x, y) = max( d_loc(x, y),
                     γ · matching(N(x), N(y), d_k, d_edge) )
```

where `d_loc`, `d_edge` are user-supplied pointwise metrics in
`[0, 1]`, `matching` is the Hausdorff lift over the per-locus
neighborhood multiset, and `γ ∈ (0, 1]` is the discount factor.

Properties:

- `d(x, x) = 0` (since `d_loc(x, x) = 0` by metric axioms).
- Symmetric.
- Triangle inequality (inherited from `d_loc`, `d_edge`).
- Bounded: `d(x, y) ∈ [0, 1]`.
- Monotone & convergent by Knaster–Tarski; pointwise `d_loc` is a
  hard lower bound.

### Code

```text
graph-core::metric             — LocusMetric / EdgeMetric traits
                                 + KindOnlyMetric
                                 + KindAndStateMetric (Euclidean, scaled)
                                 + KindOnlyEdgeMetric
                                 + KindAndStrengthEdgeMetric
                                 + hausdorff_distance helper

graph-query::metric            — behavioral_distance(world, a, b, opts)
                                 — bounded-depth, memoized recursion
                               — behavioral_distance_fixpoint(...)
                                 — full table contraction to fixpoint
```

`MetricOptions` carries `discount`, `max_rounds`, `epsilon`,
`locus_metric`, `edge_metric`. Defaults give a `KindAndStateMetric` /
`KindOnlyEdgeMetric` setup with `discount = 0.5`.

### When to use which

- **Cohere clustering** (`CoherePerspective::cluster`): when a
  perspective wants distance-based clustering rather than activity-
  bridge thresholding, compute `behavioral_distance_fixpoint` over
  candidate pairs and cluster by `d < ε`. The default activity-bridge
  perspective stays as-is for activity-driven semantics.

- **Entity merge candidates**: pairwise distance below a calibration
  level signals a merge candidate. Combine with the §4 final-coalgebra
  projection for the strict equivalence test.

- **Calibration-free thresholds**: any place currently using a hand-
  tuned threshold (`min_bridge_activity`, `EmergenceThreshold` floor)
  can be re-expressed as "distance below ε" once the metric is wired
  in. **This wiring is gated by the project's evidence-based removal
  policy** — see §6.

---

## 2. Bialgebraic SOS / GSOS format — *policy*, deferred runtime

### Theory

Turi & Plotkin's bialgebraic semantics (LICS 1997) gives an exact
characterization of when bisimulation is automatically a
**congruence** for the algebraic operations layered on top of a
coalgebra. The structural condition is the existence of a
*distributive law* `λ: ΣF → FΣ` between the operation signature `Σ`
and the observation functor `F`.

In practice, distributive laws for first-order signatures are
characterized by **GSOS format** (Klin, *Bialgebras for structural
operational semantics*, TCS 2011): operation rules whose premises
inspect at most one transition step from each operand. GSOS rules
guarantee:

> `x ~ x'` and `y ~ y'`  ⇒  `op(x, y) ~ op(x', y')`

for every operator `op` in the signature.

### What this means for the substrate

The Σ side of the substrate is the `StructuralProposal` enum (9
variants: `CreateRelationship`, `DeleteRelationship`, `Subscribe…`,
`CreateLocus`, `DeleteLocus`, …). The F side is `LocusProgram::process`.
A GSOS-style rule for, say, `CreateRelationship` is *"the new
relationship's initial activity depends only on the proposal's own
fields, not on deep observation of the endpoints' future behavior"*.

This is the invariant we want, because it is exactly what makes
refactors safe: if two loci are bisimilar at the depth the perspective
cares about, applying the same `StructuralProposal` to each must
preserve bisimilarity.

### Policy (binding for new variants)

When adding a new variant to `StructuralProposal` or `WorldEvent`, the
PR description must answer:

1. **What is the variant's GSOS rule?** Specifically: which fields of
   the proposal are *constructors* (from the variant's own data) and
   which are *observations* of operands (from `LocusContext`)?
2. **Is each observation 1-step?** I.e., does it read state at the
   start of the batch (1-step) rather than recursively unfolding into
   neighbor behavior?
3. **If not 1-step, why is congruence still preserved?** Answer must
   reference an external mechanism (e.g., the engine batch loop's
   stratification ensures 1-step-ness even when the rule looks deeper
   in the operand).

A variant that fails (1)–(3) is *not* GSOS-format and bisimulation is
not automatically a congruence for it. Such variants should be
flagged in `docs/redesign.md` §8 as exceptions, and their congruence
status must be proven case-by-case.

The existing 9 variants pass this check trivially: they all act on
constructor data only. The risk surface is for *future* additions
(particularly anything involving `EmergenceEvidence` aggregation).

### Why no runtime check

Mechanizing GSOS-format checking in Rust requires either a custom
proc-macro or a typed enum representation that is more invasive than
the benefit warrants. The policy above plus a CLAUDE.md invariant
entry catches new variants at PR time, which is when the cost of
fixing is lowest. This may be revisited if `StructuralProposal` grows
past ~15 variants.

---

## 3. Predicate liftings & coinvariant classification — `graph-core::coinvariant`

### Theory

A predicate `P ⊆ X` lifts along the F-functor to `F̄(P) ⊆ F(X)`
(Pattinson, TCS 2003; Hermida-Jacobs, IC 1998). A **coinvariant** is
a predicate `P` such that `α(x) ∈ F̄(P)` for every `x ∈ P` — one F-step
preserves it. This is the coalgebraic dual of "closed under
constructors" for an inductive type.

For a Mealy-style locus coalgebra, a one-step coinvariant is a
property the engine batch loop preserves automatically; checking it
once per crate is sufficient.

### Three classes of invariant

The current CLAUDE.md "Design invariants" section lists six rules in
prose. Coalgebraic analysis splits them into three classes:

| Class | Meaning | CLAUDE.md examples |
|-------|---------|--------------------|
| **OneStep** coinvariant | Preserved by a single F-step (one batch). Local check. | ChangeId density · Predecessor auto-derivation |
| **Trace** invariant | Required of every operation, not just batch transitions. Global check. | ChangeLog append-only · Subscription generation monotonicity |
| **Boundary** invariant | Checked at API edges only. | Schema versioning |

### Code

```text
graph-core::coinvariant::Coinvariant         — trait
graph-core::coinvariant::InvariantKind       — OneStep | Trace | Boundary
graph-core::coinvariant::ChangeIdDensity     — concrete OneStep
graph-core::coinvariant::PredecessorsAreAntecedent  — concrete OneStep
graph-core::coinvariant::ChangeLogAppendOnly — concrete Trace
graph-core::coinvariant::SchemaVersionMatches — concrete Boundary
graph-core::coinvariant::classification_summary(&[Kind])
```

Each concrete coinvariant has a stable `name()`, a `kind()`, and a
`check(witness) -> Result<(), String>`. Use in debug-build assertions
or in CI auditing scripts.

### Policy (recommended)

When adding a new design invariant, classify its `InvariantKind`
explicitly. If the answer is "I'm not sure", default to **Trace** —
it is the strictest class and demands the most rigor; downgrading
later is safe, upgrading is not.

---

## 4. Final coalgebra projection — `graph-query::coalgebra`

### Theory

The unique morphism `!: X → νF` from any coalgebra to the final
coalgebra projects each state to its **canonical observable identity**
— two states share the projection iff they are bisimilar at every
depth (Adámek-Milius-Moss, FoSSaCS 2019; Aczel, *Non-Well-Founded
Sets*, 1988).

For our finite-locus coalgebra, the projection is computed by running
the refinement procedure of `behavioral_partition` until fixpoint
(at most `|loci|` rounds, usually far fewer). The early-termination
condition we already had detects fixpoint and bails.

### Why this matters operationally

The HEP-PH Ω5 finding (memory: `project_hep_ph_finding.md`) discovered
that `recognize_entities` was non-idempotent. The fix wrapped it in a
fixpoint loop. **That fix was, in coalgebraic terms, the final-
coalgebra projection of the entity recognition coalgebra.** It is not
incidental — every "the same input must give the same output" bug in
a perspective-style coalgebra has the same shape and the same fix.

Looking forward, entity merge / split policies should use the final-
coalgebra projection (= deep behavioral signature) as the equivalence
test, not surface-level overlap heuristics. The Split-then-rematch
issue tracked in `project_lfr_finding.md` is exactly the failure mode
to watch for: surface-level identity diverged from final-coalgebra
identity.

### Code

```text
graph-query::coalgebra::behavior_fixpoint(world, locus, locus_enc, edge_enc)
    -> Option<BehaviorColor>

graph-query::coalgebra::behavioral_partition_fixpoint(world, locus_enc, edge_enc)
    -> Vec<Vec<LocusId>>
```

Both run with `rounds = |loci|` and rely on the existing fixpoint
detection in `compute_colors` to bail early.

---

## 5. Weighted coalgebras — `graph-core::evidence`

### Theory

A weighted coalgebra has the form `α: X → T(F(X))` for a strong monad
`T` capturing the kind of evidence accumulation desired
(Bonchi-Bonsangue-Rutten, IC 2009; Hasuo-Jacobs-Sokolova, LMCS 2007).
Common choices for `T`:

| Monad `T` | Semantics | Combine |
|-----------|-----------|---------|
| `T(X) = X × ℝ_{≥0}` | sum-of-evidence | `+` |
| `T(X) = X × ℝ` | max-of-evidence | `max` |
| `T(X) = D(X)` (distribution) | probabilistic | convex combination |
| `T(X) = (X × ℝ_{≤c})` | bounded-sum-of-evidence | clamp at `c` |

Bisimulation lifts to the weighted setting via the *Kantorovich
lifting* of the underlying functor `F` along `T`; the metric of §1 is
exactly this lifting for our deterministic case.

### Why this matters for the roadmap

Phase 1+ of `project_emergence_trigger_roadmap.md` elevates emerge
atom to a first-class `EmergenceEvidence` record. The shape of that
record is a weighted observation `(EmergenceAtom, Weight) ∈ T(Atom)`
for some `T` chosen by the domain. The ad-hoc choice "just stick a
`f64` weight in there" works but loses the algebraic structure that
makes aggregation correct under merging, marginalization, and
behavioral-distance lifting.

### Code

```text
graph-core::evidence::EvidenceMonoid     — trait (associative + identity)
graph-core::evidence::SumF64             — additive
graph-core::evidence::MaxF64             — peak
graph-core::evidence::MinF64             — dual
graph-core::evidence::BoundedSumF64      — clamped additive
graph-core::evidence::ProbProductMonoid  — independent-events product
graph-core::evidence::WeightedObservation<T, W> — Kleisli container
                                                 (.pure, .combine_with, .map_observation)
```

Tests verify the monoid laws (associativity, identity) for each
default. When Phase 1 lands, `EmergenceEvidence` becomes a one-liner:

```rust
type EmergenceEvidence = WeightedObservation<EmergenceAtom, BoundedSumF64>;
```

with the choice of monoid being the explicit domain decision.

---

## 6. Wiring policy and the evidence-based removal rule

The substrate adopted, after HEP-PH Finding 5, an evidence-based
removal policy: every demotion / removal of a feature must be
justified across three diversity axes (scale × temporality ×
curation). The same logic applies in reverse to **introduction** of
new abstractions.

The primitives shipped here are intentionally not wired into the
existing engine paths. Each wiring decision must satisfy:

1. **Trigger condition stated in prose:** "this primitive replaces
   threshold X in code path Y under conditions Z."
2. **Behavioral validation:** show on at least one realistic dataset
   (Davis / HEP-PH 122m / Enron) that the new primitive produces a
   demonstrably better outcome on a metric the policy already tracks
   (precision@K, false-merge rate, etc.) than the threshold it
   replaces.
3. **Reversibility:** the wiring lands behind a feature flag or
   pluggable trait so that benchmark regression backs it out without
   a full revert.

Until those conditions are met for a specific wiring, the primitive
remains a library — usable by callers who opt in, not a default.

This memo does *not* propose any specific wiring. The next concrete
candidate (per the roadmap) is the Phase 1 `EmergenceEvidence` →
`graph-core::evidence` adoption, since that is greenfield and the
policy reduces to "use this rather than a raw `f64`".

---

## 7. Deferred but watched

Areas surveyed in the design conversation but not implemented this
round, with the trigger that would justify revisiting:

| Area | Trigger to revisit | Reference |
|------|-------------------|-----------|
| **Up-to congruence** (Pous-Sangiorgi) | When `behavioral_partition` becomes a hot path on >100k-locus worlds | Pous, *Coinduction All the Way Up*, LICS 2016 |
| **Polynomial functors / containers** | When `StructuralProposal` exceeds ~15 variants and generic traversal is needed | Abbott-Altenkirch-Ghani, TCS 2005 |
| **Fibered coalgebras** | When `InfluenceKindId` sub-kinding is needed (currently deferred per `relationship.rs`) | Hermida-Jacobs, IC 1998 |
| **Generic determinization** | When LLM ingestion produces non-deterministic parses and we want bisimulation-preserving normal forms | Silva-Bonchi-Bonsangue, TCS 2013 |
| **Coinductive specification logics** | When Hoare-style reasoning about engine ticks is needed | Reichel; Hasuo-Cho |

All of these are documented here so future PR reviews can recognize
when one of the triggers is crossed, rather than rediscovering them
ad hoc.

---

## 8. Cross-reference

- `docs/coalgebra.md` — the foundational framing this memo extends.
- Source: `crates/graph-core/src/{metric,evidence,coinvariant,coalgebra}.rs`,
  `crates/graph-query/src/{metric,coalgebra}.rs`.
- Tests: each module's `tests` submodule; all green at time of writing.
- Related findings: `docs/hep-ph-finding.md` (Ω5), `docs/lfr-finding.md`
  (Split rematch issue mentioned in §4).
- Related design notes: `docs/redesign.md` (5-layer ontology),
  `docs/identity.md`, `docs/complexity-audit.md`.
