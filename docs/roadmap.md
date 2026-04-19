# Roadmap

Last updated: 2026-04-19.

This document plans forward-looking work. The substrate (`docs/redesign.md`)
is feature-complete; the current chapter is **making the surface honest**:
shrinking the tuning vocabulary to what benchmarks actually require, proving
emergence claims are falsifiable, and turning declared-vs-observed drift
into a visible workflow.

---

## 1. Where we are

### 1.1 Substrate (frozen)

Nine crates across five observational layers (Locus → Change →
Relationship → Entity → Cohere) plus the declarative side
(`graph-schema`, `graph-boundary`) and LLM assist (`graph-llm`). Original
Phase 0–3 plan shipped. `identity.md §8` closed.

| Completed | Outcome |
|---|---|
| Phase 0 landing | `controller.rs`, `handle.rs`, `causal_strength.rs`, `configure.rs` promoted |
| Phase 1 measurement | `docs/perf/phase1-report.md` hotspots ranked |
| Phase 2 Cohere perf | O(E²×R) → O(E×M+R), 17.8× at N=1000 |
| Phase 3 E2/E3 | `ChangeLog::trim_before_batch`, `CoarseTrail`, per-kind demotion |
| Phase 3 E4 (partition parallel) | **Rejected — negative result**. Binding. |
| A1/A2/A3 | `async` feature, `subscribe_world_events`, backpressure queue |
| B3/B4 | Time-travel queries, graph-query criterion baselines |
| D1/D3/D4 | Causal-strength, counterfactual replay, entity-level causality |
| Track H (emergence) | Dense/synergy/decay/pair-grain Ψ + leave-one-out all shipped |
| Phase 9 P1 | `PairPredictionObjective` + `PlasticityLearners` in `graph-engine::plasticity` |

### 1.2 Active operating principles

These bind every track below.

1. **Complexity reduction is the default direction.** The knob surface went
   from 47 → ~16 between 2026-04-17 and 2026-04-18 (see
   `docs/complexity-audit.md`). New features add a knob only with dataset
   evidence; existing knobs are removed on any dataset producing evidence
   of irrelevance. Target through Enron: 16 → ~13.
2. **Performance work is measurement-gated.** No speculative perf PRs. A
   flamegraph or committed bench pointing at the site comes first. The E4
   negative result (`docs/e4-design.md §12`) is the binding precedent.
3. **Benchmarks drive knob decisions.** The dataset queue — Karate → Davis
   → LFR → SocioPatterns → Enron — is the ground truth for which
   abstractions are load-bearing. Each produces a Finding in
   `docs/complexity-audit.md`.
4. **Declarative anchor + observed drift is the differentiator.** The
   `graph-boundary` Confirmed/Ghost/Shadow/Null quadrants are what lets
   this engine say something the user did not already know. This remains
   the long-term product wedge.

### 1.3 Recent refactor wave (2026-04-18 → 19)

`a55abcc` → `0fa4d79` → `ac90078` → `61ce5f1`: engine batch loop staged
into `compute → build → apply → settle`; query pipeline split into
seed-selection vs. sorting stages. `docs/performance-priorities.md`
lists the five hot-path priorities this reorg exposed.

---

## 2. Active tracks

Priority ordering: tracks producing new **evidence** (benchmarks, knob
reductions, closure proofs) lead; tracks that **stabilize** surface
follow; perf/determinism tracks are on-demand.

### Track Ω — Knob reduction + dataset evidence *(priority 1)*

Folds old Track H closure-remainder + new evidence loop into one program.

- **Ω1. Enron benchmark.** ✓ **Complete (2026-04-19)**. 120-node,
  5-phase synthetic workload. `precision@20/50/100 = 1.000` (5.29×
  lift). Born 6/6, Merge 1/1, Revived 1/1. First exercise of the
  Revived transition. Finding in `docs/enron-finding.md`.
  Auto-threshold confirmed across all 5 datasets (stable-community
  class). See Ω4 for the dynamic-temporal class.
- **Ω5. HEP-PH citation network benchmark + `recognize_entities` fixpoint fix.**
  ✓ **Complete (2026-04-19)**. SNAP ArXiv HEP-PH: 34,546 papers,
  421,578 citations, 122 monthly batches.
  - Initial runs (pre-fix): 60m stress region showed active/node ratio
    3.63× and 37,815 active entities at month 48 — ABORT. Two hypotheses
    (hub-exclusivity on Born path, then DepositLayer path) tested; small
    positive but not the fix.
  - **Diagnostic breakthrough**: component-count probe (`Active ==
    Components` throughout) + idempotency probe (Δ=−154 at month 36)
    isolated `recognize_entities` as non-idempotent. Advisor-identified
    branch: "recognize itself is non-idempotent."
  - **Fix shipped**: `recognize_entities` wrapped in fixpoint loop
    (max 8 passes, proposals.is_empty() termination) in
    `crates/graph-engine/src/engine/world_ops.rs`.
  - **Post-fix results on 122m full corpus**: 716 active entities on
    30,566 nodes, ratio **0.02×**. Δidempotent=+0 at every checkpoint.
    All 7 LayerTransitions fire naturally: Born 14,810, Split 2,775,
    Merge 13,884, BecameDormant 132, Revived 4 (first natural uncurated
    fire), MembershipDelta 1,052, CoherenceShift 1,388.
  - **Implication for Finding 2a**: `MembershipDelta` / `CoherenceShift`
    / `Revived` demotion shortlist withdrawn. All 7 variants confirmed
    load-bearing.
  - **Ω4 reinterpreted**: same bug amplified EU email explosion.
    Post-fix EU email: 14,624 → 11 active entities at week 115. The
    "dynamic-temporal exceeds contract" claim is retracted.
  - Complementary single-perspective exclusivity (Born + DepositLayer)
    retained — encodes redesign.md §3.4 correctly; ~1–2% fire rate.
  - CLAUDE.md "Feature removal policy" section added to prevent the
    Finding 2a misclassification from recurring.
  - Finding in `docs/hep-ph-finding.md`.
- **Ω4. EU email temporal benchmark.** ✓ **Complete (2026-04-19)**,
  **revised after Ω5**. 986 nodes, 332,334 edges, 115 weekly batches,
  42 department labels. Pre-fix showed active/node=14.8× (14,624
  entities @ week 115) and was classified as "dynamic-temporal exceeds
  engine contract." Ω5 identified `recognize_entities` non-idempotency
  as the actual root cause. Post-fix: **11 active entities @ week 115**,
  ratio 0.01×, Born 177 ≈ Merge 161 (balanced steady state). The
  "out-of-scope" claim is **retracted**. EU email now a valid oracle
  for churn-heavy workloads. NMI=0.078 vs 42 depts reinterpreted as
  "engine finds ~11 cross-dept communication cores, not administrative
  partition" — informative, not a failure. Finding in
  `docs/eu-email-finding.md`.
- **Ω2. 16 → ~13 knob reduction.** ✓ **Partial (2026-04-19)**: 16 → 14.
  `min_activity_threshold`/`min_bridge_activity` demoted to private fields
  with escape-hatch builders — all workspace tests pass. Remaining:
  `demotion_policy` (evidence neutral — no workload distinguishes the 3
  variants yet), `PlasticityConfig.weight_decay` (evidence neutral — needs
  Hebbian workload). Each further reduction requires passing workspace
  tests plus the benchmark suite.
- **Ω3. Track H seed reproduction.** The Entity 73 `Ψ_pair_top3 = +0.0718`
  signal is n=1 at the default seed. Extend `partition_determinism.rs`
  (or a new harness) to rerun `stress_emergence` b=50 across N≥10 seeds.
  Success = positive pair-grain Ψ survives with documented seed-level
  variance. This is the *only* remaining item on Track H; closing it
  firmly retires emergence as an open question.

### Track G — Boundary maturity *(priority 2, shipping visible value)*

`graph-boundary` is wired; nothing downstream consumes it end-to-end.

- **G1. End-to-end boundary example.** `crates/graph-engine/examples/boundary_workflow.rs`:
  load a declared schema, run a real workload, produce a
  `BoundaryReport`, derive `BoundaryAction`s, narrate via `graph-llm`,
  apply a subset. Must exercise all four quadrants. Becomes a cookbook
  anchor under Track I.
- **G2. `prescribe_updates` severity tags.** Each `BoundaryAction`
  gains a `severity: LayerTension` so callers filter by magnitude.
- **G3. Per-entity / per-locus drift breakdown.** `layer_tension`
  currently emits one number; drop it to per-node granularity so
  hotspots are locatable.
- **G4. Regression fixture.** Known-divergent canonical world; assert
  Ghost/Shadow counts within tolerance. Protects G1 once shipped.

### Track I — Public API + cookbook *(priority 3, starts after Ω reaches ≤13 knobs)*

The surface is stable enough to lock once Enron clears.

- **I1. API completeness audit.** Every internal `graph-query` function
  reachable via `api::Query` or explicitly excluded with rationale.
- **I2. `docs/cookbook/`.** One document per canonical pattern (ring
  dynamics, conflict model, knowledge graph, sensor fusion, rumor
  spread, supply chain). Each combines `LocusProgram` +
  `InfluenceKindConfig` + one interesting query. G1 provides the
  boundary-workflow cookbook entry.
- **I3. API stability markers.** `#[doc(hidden)]` on internals,
  `#[deprecated]` on anything about to move, semver policy at the top
  of each crate rustdoc.
- **I4. External transport layer — deferred** until I1–I3 prove the
  surface stable. When reactivated, pick exactly one of CLI / HTTP-JSON
  / gRPC. Do not build all three.

### Track J — LLM integration consolidation

`graph-llm` ships; regression coverage is thin and prompt caching is
not exploited.

- **J1. Mock-backend regression harness.** Assert prompt *structure*,
  not prose, for each `configure_*` / `narrate_*` call.
- **J2. `narrate_boundary(&BoundaryReport)`.** Depends on G1 (consumer
  exists). Ties Track G to Track J.
- **J3. Anthropic prompt caching.** Tag long stable prompts (system +
  graph snapshot) as cache-eligible. Target > 80% hit rate across
  repeated narration on identical snapshots.
- **J4. Counterfactual narration.** Consume `CounterfactualDiff` from
  `graph-query::counterfactual`. Composes D3 with Track J.

### Track K — Diagnostic snapshot

Observability currently requires N separate API calls. One bundled
snapshot makes debugging tractable.

- **K1. `graph-query::DiagnosticSnapshot { metrics, regime, boundary,
  emergence, guard_rail, recent_batches }`.** Owned, serialisable,
  `render_markdown()`. Consumes outputs from Ω3 (emergence) and G3
  (boundary per-locus) so lands after them.
- **K2. CLI integration.** `cargo run --example diagnose` prints the
  snapshot for any example workload at its current batch.
- **K3. Watch loop.** `Simulation::subscribe_diagnostics(stride)` emits
  a snapshot every N batches over the existing world-event channel.
  Cheap when nothing changes (reuse `diff_since`).

### Track Φ — Phase 9 follow-through

P1 shipped (`objective.rs`, `learner.rs`, 5 integration tests).

- **Φ1. P2 competitiveness test.** Add
  `crates/graph-engine/tests/sociopatterns.rs::plasticity_auto_scale_beats_fixed`
  (or an equivalent `phase9_engine` test): same stream, same seed,
  objective-driven scale improves precision@20 over a
  badly-chosen-fixed `learning_rate` — or **rolls back**. Failure
  triggers immediate P1 deletion per `phase9-plasticity-objective.md §10`.
- **Φ2. P3 step-rule revisit.** Only if P2 ships and the placeholder
  loss-band rule proves insufficient. Choices documented in
  `docs/phase9-plasticity-objective.md §8` (observation carries
  `prev_loss` vs. stateful `PerKindLearnable`).
- **Φ3. Close `docs/phase9-plasticity-objective.md §8` open questions**
  before any extension work — step rule choice, crate location
  (engine vs. query), builder vs. runtime-set.

### Track N — Open D-items

- **D2. Lagged mutual information score.** Reuse
  `gaussian_mi_from_series` (Track H machinery) against lagged locus
  signals; compare to D1's STDP weight. Ships as
  `graph-query::causal_strength::mi_score`.

### Track L — Performance follow-ups *(gated, on-demand)*

Do not start until an actual workload regresses. Each item requires a
committed flamegraph showing the site is the current hotspot.

- **L1.** `sim.step()` cascade cost under `GRAPH_ENGINE_PROFILE=1`,
  N=1000/10000 on conflict-model + `stress_emergence`.
- **L2.** `PropertyStore` hash-lookup — `FxHashMap` if not already,
  interning for string keys.
- **L3.** `StateVector` allocation via dhat-rs on `neural_population`
  N=2000; `SmallVec<[f32; 4]>` only if heap volume justifies it.
- **L4.** `Storage::commit_batch` fsync — expand `benches/storage.rs`
  to cover realistic batch sizes.
- **L5.** Phase-9 `score_window` cost. `xlarge/score_window` is
  ~0.42 s per window per `docs/phase9-benchmarks.md`. Becomes L-gated
  only if Φ2 makes this a tuning-loop hotspot.

### Track M — Determinism + replay fidelity

- **M1. Determinism harness.** Extend `tests/partition_determinism.rs`
  into a general `tests/determinism_harness.rs`: N=500 ring + dense
  workloads, seed-locked, assert world-hash equality across 10
  consecutive runs. **Overlaps Ω3** — share infrastructure.
- **M2. Determinism contract.** `docs/determinism.md`: conditions under
  which bits diverge (thread count, rayon scheduling, feature flags),
  recovery path.
- **M3. Replay fidelity.** Prove `counterfactual_replay(world, remove: [])`
  reconstructs original state across 100 seeds.
- **M4. Cross-platform — deferred**, opt-in behind a `deterministic`
  feature flag that disables rayon for the batch loop. Overhead budget
  2×. Reactivate only on explicit user demand.

---

## 3. Dependency graph

```
(frozen)  Phase 0 → Phase 1 → Phase 2 → Phase 3 (E3, E2)   E4 rejected
                                        │
                   ┌────────────────────┼────────────────────┐
                   ▼                    ▼                    ▼
                Track Ω              Track G              Track N (D2)
            (Enron + 16→13)        (boundary)
                   │                    │
                   │ Ω3 (seed repro)    │ G1 unlocks J2
                   ▼                    ▼
                Track Φ              Track J
             (Phase 9 P2/P3)       (LLM narrate)
                   │                    │
                   └─────────┬──────────┘
                             ▼
                          Track K
                     (diagnostic snapshot)
                             │
                             ▼
                          Track I
                     (cookbook + API lock)
                             │
                             ▼
                       Track L / M
                    (perf / determinism, gated)
```

- Ω1 (Enron) is the single immediate blocker for Ω2 (16→~13) and for
  locking Track I's API surface.
- G1 unlocks J2 and one cookbook entry (I2).
- K composes G3 + Ω3 outputs, so lands after both.
- L/M items are on-demand. M1 naturally overlaps Ω3 — ship once.

---

## 4. Non-goals

Holding these explicit so scope creep is visible.

- **Not a distributed system.** Single-process only. Cross-machine
  replication is out of scope (`docs/redesign.md §9`).
- **Not a query language.** `graph-query::api::Query` is the surface.
  No Cypher / SPARQL / GraphQL parser.
- **Not a visualization tool.** `to_dot_named` stays minimal; visual
  debugging UIs are out of scope. External tooling consumes DOT.
- **Not a universal RAG backend.** `graph-llm` integrates with the
  substrate; it is not a general retrieval-augmented generation service.

---

## 5. Scope adjustments log

Decisions that changed item scope. Most recent first. Pre-2026-04-18
entries archived in `docs/archived/phase2-state.md` history.

- **2026-04-19 — Ω5 complete: `recognize_entities` fixpoint fix shipped.**
  Investigation on HEP-PH 60m stress region isolated non-idempotent
  `recognize_entities` as root cause of entity explosion — not
  hub-membership accumulation as initially hypothesised, not EU email
  churn incompatibility either. Single-pass `perspective.recognize`
  produced a proposal set that a second pass immediately collapsed via
  late Merges; the residue accumulated every tick. Fix in
  `crates/graph-engine/src/engine/world_ops.rs`: fixpoint loop (max 8
  passes). 229-test regression suite unchanged. Post-fix HEP-PH 122m:
  716 active entities on 30,566 papers (ratio 0.02×). Post-fix EU email
  115w: 11 active entities on 986 nodes (14,624 → 11, −99.92%). Both
  Ω4 and Ω5 "failure mode" narratives retracted. Finding 2a demotion
  shortlist withdrawn — all 7 LayerTransitions confirmed load-bearing.
  CLAUDE.md gains "Feature removal policy" binding future demotions to
  three-axis diversity verification.
- **2026-04-19 — Ω5 HEP-PH discovery run (PRE-FIX, superseded).** 34,546
  papers / 421K citations. Initial framing: 24m contract region + 60m
  stress region with hub-membership failure mode. Superseded by
  non-idempotency discovery above.
- **2026-04-19 — Ω4 EU email discovery run.** 986 nodes, 332,334 edges,
  115 weekly batches. Entity lifecycle code verified correct (Split →
  Dormant in `entity_mutation.rs:460–483`). Active-entity explosion is a
  dataset property (highly dynamic temporal communities), not a bug.
  Auto-threshold not the cause (discriminating experiments). DECAY tuning
  is a partial lever. Finding in `docs/eu-email-finding.md`.
- **2026-04-19 — Ω2 partial: 16 → 14 knobs.** `min_activity_threshold`
  (DefaultEmergencePerspective) and `min_bridge_activity`
  (DefaultCoherePerspective) demoted to private fields. Escape-hatch
  builders `with_min_activity_threshold` / `with_min_bridge_activity`
  retained. 9 examples converted to auto path; 7 test overrides use
  builder. `docs/complexity-audit.md` Ω2 row added.
- **2026-04-19 — Ω1 Enron benchmark complete.** 6-test harness
  (`crates/graph-engine/tests/enron.rs`): Born/Merge/Dormant/Revived
  lifecycle over 120 nodes, 5 phases. `precision@20/50/100 = 1.000`.
  First Revived exercise in test tree. `CoherenceShift = 1` (reachable,
  not a failure). Finding in `docs/enron-finding.md`;
  `docs/complexity-audit.md` updated with Finding 3 + dataset queue.
  `min_activity_threshold` auto-path confirmed across all 5 datasets —
  Ω2 demotion unblocked.
- **2026-04-19 — Roadmap reorganised around evidence loop.** Tracks
  renumbered; complexity reduction promoted to Track Ω (priority 1)
  alongside Track H seed reproduction (the only H item left). Phase 9
  P1 acknowledged as shipped; P2/P3 folded into Track Φ with explicit
  rollback gate. Old Tracks G/I/J/K/L/M preserved with status audited
  against code (not just docs). E4 status re-confirmed as binding
  rejection.
- **2026-04-18 — Phase 9 P1 shipped.** `PairPredictionObjective`,
  `PlasticityLearners`, 5 integration tests in `phase9_engine.rs`,
  Criterion bench `phase9_plasticity.rs`. `activity` ranking proven
  insensitive to `learning_rate`; `strength = activity + weight` is
  the revised default ranking signal.
- **2026-04-18 — Complexity reduction 47 → 16.** Phases 1–8 executed
  in sequence. Details in `docs/complexity-audit.md` §Phases 1–8. 810+
  workspace tests passing.
- **2026-04-18 — Track H affirmative closure gate triggered.**
  `psi_synergy_leave_one_out` shipped; 0/42 sign flips on Entity 73.
  Remaining: seed reproduction (now Ω3). See
  `docs/emergence/h4-report.md` for full five-pass history.
- **2026-04-18 — E4 rejected permanently.** Parallel Apply + Parallel
  Dispatch measured +0.3–2.4% overhead on every workload. Partition
  parallelism on the batch loop will not be reattempted. See
  `docs/e4-design.md §12` and memory `feedback_parallelization.md`.
- **2026-04-17 — Phase 2 structural fix.** `DefaultCoherePerspective::cluster`
  reduced from O(E²×R) to O(E×M + R) via `locus→entity` index. 17.8×
  speedup at N=1000.

---

## 6. Living documents

- `docs/redesign.md` — substrate ontology (authoritative framing).
- `docs/identity.md` — resolved design decisions.
- `docs/complexity-audit.md` — knob inventory + Findings (living).
- `docs/emergence/h4-report.md` — Track H five-pass history.
- `docs/phase9-plasticity-objective.md` — Phase 9 P1 design + P2/P3 gates.
- `docs/phase9-benchmarks.md` — Phase 9 suitability + efficiency numbers.
- `docs/performance-priorities.md` — five hot-path priorities from the
  post-refactor staging.
- `docs/sociopatterns-finding.md` — precision@K as supervised metric.
- `docs/enron-finding.md` — 5-phase Enron results + Ω2 evidence.
- `docs/eu-email-finding.md` — EU email temporal run + entity lifecycle correctness proof.
- `docs/hep-ph-finding.md` — HEP-PH citation network + accumulative regime contract confirmation.
