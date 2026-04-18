# Roadmap

This document tracks forward-looking work for the engine. The substrate
rewrite (`docs/redesign.md`) and the original Phase 0–3 plan are **done**;
this file now plans the *next chapter* — what the engine does with the
substrate now that it exists.

Last updated: 2026-04-18.

---

## 1. Where we are

The redesign produced nine crates across five observational layers
(Locus → Change → Relationship → Entity → Cohere) plus the static /
declarative side (`graph-schema`, `graph-boundary`) and LLM assist
(`graph-llm`). The substrate is feature-complete against `identity.md §8`
and performance is acceptable after the Phase 3 shake-out.

### 1.1 Completed phases (frozen)

| Phase | Track / item | Outcome |
|---|---|---|
| Phase 0 | Landing (`controller.rs`, `handle.rs`, `causal_strength.rs`, `configure.rs`) | All promoted, tested, shipping |
| Phase 1 | E1 — Measurement | `docs/perf/phase1-report.md`; hotspots ranked |
| Phase 2 | Cohere O(E²×R) fix | 17.8× speedup at N=1000; N=10000 viable |
| Phase 3 | E3 (demotion) | `demotion_policy` + hot/cold promotion shipped |
| Phase 3 | E2 (trim summaries) | `ChangeLog::trim_before_batch` + `CoarseTrail` |
| Phase 3 | E4 (partition parallelism) | **Rejected — negative result** (`e4-design.md §12`). Binding. |

### 1.2 In-flight tracks (status audit)

| Track | Item | Status as of 2026-04-18 |
|---|---|---|
| A | A1 — `tokio` async feature gate | Shipping behind `async` feature |
| A | A2 — `subscribe_world_events` stream | Shipping (see `handle.rs`) |
| A | A3 — Bounded `pending_stimuli` + backpressure | Shipping (see `simulation/ingest.rs`) |
| B | B3 — Time-travel (reverse diff replay) | Design doc `docs/b3-time-travel.md` done; `time_travel.rs` in `graph-query` shipping |
| B | B4 — Criterion benches for `graph-query` | Baselines committed under `benches/` |
| D | D1 — Causal-strength into `api::Query` | Shipping (`causal_strength.rs`) |
| D | D2 — Lagged mutual information score | **Not started** (still open) |
| D | D3 — Structural counterfactual replay | Shipping (`counterfactual.rs`) |
| D | D4 — Entity-level causality | Shipping (`entity_causality.rs`) |

**Deferred tracks reactivated:**

- Track C (LLM integration) is no longer deferred — `graph-llm` is shipping
  with Anthropic / Ollama / Mock backends and a `GraphLlm` façade. Remaining
  work folds into the new Track **J** below.
- Track F (domain cookbook / external API) becomes the new Track **I** —
  substrate is stable enough to document a public surface.

### 1.3 Emergent new direction (not captured in the old roadmap)

`crates/graph-query/src/emergence.rs` introduces Ψ-scalar, coherence
autocorrelation, pairwise Φ-ID decomposition, and decay-aware V
reconstruction. Track **H** originally framed this as "validating
whether the emergence is real"; four measurement passes (see
`docs/emergence/h4-report.md`) have since converged on the finding
that Ψ_corrected is uniformly ≤ 0 at the entity-coherence scalar but
pair-level synergies are non-trivial. Track H has been re-scoped
below to reflect this: the new goal is to *characterise the
interaction structure the engine actually produces* rather than to
validate a scalar emergence claim.

---

## 2. Shape of the next chapter

Three framings drive the new plan.

1. **The substrate is done; the story is not.** The engine observes and
   derives; it does not yet *argue*. A user running a real workload should
   get a narrative — "entity X became dormant because Y" — backed by
   causal queries, not a raw change log dump.
2. **Emergence must be falsifiable.** If Ψ is negative for every workload,
   the "emergence" framing is marketing. The Ψ diagnostic is the first of
   several emergence-validity metrics the engine owes the user.
3. **Declarative anchor + observed drift is the differentiator.** Nobody
   else ships `graph-boundary`. Its "Confirmed / Ghost / Shadow" quadrants
   are the cleanest way to make a graph engine *tell you something you did
   not already know*. This needs to be a first-class workflow.

Corollary: performance work returns only when an *observed* user workload
hits a wall. We do not chase benchmarks.

---

## 3. Active tracks

### Track G — Declarative / observed boundary maturity

`graph-boundary` analyses the gap between `graph-schema` (declared) and
`graph-world` (observed). Everything is wired up, nothing is *practised*.

- **G1.** End-to-end boundary workflow example in `crates/graph-engine/examples/`:
  load a declared schema, run a workload, produce a `BoundaryReport`,
  derive `BoundaryAction`s, narrate them via `graph-llm`, apply a subset.
  Must exercise all four quadrants (Confirmed / Ghost / Shadow / Null).
- **G2.** `prescribe_updates` cost classes — each `BoundaryAction` gains a
  `severity: LayerTension` tag so callers can filter ("only show tensions
  above X"). Current output is flat.
- **G3.** Bidirectional drift metric: `layer_tension` currently reports one
  number for the whole graph. Produce a per-entity / per-locus drift
  breakdown so hotspots are locatable.
- **G4.** Regression test — `graph-boundary` integration test that runs a
  known-divergent fixture and asserts Ghost/Shadow counts within a tolerance.

### Track H — Information structure at component grain (was: "emergence validity")

**Original framing**: make the "emergence" claim falsifiable by measuring
Ψ and stopping if Ψ is uniformly negative.

**Current framing (post-H4 four-pass rewrite)**: four successive revisions
(deposit sampling → dense sampling → synergy correction → decay-aware
reconstruction) have produced Ψ_corrected ≤ 0 for every measurable entity
across every reference workload. Two entities now produce Ψ_naive > 0,
but under the redundancy-free joint-MI metric the signal disappears
because `I_joint > Σᵢ I(Xᵢ; V_{t+1})` — the joint of the member
relationships carries information the entity-coherence summary does not.

The honest reading: **causal emergence in the Rosas-Mediano sense is not
at the entity-coherence scalar on this architecture**. It may yet exist
at the *component-interaction* grain, where pairwise synergies are
non-trivial (see top-pair breakdown in `EmergenceSynergyReport`).

Track H therefore shifts from *validating* a scalar emergence claim to
*characterising* the interaction structure the engine actually produces.
Below, H1–H4 are marked done for historical reference; H2b onward are
new items under the re-scoped framing. Detail in
`docs/emergence/h4-report.md`.

#### Completed under the original framing

- **H1 — done.** `emergence.rs` shipped; public exports from
  `graph-query`.
- **H2 — done.** `psi_synergy`, `emergence_report_synergy`, multivariate
  Gaussian joint MI with pairwise PID, per-entity top-K pair attribution.
- **H3 — done.** `emergence_report[_synergy]` both shipping; Markdown
  renderers with naïve + corrected + top-pair columns.
- **H4 — done (five passes).** Ψ distributions published; dense
  sampling, decay-aware V reconstruction, and pair-grain Ψ shipped.
  Full history in `docs/emergence/h4-report.md`.
- **H4.1 (dense sampling), H4.3 (decay-aware), H5 (pair grain) — shipped**.
  H5 fifth pass flipped one entity to `Ψ_pair_top3 > 0` on
  `stress_emergence` b=50 — the closure gate specified below is now
  triggered.

#### Open under the re-scoped framing

- **H2b — interaction graph report.** For each measurable entity, emit
  the full synergy DAG (not just top-K): nodes = member relationships,
  edge weight = pairwise `synergy − redundancy`. Lives alongside
  `EmergenceSynergyReport`. Renders as DOT for visual debugging.
- **H4.2 — leave-one-out robustness.** Per `psi_synergy` entity, drop
  each member relationship and recompute `psi_corrected`. Report the
  relationship whose removal changes `psi_corrected` the most — that is
  the one carrying the most unique information about `V_{t+1}`. Small;
  builds on the existing pairwise machinery.
- **H4.4 — ChangeLog trimming interaction.** `neural_population` loses
  ~99% of its log to trimming, defeating dense sampling. Options:
  (a) add `ChangeLog::trim_retaining_per_relationship(min_n)` to keep
  at least N most-recent changes per relationship; (b) persist an
  entity-relevant subset to a separate summary store; (c) compute Ψ
  incrementally during the run, before trimming. Decide before
  committing to a direction.
- **H5 — done.** `psi_pair_top3 = I_self − Σ_top3 I(X_a, X_b; V_{t+1})`
  plus aggregate pair-synergy fields on `PsiSynergyResult`. Fifth-pass
  result: `Ψ_pair_top3 = +0.0718` on `stress_emergence` b=50 Entity 73
  — first positive pair-grain Ψ across all passes. `total_pair_synergy
  > 0` on every measurable entity (range +0.75 to +16.4 nats).
- **H4.2 — done.** `psi_synergy_leave_one_out` with per-drop
  `Ψ_corrected` / `Ψ_pair_top3`. Entity 73 sixth-pass result:
  **0 / 42 sign flips** on both metrics under single-component
  ablation. Signal is distributed, not dependent on any load-bearing
  component.

**Closure status — affirmative, robust.** The original closure gate
("`Ψ_pair_top3 > 0` somewhere, AND robust") is satisfied. The
remaining open item is *seed reproduction* — the Entity 73 signal is
n=1 at the default seed and should be confirmed across seed
variation. After that, Track H can be declared closed with a
positive-but-narrow result.

**Remaining (non-gating) items**:

- **Seed reproduction** — verify Entity 73's pair-grain Ψ survives
  seed variation on the same workload. Small extension to the
  determinism harness.
- **H2b** — interaction graph DAG (DOT export).
- **H4.4** — ChangeLog trimming interaction so `neural_population`
  becomes measurable.
- **Extend to more workloads** — map the distribution of pair-grain
  emergence across parameter space.

### Track I — Public API surface + cookbook (was Track F)

Lock the outward-facing API and write the missing documentation.

- **I1.** Audit `graph-query::api::Query` for completeness: every internal
  query function should be reachable via the declarative API or explicitly
  excluded with a one-line rationale.
- **I2.** `docs/cookbook/` — one document per canonical workload pattern
  (ring dynamics, conflict model, knowledge graph, sensor fusion, rumor
  spread, supply chain). Each combines a minimum `LocusProgram`, an
  `InfluenceKindConfig`, one interesting `graph-query` query, and (if
  relevant) a boundary analysis.
- **I3.** API stability markers — `#[doc(hidden)]` on internals,
  `#[deprecated]` on anything about to change, semver policy written at the
  top of each crate-level rustdoc.
- **I4.** External transport layer — **deferred** until I1–I3 prove the
  surface is stable. When reactivated, pick one of: CLI (small, fast) /
  HTTP-JSON / gRPC. Do **not** build all three.

### Track J — LLM integration consolidation (was Track C)

`graph-llm` is shipping but under-documented and under-regression-tested.

- **J1.** Regression harness — record expected outputs for each
  `configure_*` / `narrate_*` / `answer_with_graph` call against the `Mock`
  backend. Assert prompt structure, not prose.
- **J2.** Narrate the boundary — `GraphLlm::narrate_boundary(&report)` that
  consumes a `BoundaryReport` and produces a summary. Ties Track G to
  Track J.
- **J3.** Anthropic prompt caching — tag long stable prompts (system +
  graph snapshot) as cache-eligible. Measure hit rate across repeated
  narration calls; target > 80% on identical snapshots.
- **J4.** Counterfactual narration — take a `CounterfactualDiff` from
  `graph-query::counterfactual` and narrate it. Combines D3 with Track J.

### Track K — Diagnostic snapshot (observability unification)

Today the system exposes `WorldDiff`, `WorldMetrics`, `BoundaryReport`,
Ψ result, adaptive guard-rail state, regime classification, causal trace,
step observation, and profile timing — but only as separate API calls. A
single `DiagnosticSnapshot` that bundles them would make one-shot debugging
tractable.

- **K1.** `graph-query::DiagnosticSnapshot { metrics, regime, boundary,
  emergence, guard_rail, recent_batches }` — owned, serialisable, with a
  `render_markdown()` method.
- **K2.** CLI integration — a `cargo run --example diagnose` that prints
  the snapshot for any example workload at its current state.
- **K3.** Watch loop — `Simulation::subscribe_diagnostics(stride: usize)`
  emits a `DiagnosticSnapshot` every N batches over an existing world-event
  channel. Cheap when nothing changes (reuse `diff_since`).

### Track L — Performance follow-ups from Phase 1

Narrow and measurement-gated. Do not start until an actual workload regresses.

- **L1.** `sim.step()` cascade cost — phase1-report §7 flagged this as next
  investigation. Instrument with `GRAPH_ENGINE_PROFILE=1` and collect
  scaling data at N=1000/10000 for the conflict model and stress_emergence.
- **L2.** `PropertyStore` hash-lookup cost (H4) — benchmark, switch to
  `FxHashMap` if not already, consider interning for string keys.
- **L3.** `StateVector` allocation (H6) — dhat-rs run on `neural_population`
  N=2000; decide on `SmallVec<[f32; 4]>` only if heap volume justifies it.
- **L4.** `Storage::commit_batch` fsync (H5) — `benches/storage.rs` expansion
  to measure WAL path under realistic batch sizes.

**Gate:** each L-item starts only with a committed flamegraph or
measurement showing the site as the current hotspot. No speculative perf
work.

### Track M — Determinism and replay fidelity

Currently `identity.md §6` promises bit-identical replay on the same
platform / toolchain only. Several recent changes (parallel build phase,
rayon `par_iter` in dispatch) have reduced the determinism confidence.

- **M1.** Determinism harness — extend `tests/partition_determinism.rs`
  into a general `tests/determinism_harness.rs`: run N=500 ring and dense
  workloads, seed-locked, assert world hash equality across 10 consecutive
  runs on the same platform.
- **M2.** Document the determinism contract — under what conditions bits
  diverge (thread count, rayon scheduling, feature flags), and what the
  recovery path is. Write as `docs/determinism.md`.
- **M3.** Replay fidelity for `counterfactual_replay` — prove that given a
  committed world, `replay(world, remove: [])` reconstructs the same state.
  Test across 100 seeds.
- **M4.** Cross-platform — **deferred**, opt-in via a `deterministic`
  feature flag that disables rayon for the batch loop. Acceptable overhead
  budget: 2×. Reactivate only on explicit user demand.

### Track N — Open D-items

- **D2.** Lagged mutual information score — previously scoped as
  "Granger-style score: count A's changes followed within N batches by a B
  change". The Track H work on Gaussian MI makes D2 cheap: reuse
  `gaussian_mi_from_series` against lagged locus signals. Compare against
  D1's STDP weight. Deliverable: `graph-query::causal_strength::mi_score`.

---

## 4. Dependency graph

```
(frozen)  Phase 0 → Phase 1 → Phase 2 → Phase 3 (E3, E2)  E4 rejected
                                     │
                                     ▼
                  ┌──────────────────┼──────────────────┐
                  ▼                  ▼                  ▼
               Track G             Track H           Track N (D2)
            (boundary)     (information structure)  (causal MI)
                  │                  │
                  │                  │
                  ▼                  ▼
               Track J            Track K
           (LLM narrate)        (diagnostic snapshot)
                                     │
                                     ▼
                                 Track I
                              (cookbook + API lock)
                                     │
                                     ▼
                                 Track L / M
                           (perf / determinism gates)
```

- G, H, N are independent and can run in parallel.
- J2 (narrate boundary) needs G1 (end-to-end workflow).
- K (diagnostic snapshot) consumes outputs from G/H so lands after at least
  G3 and H1.
- I (cookbook) ossifies the surface — start after G/H/K are stable.
- L/M are on-demand; neither blocks the others.

---

## 5. Scope adjustments log

Decisions that changed item scope. Most recent first.

- **2026-04-18 — H4.2 shipped; Track H closure gate firmly
  triggered.** `graph_query::psi_synergy_leave_one_out` (+ decay
  variant) with per-drop `Ψ_corrected` / `Ψ_pair_top3` deltas. Sixth
  pass on `stress_emergence` b=50 Entity 73: 0/42 sign flips on both
  metrics — the positive `Ψ_pair_top3` baseline is preserved under
  every single-component ablation. Signal is distributed across the
  42-component set, not dependent on any load-bearing component.
  Closure gate ("positive AND robust") now satisfied. Seed
  reproduction (across seeds on same workload shape) is the final
  non-gating robustness check.
- **2026-04-18 — H5 shipped (pair-grain Ψ); Track H closure gate
  triggered.** `PsiSynergyResult` gains `psi_pair_top3` +
  aggregate pair synergy/redundancy fields. Fifth-pass result on
  `stress_emergence` b=50: Entity 73 shows `Ψ_pair_top3 = +0.0718` —
  the first positive pair-grain Ψ across five revisions. Aggregate
  `total_pair_synergy > 0` on every measurable entity (both b=20 and
  b=50), confirming at the aggregate level that emergence lives at
  component-pair grain. Track H's affirmative closure gate is
  triggered; firm closure pending H4.2 (leave-one-out) + seed
  reproduction on Entity 73. See `docs/emergence/h4-report.md §0*,
  §2.0*/**/***`.
- **2026-04-18 — Track H re-scoped + H3 redux shipped.** Track H
  re-framed from "validate emergence (Ψ > 0 somewhere)" to
  "characterise information structure at component grain", following
  the four-pass H4 convergence on Ψ_corrected ≤ 0 at the entity
  scalar. Original H1–H4 + H4.1/H4.3 marked done; new items H2b
  (interaction graph), H4.2 (leave-one-out), H4.4 (ChangeLog trimming
  interaction), H5 (Ψ at pair grain) added. Closure gate specified.
  H3 redux also shipped: `EmergenceSynergyReport::render_markdown` now
  emits a per-entity top-pair attribution table (rel_a, rel_b,
  synergy, joint_mi, redundancy, mi_a, mi_b).
- **2026-04-18 — H4.3 shipped (decay-aware V reconstruction).**
  Introduced `graph_query::DecayRates` and `*_with_decay` variants of
  the emergence functions; `Simulation::activity_decay_rates()`
  publishes per-kind `decay_per_batch` from the registry. Fourth-pass
  results: two entities now cross Ψ_naive > 0 on `stress_emergence`
  (Entity 16: +0.2386 at b=20, Entity 74: +0.0606 at b=50) — the first
  positive naïve Ψ the pipeline has produced across four revisions.
  **Ψ_corrected remains ≤ 0 for every measurable entity**, so the
  synergy-corrected verdict (redundancy-free) is unchanged from the
  third pass: coherence does not predict its own future beyond what
  the joint of member relationships predicts. See
  `docs/emergence/h4-report.md §0a, §2.0a–e`.
- **2026-04-18 — H2 shipped (synergy-corrected Ψ).** Implemented
  `graph_query::psi_synergy` and `emergence_report_synergy` with
  multivariate Gaussian joint MI (OLS R²) replacing the naïve
  `Σᵢ I(Xᵢ; Y)`. Pairwise MMI-style PID decomposition surfaces the
  top-K synergistic component pairs per entity. Rerun on the three
  reference workloads: `psi_corrected ≤ 0` on every measurable entity
  across every workload; redundancy correction shrinks magnitudes but
  does not flip signs. The naïve-negative Ψ was therefore **not**
  purely a redundancy artefact. Implications: the falsification
  criterion is much closer to being triggered; Track H should be
  re-scoped from "validate emergence" to "characterise component-grain
  information structure". See `docs/emergence/h4-report.md §4–§5`.
- **2026-04-18 — H4.1 shipped (dense Ψ sampling).** Option A from the
  first H4 report implemented: `psi_scalar` now samples coherence at
  every batch where a member relationship had a ChangeLog entry,
  instead of only at deposit events. Measurable-entity counts rose
  4–8× on `stress_emergence` (sample sizes 10–55 vs. 3–4). Ψ remains
  uniformly ≤ 0 across every workload, with `Σ I(X_i; V_{t+1})`
  dominating `I(V_t; V_{t+1})` by 1–2 orders of magnitude. H2
  (synergy/redundancy decomposition) is promoted from optional to
  top-priority follow-up because the dominant term is likely inflated
  by component redundancy. See `docs/emergence/h4-report.md`.
- **2026-04-18 — Post-substrate reset.** Phase 0–3 declared complete.
  Remaining A/B/D items audited: A1–A3, B3, B4, D1, D3, D4 all shipping.
  D2 remains open and folded into Track N. Deferred tracks C (LLM) and F
  (cookbook/API) reactivated as Tracks J and I. New tracks added: G
  (boundary maturity), H (emergence validity), K (diagnostic snapshot), L
  (perf follow-ups, gated), M (determinism).
- **2026-04-18 — E4 rejected permanently.** Parallel Apply + Parallel
  Dispatch measured +0.3~2.4% overhead on every workload. Binding: do not
  re-attempt partition-level parallelism on the batch loop. Kept for
  inspection: `PartitionIndex`, `PartitionAccumulator`,
  `partition_determinism.rs`, `e4_*` benches (regression guard). See
  `docs/e4-design.md §12` and memory `feedback_parallelization.md`.
- **2026-04-17 — Phase 2 structural fix.** `DefaultCoherePerspective::cluster`
  reduced from O(E²×R) to O(E×M + R) via `locus→entity` index. 17.8×
  speedup at N=1000 (517ms → 29ms). This superseded E3 as the immediate
  throughput win but did not remove E3's rationale for `sim.step()`
  cascade cost.
- **2026-04-17 — BCM plasticity added.** `PlasticityConfig::bcm_tau`
  introduces Bienenstock-Cooper-Munro alongside Hebbian and STDP.
  θ_M persisted in World / Storage / Snapshot.
- **D3 narrowed.** Scope reduced to structural counterfactual replay (no
  LLM narration) when Track C was deferred. LLM narration of
  counterfactuals is now Track J4.
- **B3 approach decision.** Chose (b) WorldDiff reverse replay over (a)
  WAL + snapshot replay. Cost concern was flagged; design doc
  `docs/b3-time-travel.md` written before implementation.

---

## 6. Non-goals

Holding these explicit so scope creep is visible.

- **Not a distributed system.** Single-process only. Cross-machine
  replication is out of scope (see `docs/redesign.md §9`).
- **Not a query language.** The `graph-query::api::Query` declarative form
  is as far as we go. No Cypher / SPARQL / GraphQL parser.
- **Not a visualization tool.** `to_dot_named` is deliberately minimal;
  visual debugging UIs are out of scope. External tooling can consume DOT.
- **Not a universal RAG backend.** `graph-llm` integrates with the
  substrate; it is not a general retrieval-augmented generation service.
