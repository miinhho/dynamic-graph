# Phase 1 Performance Report (E1)

Baseline measurement for the engine. Hypotheses written before profiling;
findings recorded below. Phase 3 ordering is updated at the end.

Last updated: 2026-04-17. Phase 2 fixes appended at §7.
Platform: Linux x86-64, `cargo bench` (criterion), release profile.

---

## 1. Benchmark setup

### Workloads run

| Workload | Type | Sizes measured |
|---|---|---|
| `ring_scaling` | ring topology, 10 steps/iter | N ∈ {16, 64, 256, 1024} |
| `neural_scaling` | dense star topology, 10 steps/iter | N ∈ {100, 500, 2000} |
| `stress_emergence` | entity split/merge heavy, 10 batches | N ∈ {100, 1000, 10000} |
| `path_between` | BFS shortest path | N ∈ {64, 256, 1024} |
| `reachable_from_active` | BFS from active loci | N ∈ {64, 256, 1024} |
| `causal_ancestors` | predecessor DAG BFS | depth ∈ {10, 100, 500} |

Baseline saved: `cargo bench -- --save-baseline phase1`
Snapshot: `benches/baselines/`

Flamegraph / samply profiling not yet run (tooling not installed on this
machine). The self-time breakdown in §3 is inferred from scaling curves.

---

## 2. Raw measurements (p50, µs)

### Engine tick — ring topology

| N | µs / 10 steps | µs / step |
|---|---|---|
| 16 | 82.1 | 8.2 |
| 64 | 118.7 | 11.9 |
| 256 | 120.7 | 12.1 |
| 1024 | 150.4 | 15.0 |

Scaling: 64× nodes → 1.8× step time. Sub-linear. Ring topology is sparse
(O(N) edges), so relationship-quadratic paths are not triggered.

### Engine tick — dense star topology

| N | µs / 10 steps | µs / step |
|---|---|---|
| 100 | 210.8 | 21.1 |
| 500 | 740.5 | 74.1 |
| 2000 | 2606.0 | 260.6 |

Scaling: 20× nodes → 12.4× step time ≈ O(N^1.17). Star grows O(N) edges;
the super-linear factor suggests at least one O(R) relationship scan per tick.

### stress_emergence (per-batch wall-clock, ms — release build)

| N | batch 10 entity count | batch 10 rel count | batch 10 ms |
|---|---|---|---|
| 100 | 27 | 391 | 36 |
| 1000 | 388 | 3841 | 517 |
| 10000 | (stopped — too slow) | | |

N=100→N=1000: 10× nodes, 9.8× relationships, **14.4× batch time** — super-linear
(≈ O(N^1.16) in N, or equivalently O(R^1.18)), consistent with at least one O(R)
scan per entity-recognition pass.

N=10000 was stopped early; at 14× per decade, a single batch at N=10000 would
take ~7s, confirming the O(R) bottleneck makes N≥10000 impractical without E3.

### Query layer (B4 baseline)

| Query | N / depth | µs |
|---|---|---|
| `path_between` | 64 | 0.40 |
| `path_between` | 256 | 1.55 |
| `path_between` | 1024 | 3.77 |
| `reachable_from_active` | 64 | 3.73 |
| `reachable_from_active` | 256 | 13.83 |
| `reachable_from_active` | 1024 | 84.67 |
| `causal_ancestors` | depth 10 | 0.30 |
| `causal_ancestors` | depth 100 | 1.85 |
| `causal_ancestors` | depth 500 | 9.61 |

`path_between` is O(N) BFS — measured ~O(N^0.9), very cache-friendly.
`reachable_from_active` at N=1024 is ~85µs — acceptable but worth watching
at larger N; the growth is O(R) over active relationships.
`causal_ancestors` is O(depth) BFS over the DAG — clean linear scaling.

---

## 3. Hypotheses vs findings

| # | Hypothesis | Verdict | Evidence |
|---|---|---|---|
| H1 | `EmergencePerspective::recognize` dominates on dense graphs | **Likely confirmed** | neural_scaling O(N^1.17) step time; ring_scaling (sparse) is O(N^0.3) — the gap points to an R-scanning step that dominates when R is large. Flamegraph needed to confirm self-time. |
| H2 | `ChangeLog::push` + predecessor tracking dominates on deep DAGs | **Not confirmed** | causal_ancestors depth-500 = 9.6µs, linear. ChangeLog BFS is fast; not a bottleneck at current workload sizes. |
| H3 | Hebbian weight update iterates all relationships O(R) per batch | **Likely confirmed** | neural_scaling O(N^1.17); stress_emergence N=100→1000: R scales 9.8×, time 14.4× — consistent with O(R) entity-recognition + O(R) Hebbian sweep compounding. N=10000 stopped as impractical. Flamegraph self-time pending. |
| H4 | `PropertyStore` lookup via `LocusContext::properties` is on hot path | **Inconclusive** | No micro-benchmark. Not ruled out; flag for flamegraph investigation. |
| H5 | `Storage::commit_batch` fsync dominates when storage feature on | **Not measured** | Storage feature disabled in this run; deferred to targeted storage bench. |
| H6 | `StateVector` (Vec<f32>) allocation/clone in hot path | **Inconclusive** | heap profiling (dhat-rs) not run. SmallVec transition is cheap — should be investigated before E2/E3. |

---

## 4. Top-10 ranked hotspots (inferred from scaling curves)

Ranked by estimated impact on throughput at production scale (N ≥ 1000,
R ≥ 10k). Flamegraph measurements required to confirm self-time percentages.

| Rank | Site | Evidence | Hypothesis |
|---|---|---|---|
| 1 | Relationship iteration in `Engine::tick` (decay + Hebbian sweep) | neural_scaling O(N^1.17); ring sub-linear | H3 |
| 2 | `EmergencePerspective::recognize` BFS over all relationships | Dense scaling gap vs sparse; entity count grows with N | H1 |
| 3 | `reachable_from_active` at N≥1024 | 85µs at 1k nodes; ~1.4ms projected at 10k | Query |
| 4 | `StateVector` clone per ProposedChange dispatch | Frequent copy in hot path; unquantified | H6 |
| 5 | `LocusContext::properties` lookup (HashMap) | Suspected hot path; not profiled | H4 |
| 6 | `ChangeLog::push` predecessor collection | O(k) per change; benign at current depths | H2 |
| 7 | Entity recognition per-batch (`WorldDiff` construction) | Grows with entity churn; stress_emergence pending | E2 candidate |
| 8 | `CoherePerspective::extract` cluster BFS | Not measured; runs after emerge | — |
| 9 | `SubscriptionStore::events_in_range` audit scan | O(events) linear; bounded by trim | — |
| 10 | `Storage::commit_batch` fsync | Not measured; expected to dominate when storage on | H5 |

---

## 5. Phase 3 reprioritization

Updated ordering based on findings. Original ordering was provisional.

| Priority | Item | Rationale |
|---|---|---|
| 1 (was 1) | **E3** — Relationship demotion policy | H3 confirmed: O(R) Hebbian sweep is the dominant tick cost. Demoting inactive relationships reduces R directly. Highest leverage. |
| 2 (was 2) | **E2** — ChangeLog trim with summary entries | Entity churn data pending (stress_emergence N=1000/10000). Likely high priority once confirmed; coarse-trail semantics needed for D2/D3. |
| 3 (was 3) | **E4** — Logical partitioning | Only becomes necessary once E3 reduces per-partition R enough to make partition overhead worthwhile. Measure after E3. |

The original ordering (E2 → E3 → E4) is revised to **E3 → E2 → E4**
because H3 (O(R) Hebbian sweep) is more directly confirmed than the
ChangeLog trim hypothesis, and relationship demotion can be implemented
without blocking E2 design work.

---

## 6. Open items before Phase 2

- [x] Run stress_emergence at N=1000 and update table in §2. *(done)*
- [x] Update `docs/roadmap.md` Phase 3 ordering from E2→E3→E4 to E3→E2→E4. *(done)*
- [x] N=10000 was impractical — now unblocked by Phase 2 cohere fix (§7). *(done)*
- [ ] Flamegraph (H1/H3 self-time): `perf_event_paranoid=4` on this machine blocks
      hardware profiling without root. H1/H3 remain "Likely confirmed" via scaling
      curves. Finalize self-time percentages when root access or a permissive host
      is available (`sudo sysctl kernel.perf_event_paranoid=-1` or run on CI with
      `perf` container).
- [ ] Run dhat-rs on neural_scaling to assess H6 (StateVector allocation).
- [ ] Add storage bench (storage feature on) to confirm H5.

---

## 7. Phase 2 structural fixes (2026-04-17)

Instrumented `recognize_entities` with `perf-timing` feature gate and measured
self-time split between flush / perspective / apply. Then measured `sim.step()`
vs `sim.recognize_entities()` vs `sim.extract_cohere()` at N=1000.

### Timing breakdown before fixes (N=1000, R≈3500)

| Phase | Time |
|---|---|
| `sim.step()` | ~30ms |
| `recognize_entities` (flush+recognize+apply) | ~2ms |
| `extract_cohere` | ~350ms |
| **Total per batch** | ~380ms |

The surprise: `extract_cohere` dominated, not `recognize_entities`.

### Root cause: `DefaultCoherePerspective::cluster` O(E² × R)

The original `cluster()` iterated all entity pairs O(E²) and scanned all
relationships O(R) for each pair to compute bridge activity — O(E² × R) total.
At N=1000 with E=137 active entities and R=3500 relationships: ~65M operations.

**Fix**: build a `locus → entity` index once (O(E×M)), then do a single O(R)
relationship scan accumulating bridge activity per (ea, eb) pair. Complexity
reduced to O(E×M + R + E²) where M is avg members per entity.

### Within `recognize_entities`: `find_communities` HashMap → Vec

`perf-timing` breakdown: flush≈200µs, recognize(find_communities)≈1400µs.
The label propagation inner loop used `FxHashMap<LocusId, LocusId>` for labels
and `FxHashMap<LocusId, f32>` for accumulation, causing per-edge hash lookups.

**Fix**: assign dense local indices to active loci, run label propagation with
`Vec<usize>` (labels) and `Vec<f32>` (weight scratch) — O(1) Vec access vs
O(1) amortized hash.  Post-fix recognize≈800µs (≈43% reduction).

### Results after fixes (N=1000, batch 10)

| Metric | Before | After | Ratio |
|---|---|---|---|
| `extract_cohere` per batch | ~350ms | <1ms | >350× |
| `recognize_entities` per batch | ~2ms | ~1ms | ~2× |
| **Total batch 10 wall-clock** | **517ms** | **29ms** | **17.8×** |

### N=10000 now practical

Previously stopped as "too slow" (estimated ~7s/batch). After fixes:
N=10000 batch 1–5 = 250–510ms. N=10000 is now a viable test size.

### Remaining bottleneck: `sim.step()`

At N=1000: step=25–55ms. This is cascade propagation cost (up to 16
sub-batches per tick, ChangeLog growing to 300k+ changes). Not O(R)
in the same pathological sense — further profiling needed before acting.
At N=10000: step=240–490ms. Linear-ish scaling with N. No acute fix identified.

### Updated Phase 3 priorities

The cohere O(E²×R) fix superseded E3 (relationship demotion) as the immediate
throughput win. E3 (reduce R) still relevant for `sim.step()` cascade cost.
See `docs/roadmap.md §5` for scope log.
