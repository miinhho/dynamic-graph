# Roadmap

Last updated: 2026-04-19 (post-Ω5 fixpoint fix).

This document plans forward-looking work. The substrate (`docs/redesign.md`)
is feature-complete; the current chapter is **making the surface honest**:
shrinking the tuning vocabulary to what benchmarks actually require, proving
emergence claims are falsifiable, and turning declared-vs-observed drift
into a visible workflow.

**2026-04-19 correction**: Ω4 (EU email) and Ω5 (HEP-PH) initially framed
as "data-characteristic failure modes" were both the same latent
non-idempotency bug in `recognize_entities`, now fixed via fixpoint loop.
EU email 14,624 → 11 active entities; HEP-PH 122m converges to 716
entities on 30,566 papers. All seven `LayerTransition` variants confirmed
load-bearing. See `docs/hep-ph-finding.md` and `docs/eu-email-finding.md`.

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
   → LFR → SocioPatterns → Enron → EU email → HEP-PH — is the ground
   truth for which abstractions are load-bearing. Each produces a Finding
   in `docs/complexity-audit.md`.
   **Bias correction (2026-04-19)**: planted / curated / sub-critical
   datasets systematically under-trigger gradual-drift features. New
   demotion proposals must pass the three-axis diversity check in
   `CLAUDE.md` "Feature removal policy". HEP-PH rescued three
   `LayerTransition` variants (`MembershipDelta`, `CoherenceShift`,
   `Revived`) from the demotion shortlist.
4. **Non-idempotent passes are bugs.** `recognize_entities` now iterates
   to fixpoint (max 8 passes). Any perspective that does not converge
   within the budget surfaces via `last_recognize_unconverged_proposals()`
   — investigate before shipping.
5. **Declarative anchor + observed drift is the differentiator.** The
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
- **Ω3. Track H seed reproduction.** ✓ **Closed (2026-04-20)**. At the
  original `size=100 batches=50` parameters the post-Ω5 engine produces
  0 positive-Ψ entities across 16 seeds (Entity 73's +0.0718 was a
  pre-fix artefact of accumulated non-idempotency residue). Recalibrated
  to `size=200 batches=100` on 17 seeds (1..15, 42, 100) via new
  `stress_emergence --seed <S> --psi-csv` flags: **7/17 seeds (41%)**
  produce at least one entity with `Ψ_pair_top3 > 0`, range +0.055 to
  +0.223 (mean ≈ +0.15), and **0/64 LOO sign flips** across every
  positive entity × component drop pair. Signal survives seed variance,
  survives LOO ablation unconditionally, and is reproducible with one
  shell loop. Track H closed. Table + procedure in
  `docs/emergence/h4-report.md §0*** "Seventh pass — Ω3"`.
- **Ω6. Post-fix housekeeping (next up).** Close loose ends from the Ω5
  fixpoint investigation.
  - **Ω6a. Entity-size sanity check on HEP-PH max=4,205.** ✓ **Closed
    (2026-04-20)**. 8 arxiv papers sampled across the 1992–2002
    id-range: all `hep-ph` primary category, subjects cluster in one
    subfield (flavor physics + precision QCD / B-meson decays — the
    dominant HEP-PH topic of the era). No threshold revisit. Probe
    retained in `tests/hep_ph.rs` behind `HEP_PH_DUMP_TOP=N`. Result
    table in `docs/hep-ph-finding.md §3 "Structural properties"`.
  - **Ω6b. Exclusivity filter audit.** ✓ **Closed (2026-04-20,
    retained)**. Ablation hatch `OMEGA6B_DISABLE_EXCLUSIVITY=1` added.
    Final `active` count is invariant across all 7 tested workloads
    (HEP-PH × 3 DECAY, Karate, Davis, SocioPatterns, LFR, Enron, EU
    email) — the Ω5 fixpoint wrapper subsumes the filter's effect on
    steady-state count. However: Revived shifts +46%/+50% on HEP-PH
    DECAY=0.5/0.9 with the filter off (13→19, 4→6), so the event log
    is not invariant. CLAUDE.md "Feature removal policy" blocks
    deletion: the hub-heavy × accumulative quadrant has n=1 coverage
    (HEP-PH only) and `redesign §3.4` is not proven to be a fixed
    point of the convergence loop. Retained with hatch for future
    re-evaluation when the next hub-heavy accumulative workload lands.
    Table in `docs/hep-ph-finding.md §3 "Exclusivity filter ablation"`.
  - **Ω6c. `MAX_FIXPOINT_PASSES` calibration.** ✓ **Closed
    (2026-04-20)**. `OMEGA6C_PROBE=1` env-gated stderr trace added in
    `engine/world_ops.rs::recognize_entities`. All six non-HEP-PH
    datasets converge in ≤4 passes at their native DECAY (Karate /
    Davis / SocioPatterns / LFR / Enron = 2, EU email = 4; all
    unconverged=0). Cap=8 held. Raising to 16 tested and reverted:
    final `active=319` identical on HEP-PH DECAY=0.98 but cap=16 emits
    80 transient Born→Dormant pairs and increases cap-hit frequency
    (3 → 5) — cap hits reveal a 2-proposal perspective oscillation
    that longer loops re-traverse, not slow convergence. Table in
    `docs/hep-ph-finding.md §3 "Fixpoint cap calibration"`.
    **Follow-up (new track item)**: `Ω7. Perspective oscillation at
    high-DECAY accumulative regime.** `DefaultEmergencePerspective`
    emits a 2-proposal cycle on HEP-PH DECAY=0.98 (residue
    2 proposals at cap-hit; `flush_relationship_decay` absorbs each
    tick's residue so correctness holds). Diagnose and flatten. Not
    blocking — correctness is maintained — but named so it isn't
    forgotten.
  - **Ω6d. DECAY ∈ {0.5, 0.98} on HEP-PH 122m.** ✓ **Closed
    (2026-04-20)**. All 3 DECAY values (0.5 / 0.9 / 0.98) converge
    idempotently across the full corpus; all 7 `LayerTransition`
    variants fire on each. Active count scales inversely with DECAY
    (1096 / 716 / 319), max size scales directly (1952 / 4205 / 4887),
    Revived count scales inversely (13 / 4 / 1). Monotonic, matches
    decay-knob semantics. Handoff to Ω6c: DECAY=0.98 hits the
    `MAX_FIXPOINT_PASSES=8` cap 3× in the last 10 months (residual
    proposals 2–4, `Δidempotent=+0`). Table in
    `docs/hep-ph-finding.md §3 "DECAY sweep at 122m"`.

### Track G — Boundary maturity *(priority 2, shipping visible value)*

`graph-boundary` is wired; nothing downstream consumes it end-to-end.

- **G1. End-to-end boundary example.** ✓ **Closed (2026-04-20)**. Shipped
  as `crates/graph-llm/examples/boundary_workflow.rs` (placed in
  `graph-llm` because `graph-llm` already depends on `graph-engine` +
  `graph-boundary` + `graph-schema`; reversing the dep would be
  circular). Walks the full pipeline against an 8-person org chart:
  `SchemaWorld::assert_fact` × 7 → `interact()` over 6 active pairs
  × 6 rounds → `analyze_boundary` (5 confirmed / 2 ghost / 1 shadow /
  tension 0.375) → `prescribe_updates` (2 retractions + 1 assertion) →
  `narrate_prescriptions` via `MockLlmClient` (hermetic, swap for
  Anthropic / Ollama client for real model) → `apply_prescriptions` →
  re-analyse (6 confirmed / 0 ghost / 0 shadow / tension 0.000). Four
  quadrants all exercised: Confirmed by active declared pairs, Ghost
  by Carol/Dave never-interact, Shadow by Alice↔Eve cross-team, Null
  by the silent majority. Run: `cargo run -p graph-llm --example
  boundary_workflow`. Unlocks J2 (`narrate_boundary`) and is the
  boundary-workflow anchor for the cookbook under Track I.
- **G2. `prescribe_updates` severity tags.** Each `BoundaryAction`
  gains a `severity: LayerTension` so callers filter by magnitude.
- **G3. Per-entity / per-locus drift breakdown.** `layer_tension`
  currently emits one number; drop it to per-node granularity so
  hotspots are locatable.
- **G4. Regression fixture.** ✓ **Closed (2026-04-20)**. Shipped as
  `crates/graph-llm/tests/boundary_regression.rs`. Three tests pin the
  canonical boundary_workflow scenario: (a) analyze_boundary produces
  confirmed=5 / ghost=2 / shadow=1 / tension≈0.375 ±0.01; (b)
  prescribe_updates at default config yields exactly 2 retractions and
  1 assertion, and the assertion pair is Alice↔Eve; (c) after
  apply_prescriptions the boundary is aligned (ghost=0, shadow=0,
  tension=0.0). Any engine change that shifts auto-emergence,
  plasticity, decay, or the analyze_boundary matching logic fails this
  fixture before reaching the example.

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
- **J2. `narrate_boundary(&BoundaryReport)`.** ✓ **Closed (2026-04-20)**.
  `graph_llm::narrate_boundary(client, report, world, names)` describes
  the raw Confirmed / Ghost / Shadow state (aligned worlds return a
  canned message without hitting the LLM). Paired with existing
  `narrate_prescriptions`: the first *observes*, the second *decides*.
  Also added `GraphLlm::narrate_boundary(&schema)` facade method and
  wired both narrators into `examples/boundary_workflow.rs`. All 25
  `graph-llm` unit tests pass including 2 new MockLlmClient-driven
  cases for `narrate_boundary` (aligned → canned message, drift →
  client invoked).
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

- **2026-04-19 — Ω5 post-validation + Ω6 scoped.** EU email three
  scenarios all converge post-fix (17 / 11 / 24 active entities; NMI
  0.10 / 0.08 / 0.18). HEP-PH checkpoint-delta analysis identifies
  active-count dips at m48/m96/m102 as **natural subfield Merge waves**
  (ΔMerge > ΔBorn) — intended consolidation behaviour, not a bug.
  `last_recognize_unconverged_proposals()` added to surface fixpoint-cap
  hits. Four Ω6 housekeeping items enumerated (entity-size sanity,
  exclusivity audit, fixpoint cap calibration, DECAY sweep at 122m).
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
