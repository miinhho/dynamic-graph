# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Commands

```bash
# Build all crates
cargo build

# Run all tests
cargo test

# Run tests for a single crate
cargo test -p graph-engine

# Run a single test by name
cargo test -p graph-engine trim_before_batch

# Check without building
cargo check

# Lint
cargo clippy
```

---

## Architecture

The redesign is documented in `docs/redesign.md`, which supersedes `docs/architecture.md`. The authoritative framing is the **5-layer emergent ontology**:

```
Layer 4: Cohere       — clusters under a user-supplied perspective       (derived)
Layer 3: Entity       — coherent bundles of relationships                (derived)
Layer 2: Relationship — observed locus-to-locus coupling                 (derived)
Layer 1: Change       — atomic event with causal predecessors            (primitive)
Layer 0: Locus        — labeled position with state and LocusProgram     (primitive)
```

Entities and relationships are **emergent, not declared**. The user registers loci and programs; the engine produces the rest.

### Crate responsibilities

| Crate | Role |
|-------|------|
| `graph-core` | Pure data types and traits: `Locus`, `Change`, `Relationship`, `Entity`, `Cohere`, `StateVector`, `LocusProgram`, `StructuralProposal`, weathering, stabilization |
| `graph-world` | In-memory stores: `ChangeLog`, `LocusStore`, `RelationshipStore`, `EntityStore`, `CohereStore`, `World` facade |
| `graph-engine` | Batch loop, kind registries (`LocusKindRegistry`, `InfluenceKindRegistry`), regime classifier, emergence/cohere perspectives, adaptive guard rail |
| `graph-tx` | *(removed)* — `ChangeLog` in graph-world covers this role |
| `graph-testkit` | Test programs, canonical world fixtures, assertions, deterministic LCG generators |

### Key types

- **`StateVector`** — heap `Vec<f32>` representing a locus or relationship's state. Relationships use a 2-slot vector: `[activity, weight]`.
- **`Change`** — committed event: `{ id, subject: ChangeSubject, kind: InfluenceKindId, predecessors, before, after, batch }`. `ChangeSubject` is `Locus(LocusId)` or `Relationship(RelationshipId)`.
- **`LocusProgram`** — user-supplied trait: `process(locus, incoming, ctx)` returns `Vec<ProposedChange>`; `structural_proposals(locus, incoming, ctx)` returns `Vec<StructuralProposal>` (default: empty). Both receive a `&dyn LocusContext` for querying neighbor states and relationships.
- **`LocusContext`** — read-only world view: `locus(id)`, `relationships_for(id)`, `relationship_between(a, b)`. Concrete impl: `BatchContext` in graph-world.
- **`InfluenceKindConfig`** — per-kind config: `decay_per_batch`, `StabilizationConfig`, `PlasticityConfig` (Hebbian opt-in, `learning_rate = 0` by default).
- **`Engine::tick()`** — the main entry point. Drains pending changes in batches until quiescent or `max_batches_per_tick` fires.

### Batch loop (engine.rs)

1. Commit all pending `ProposedChange`s as the current `BatchId`.
2. For each `ChangeSubject::Locus`: apply stabilization, update locus state, auto-emerge relationships for cross-locus predecessors, collect Hebbian observations.
3. Dispatch `LocusProgram::process(locus, inbox, ctx)` for each affected locus; queue follow-up changes.
4. Collect `structural_proposals(locus, inbox, ctx)`; apply `CreateRelationship` / `DeleteRelationship`.
5. Apply Hebbian weight updates (`Δweight = η × pre × post`).
6. Apply per-kind activity decay and weight decay to all relationships.
7. Advance `BatchId`; repeat until `pending` is empty.

### Entity weathering

`EntityWeatheringPolicy` controls how entity sediment layers erode. The default (`DefaultEntityWeathering`) has three windows: Preserved (< 50 batches), Compress (50–200), Skeleton (200–1000), Remove (≥ 1000). The engine never removes a layer whose transition `is_significant()` (Born/Split/Merged) — it falls back to Skeleton.

### ChangeLog query surface

`ChangeLog` (and `World` wrappers) provide O(1) or O(k) queries:

| Method | Complexity | Notes |
|--------|-----------|-------|
| `get(id)` | O(1) | Uses ChangeId density invariant: `index = id − offset` |
| `batch(batch_id)` | O(k) | `by_batch` reverse index |
| `changes_to_locus(id)` | O(k) | `by_locus` reverse index, newest first |
| `changes_to_relationship(id)` | O(k) | `by_relationship` reverse index, newest first |
| `predecessors(id)` | O(k) | Direct predecessor `ChangeId`s in the Change |
| `causal_ancestors(id)` | O(ancestors) | BFS, deduped via `HashSet`, stops at trimmed entries |
| `is_ancestor_of(a, b)` | O(ancestors) | DFS with ID-based pruning (`pid >= ancestor.id`) |

The `World` type exposes all of these as convenience wrappers.

### Design invariants

- `ChangeLog` is append-only; trimming via `trim_before_batch` requires no live predecessor references point into the trimmed range.
- **ChangeId density**: IDs are assigned as a dense monotone sequence starting at 0. After `trim_before_batch`, the offset shifts but density is preserved. `get()` relies on this — never insert a change with a non-sequential ID.
- Predecessor auto-derivation (O1): internal changes inherit the `ChangeId`s of changes that fired into their subject locus during the same batch.
- Debug-only panics (O6): `require()` on both registries panics in debug builds for unregistered kinds; returns `None` in release.
- `PlasticityConfig::is_active()` is `pub(crate)` — callers outside engine should not gate on it.
