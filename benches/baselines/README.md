# Criterion Baselines — Phase 1 (E1) + B4 (query)

This directory stores committed criterion baseline snapshots that serve as the
regression guard for Phase 1 measurements. Baselines live here rather than
inside `target/criterion/` so they are tracked by git and survive `cargo clean`.

---

## Saving a baseline

Run the bench with `--save-baseline` to write the baseline into
`target/criterion/<bench-name>/<group>/base/`:

```sh
# Engine scaling curves
cargo criterion --bench engine -p graph-engine --save-baseline phase1

# Query scaling curves
cargo criterion --bench graph_query -p graph-query --save-baseline phase1
```

After the run, copy the baseline data into this directory:

```sh
cp -r target/criterion benches/baselines/criterion-phase1
git add benches/baselines/criterion-phase1
git commit -m "perf: commit Phase 1 criterion baselines"
```

---

## Comparing against a saved baseline

```sh
cargo criterion --bench engine -p graph-engine --baseline phase1
cargo criterion --bench graph_query -p graph-query --baseline phase1
```

For B4 query baselines specifically (stored under `query/`):

```sh
cargo bench -p graph-query --bench graph_query -- --baseline phase1 \
  "path_scaling|reach_active_scaling|causal_scaling|counterfactual_scaling"
```

Criterion prints a regression/improvement summary against the named baseline.
A regression is flagged when the new p-value falls outside the confidence
interval recorded in the baseline.

---

## Phase 1 Hypotheses

These hypotheses were written before profiling. Phase 3 priorities are
re-ordered based on which hypotheses the E1 measurements confirm or refute.

| # | Hypothesis | Potentially affects |
|---|---|---|
| H1 | `EmergencePerspective::recognize` (BFS over relationships) dominates on dense graphs | E4 |
| H2 | `ChangeLog::push` + predecessor tracking dominates on deep DAGs | E2 |
| H3 | Hebbian/STDP weight update iterates all relationships each batch (O(R)) | E3 |
| H4 | `PropertyStore` lookup via `LocusContext::properties` is on hot path | micro-opt |
| H5 | `Storage::commit_batch` fsync dominates when storage feature is on | storage tier |
| H6 | `StateVector` (Vec&lt;f32&gt;) allocation/clone in hot path | SmallVec transition |

---

## Workloads captured as E1 baselines

| Bench | Group | Sizes | File |
|---|---|---|---|
| `engine` | `ring_scaling` | N ∈ {16, 64, 256, 1024} | `crates/graph-engine/benches/engine.rs` |
| `engine` | `neural_scaling` | N ∈ {100, 500, 2000} | `crates/graph-engine/benches/engine.rs` |
| `graph_query` | `path_scaling` | N ∈ {64, 256, 1024} | `crates/graph-query/benches/graph_query.rs` |
| `graph_query` | `reach_active_scaling` | N ∈ {64, 256, 1024} | `crates/graph-query/benches/graph_query.rs` |
| `graph_query` | `causal_scaling` | depth ∈ {10, 100, 500} | `crates/graph-query/benches/graph_query.rs` |

---

## Workloads captured as B4 baselines (`query/`)

| Bench | Group | Sizes | File |
|---|---|---|---|
| `graph_query` | `path_scaling` | N ∈ {64, 256, 1024} | `crates/graph-query/benches/graph_query.rs` |
| `graph_query` | `reach_active_scaling` | N ∈ {64, 256, 1024} | `crates/graph-query/benches/graph_query.rs` |
| `graph_query` | `causal_scaling` | depth ∈ {10, 100, 500} | `crates/graph-query/benches/graph_query.rs` |
| `graph_query` | `counterfactual_scaling` | depth ∈ {10, 100, 500} | `crates/graph-query/benches/graph_query.rs` |

---

## Notes

- `target/criterion/` is in `.gitignore` and is never committed directly.
- Baselines under `benches/baselines/criterion-phase1/` are committed as a
  point-in-time snapshot after the Phase 1 measurement runs complete.
- B4 baselines are stored under `benches/baselines/query/` (committed 2026-04-17).
