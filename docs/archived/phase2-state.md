# Current Engine State (Archived — Phase 1+2)

> **This document is archived.** It describes the Phase 1+2 engine that was
> replaced by the substrate redesign. See `docs/identity.md` and
> `docs/redesign.md` for the current architecture.

---

## 1. Scope

This repository currently implements a generic in-process runtime engine, not a product.

The engine provides:

- stateful `Entity` execution
- routed `Channel` propagation
- bounded per-tick execution
- typed Rust client access
- snapshot-based inspection
- deterministic commit boundaries

The engine does not currently provide:

- a query language
- product-specific event semantics
- domain ontology semantics
- network server hosting
- distributed execution

## 2. Current Architecture

### 2.1 Crate Boundaries

- `graph-core`
  - core ids
  - value vectors
  - entities
  - channels
  - laws
  - stimuli
- `graph-world`
  - live world state
  - world snapshots
  - selector/index resolution
  - read/query surface
- `graph-engine`
  - tick execution
  - routing and dispatch
  - stabilization
  - coordinator and retry
  - client facade
  - inspection
- `graph-tx`
  - tick transactions
  - deltas
  - provenance and replay-oriented types
- `graph-testkit`
  - fixtures
  - benchmark worlds

### 2.2 Core Runtime Flow

Current tick flow:

1. prepare active source entities
2. route outbound channels
3. dispatch direct or cohort emissions
4. aggregate inbound signals
5. compute next entity state
6. stage updates
7. validate and commit
8. expose snapshot and tick inspection

This is still a deterministic single-commit engine:

- compute may run in parallel
- commit remains validated and singular
- read consistency is snapshot-based

## 3. Public Surface

### 3.1 Standard Client

The standard access layer is `RuntimeClient`.

It is the intended Rust API surface for:

- ticking the engine
- reading current world state
- inspecting recent tick results

This is intentionally a typed Rust API, not a textual query layer.

### 3.2 Read Layer

Read access is built on:

- `WorldSnapshot`
- `SnapshotQuery`
- `EntityQuery`
- `ChannelQuery`
- `TickInspection`

This layer is meant for engine inspection and integration.
It is not meant to encode domain semantics.

## 4. Design Decisions That Survived

These changes remained after benchmark and design review because they improve the engine structurally rather than only helping a narrow benchmark case.

### 4.1 `SelectorCache` as Internal Routing Cache

`SelectorCache` remains an internal routing-only cache.

Current decision:

- use a plain `FxHashMap<ChannelId, ResolvedSelection>`
- do not expose cache semantics outside engine routing

Reason:

- simpler than the hybrid experiment
- more stable on representative workloads
- avoids overfitting to tiny synthetic worlds

### 4.2 `Flat field` Fast Path

`FieldKernel::Flat` is effectively treated like non-field direct delivery once selection is already resolved.

Reason:

- selector resolution already enforces radius membership
- the dispatch path does not need to recompute a trivial scale

This is a structural simplification, not a benchmark-only trick.

### 4.3 `FieldEvaluator`

Non-flat field propagation now precomputes a channel-level evaluator.

Reason:

- removes repeated kernel branching inside target loops
- moves field semantics toward a normalized internal form

### 4.4 One-Dimensional Distance Fast Path

`StateVector::distance()` has a fast path for one-dimensional vectors.

Reason:

- current position representation is generic
- most benchmark and fixture worlds use one-dimensional positions
- this keeps the generic type while reducing obvious overhead

## 5. Optimization Work That Was Rejected

These experiments were tried and then rejected because they increased complexity without stable improvement.

### 5.1 Selector Distance Caching

We tried carrying per-target distances from selector resolution into field dispatch.

Result:

- plausible in theory
- unstable or regressive on mixed workloads
- increased complexity across `graph-world` and `graph-engine`

Decision:

- removed

### 5.2 Hybrid `SelectorCache`

We tried:

- `SmallVec` for small channel counts
- `FxHashMap` promotion for larger channel counts

Result:

- strong wins on tiny synthetic workloads
- regressions on representative larger workloads

Decision:

- removed
- plain `FxHashMap` kept

## 6. Benchmark Interpretation

## 6.1 What the Internal Benchmarks Show

Current internal runtime benchmarks consistently show:

- `broadcast` dispatch is cheap
- `pairwise` dispatch is moderate
- `field` dispatch remains the most expensive direct path

Representative current shape:

- `dispatch_source_broadcast`: very small
- `dispatch_source_pairwise_only`: smaller
- `dispatch_source_field_only`: larger
- `dispatch_source_pairwise_and_field`: dominated by field work

This means the runtime bottleneck is still the field path, not state update or cohort aggregation.

### 6.2 What the Representative Benchmarks Show

Representative workloads matter more than tiny benchmark worlds.

Current benchmark policy:

- keep improvements that stay neutral or positive on representative worlds
- reject changes that only help tiny synthetic cases

This is why:

- plain `FxHashMap` was preferred over hybrid selector cache
- field specialization was kept
- selector-distance caching was removed

## 7. Current Risks

### 7.1 Position Representation

`position` still uses `StateVector`.

This is flexible, but likely not optimal for long-term field-heavy workloads.

Future option:

- introduce a dedicated position type or normalized spatial representation

### 7.2 Channel Internal Representation

`Channel` still carries all delivery modes in one shared structure.

This is acceptable for the MVP, but it means execution still branches over:

- pairwise
- broadcast
- field
- cohort

Future option:

- normalize channels internally into execution-specific forms

### 7.3 Query Reverse Paths

Read/query support is sufficient for inspection, but reverse query cost can still grow with dynamic channels.

This is acceptable for the current engine stage, but it should continue to be benchmarked against larger worlds.

## 8. Recommended Next Steps

### 8.1 Preserve Current Boundary Discipline

Do not add product semantics into the engine core.

Keep:

- engine primitives
- snapshot reads
- typed Rust client access
- generic transaction and inspection

Do not add:

- domain event semantics
- product-specific timelines
- domain query language

### 8.2 Prefer Representative Benchmarks Over Micro-Optimizing

Further optimization should be accepted only if it survives:

- small synthetic benchmarks
- representative 64/256 runtime worlds
- representative query worlds

### 8.3 Next Structural Candidate

The next meaningful structural change is not another small optimization.

It is one of:

- internal `NormalizedChannel` execution form
- dedicated position/spatial representation
- larger-scale selector/index backends

Those are design-level changes, not micro-tuning.

## 9. Current Summary

The engine is currently in a good MVP state:

- boundaries are clear
- the client API is stable enough for internal use
- read and inspection layers are present
- the worst obvious runtime bottlenecks have been reduced
- benchmark-driven rollback discipline has been applied

The remaining work is no longer “make every small benchmark smaller”.
It is “decide which internal representations should become first-class as scale increases”.
