# Project Identity

This document is the source of truth for *what this project is* and *what
stability means in it*. When `architecture.md` and this file disagree, this
file wins. `architecture.md` is the design playground; this file is the
contract.

`redesign.md` extended this document with a new ontology. The decisions there
are now folded in below. Where the two previously disagreed, `redesign.md`
won; this file now reflects the settled resolution.

## 1. What this is

A graph dynamics engine where:

- **loci** hold state — discrete positions in the substrate where programs run
- **changes** are the atomic events — first-class objects with causal predecessors
- **relationships** emerge automatically from cross-locus change-flow
- **entities** are recognized as coherent patterns of relationships — they are
  never declared upfront, always derived
- both the topology of relationships and the identity of entities evolve as
  part of normal operation
- the value of the system is in observing **how things change and why**, not
  in the values themselves at any single batch

The unit of interest is the *transformation* — input received, kind of
influence, causal chain, resulting change — not the post-transformation value
alone. Causal chains between transformations are reconstructable by walking
the change DAG backwards from any change of interest.

## 2. What this is not

- **Not a graph database.** No query language, no persistence as a primary
  goal, no transactional CRUD surface for end users.
- **Not an equilibrium solver.** The engine is not trying to drive the system
  toward a fixed point. A system that reaches a fixed point has simply moved
  into a "no current change" regime; that is information, not victory.
- **Not a simulation framework for known dynamical models.** Kuramoto,
  consensus, opinion dynamics etc. are valid *workloads to run on top* but
  none of them are the project. The project is the substrate.
- **Not a place to encode product semantics.** Domain laws live in the
  programs and influence kind configs supplied by callers, not in the engine.
- **Not an entity–relationship database.** Entities are not registered. They
  are recognized from relationship patterns by user-supplied perspectives.

## 3. Stability is a guard rail, not a goal

This is the most important framing in this document.

Past iterations of this project (and much of `architecture.md`'s §7) treat
stability as a first-class requirement, listing damping, leak, trust region,
SCC scheduling, oscillation detection, divergence detection as if they were
the point of the engine. They are not. They exist for one reason:

> Without them, external inputs can drive the system into runaway suppression
> (over-damping → nothing observable happens) or runaway growth (numerical
> blow-up → nothing observable happens). Both failure modes destroy the
> engine's actual purpose: making the dynamics observable.

So the stability layer is a **guard rail**. Its job is to keep the system
inside the range where its dynamics can be observed. It is not trying to
make the system *boring*.

Concretely:

- `StabilizationConfig` (alpha blending, saturation, trust region) is the
  guard rail. It is applied per-kind at commit time by the engine.
- `AdaptiveGuardRail` raises the guard rail when external shocks are pushing
  the system toward divergence and lowers it when nothing is happening. It
  does **not** treat oscillation or limit cycles as problems to suppress.
- The regime classifier sorts the current behaviour into observation regimes,
  of which only one — `Diverging` — calls for the guard rail to push back.
- The `AdaptiveGuardRail` is *not* integrated into the batch loop
  automatically. The caller observes the regime, feeds it to the guard rail,
  and reads back the effective alpha. This is deliberate: callers choose when
  adaptation matters.

## 4. Regime classification

`DynamicsRegime` is a classification of the *current observation regime*, not
a verdict of success or failure:

| Regime | Meaning | Guard rail action |
|---|---|---|
| `Initializing` | Not enough history to classify yet | none |
| `Settling` | Per-batch deltas are decreasing — system is in a transient | none |
| `Quiescent` | Per-batch deltas are at the noise floor — no observable change | relax (allow guard rail to weaken) |
| `Oscillating` | Bounded sign-flipping behaviour | none — valid regime |
| `LimitCycleSuspect` | Recent samples show a repeated pattern | none — valid regime |
| `Diverging` | Energy or per-batch delta is growing past the configured ratio | tighten (shrink alpha) |

`Quiescent` and `Diverging` are the only two regimes the guard rail acts on.
Everything else is a regime to *observe*, not a regime to *correct*.

## 5. The ontology (post-redesign)

The substrate is layered. Each layer derives from the layer below by pattern
recognition. Each layer is observable.

```
Layer 4: Cohere       — clusters of relationships/entities under a perspective
            ▲                                       (output, derived, ephemeral)
Layer 3: Entity       — coherent bundles of relationships (sedimentary)
            ▲                                       (output, derived)
Layer 2: Relationship — observed coupling between loci, derived from change-flow
            ▲                                       (output, derived, entity-like)
Layer 1: Change       — atomic event with causal predecessors
            ▲                                       (primitive, recorded)
Layer 0: Locus        — labeled position with state and program
                                                    (primitive, user-registered)
```

### Locus (Layer 0)

The substrate primitive. Users register loci with an id, a kind tag, an
initial `StateVector`, and a `LocusProgram`. A locus is not the same as the
user's mental "entity" — it is just a *position* where state lives and
changes can arrive.

### Change (Layer 1)

The atomic event. Everything that happens is a change. Changes are
first-class — recorded, queried, and the basis of all higher layers.

Each change carries: subject locus, influence kind, causal predecessors
(other `ChangeId`s), before/after `StateVector`, and batch index. The set of
all changes forms a DAG. A *batch* is a maximal antichain — changes that are
causally independent and processed together by the batch loop.

### Relationship (Layer 2)

Automatically detected when cross-locus change-flow is observed. When change
A at locus L₁ has change B at locus L₂ in its predecessors, the engine
recognizes a relationship of the relevant kind from L₂ to L₁. Relationships
have their own ID, state (including an activity score that decays per batch),
and lineage.

### Entity (Layer 3)

Recognized — never declared. Users supply an `EmergencePerspective`; the
engine runs it on-demand and reconciles proposals against the existing entity
store. Entities are **sedimentary**: each significant event deposits a new
layer on top of the existing stack rather than replacing it. Entities are
never deleted — they may become dormant, but remain in the store.

### Cohere (Layer 4)

Clusters of relationships and/or entities under a user-supplied
`CoherePerspective`. Multiple perspectives can be active simultaneously.
Cohere sets are ephemeral (recomputed on demand); they are not sedimentary.

## 6. Key resolved design decisions

These supersede the old §6 ("Resolved questions from architecture.md §17").

| Decision | Resolution |
|---|---|
| State representation | `StateVector` (Vec<f32>) — generic over component count, programs interpret semantics |
| Time model | Logical batches (causal partial order), no wall clock |
| Entity registration | None — entities are recognized by `EmergencePerspective`, not declared |
| Influence kinds | Per-kind `InfluenceKindId` with separate decay, stabilization config, and regime tracking |
| Relationship kind | `RelationshipKindId = InfluenceKindId` (same dimension, resolved O8) |
| Cross-platform determinism | Bit-identical on the same platform/toolchain; cross-platform not promised |
| Relationship subject in changes | **Done** — `ChangeSubject::Relationship(RelationshipId)` |
| Structural mutation (topology change) | **Done** — `StructuralProposal` (CreateRelationship / DeleteRelationship) emitted per batch |

## 7. Phase 1+2 retrospective

| What we built | Status | Notes |
|---|---|---|
| `BasicStabilizer` (alpha blend, saturation, trust region) | **Ported and active** as `StabilizationConfig`; applied per-kind at commit time |
| Regime classifier | **Ported and renamed**: `RuntimeStatus` → `DynamicsRegime`, `Converging` → `Settling`, `Converged` → `Quiescent` |
| `AdaptiveStabilizer` | **Ported and corrected** as `AdaptiveGuardRail`: no longer shrinks on Oscillating/LimitCycleSuspect — those are valid regimes; only Diverging shrinks |
| SCC primitive | **Parked** — becomes relevant when structural mutation lands |
| Channel / Entity as primitives | **Replaced**: channels → Relationships (emergent), entities → Entities (emergent); old primitive-first model discarded |
| `TickId` wall clock | **Replaced**: `BatchId` (logical batch index, causal partial order) |

## 8. Roadmap (post-redesign)

The substrate redesign is **complete**. All five layers (Locus, Change,
Relationship, Entity, Cohere) are implemented across four crates
(`graph-core`, `graph-world`, `graph-engine`, `graph-testkit`).

### Completed

- **Weathering** ✓ — `EntityWeatheringPolicy` trait + `DefaultEntityWeathering`;
  entity layer compression (Full → Compressed → Skeleton) and change log
  trimming via `ChangeLog::trim_before_batch`.
- **Relationship subjects in changes** ✓ — `ChangeSubject::Relationship(RelationshipId)`.
- **Edge plasticity** ✓ — `PlasticityConfig` (Hebbian: `Δweight = η × pre × post`)
  on `InfluenceKindConfig`; opt-in per influence kind.
- **Structural mutation** ✓ — `StructuralProposal` (CreateRelationship /
  DeleteRelationship) collected from `LocusProgram::structural_proposals` and
  applied at end-of-batch.
- **Causal lineage queries** ✓ — full query surface on `ChangeLog` and `World`:
  `predecessors`, `causal_ancestors` (BFS), `is_ancestor_of` (DFS with
  ID-based pruning); reverse indices (`by_locus`, `by_relationship`,
  `by_batch`) make subject/batch queries O(k); `get()` is O(1) via ChangeId
  density invariant.

- **WAL persistence** ✓ — `graph-wal` crate: append-only segment files,
  periodic checkpoints, two-phase recovery, CRC-32 torn-record detection,
  atomic checkpoint writes, WAL compaction after `trim_before_batch`.
- **WAL–Simulation integration** ✓ — `SimulationConfig::wal: Option<WalConfig>`
  (requires `graph-engine`'s `wal` feature). Auto-writes every committed batch
  after each `step()`; exposes `flush_wal()`, `compact_wal()`,
  `last_wal_error()`, and `Simulation::from_recovery()`.

- **Induced subgraph + activity filter** ✓ —
  `World::induced_subgraph(loci)` (relationships fully contained within a
  locus set; O(Σk_i)); `World::relationships_active_above(threshold)` (live
  relationship filter); `Endpoints::all_endpoints_in`; `WorldMetrics::
  active_relationship_count` (relationships above `ACTIVITY_THRESHOLD = 0.1`).

- **`WorldDiff`** ✓ — batch-range diff: `World::diff_since(from)` and
  `diff_between(from, to)` return `WorldDiff` with `change_ids`,
  `relationships_created`, `relationships_updated`, `entities_changed`.
  O(k + R + E×L_avg) where k = changes in range.

- **Connected components + kind-filtered traversal** ✓ —
  `World::connected_components()` / `connected_components_of_kind()` (BFS,
  O(V+E)); `path_between_of_kind`, `reachable_from_of_kind`; `WorldMetrics`
  extended with `component_count` and `largest_component_size`.

- **`WorldMetrics` + degree centrality** ✓ — `World::metrics()` snapshot
  (counts, activity stats, top-N by degree/activity); O(1) `degree()`,
  `in_degree()`, `out_degree()` via `by_locus` index; `Simulation::step_n(n, stimuli)`
  and `step_until(pred, max, stimuli)` multi-step convenience methods.

- **Graph traversal** ✓ — `World::path_between(a, b)` (BFS shortest path through
  relationship graph), `World::reachable_from(start, depth)` (all loci within N hops);
  `World::entity_members`, `World::entity_member_relationships` (current snapshot
  lookups from `EntitySnapshot`).

- **Relationship query surface** ✓ — `by_locus` reverse index on `RelationshipStore`;
  four indexed queries: `relationships_for_locus`, `relationships_from`,
  `relationships_to`, `relationships_between`; all exposed on `World`.
  `relationships_for_locus_of_kind` for kind-filtered traversal.

- **Criterion benchmarks** ✓ — `benches/engine.rs`: single-tick topology
  benchmarks, steady-state `Simulation::step` cost, causal-ancestor DAG depth
  scaling, changelog query comparison. `benches/wal.rs` (requires `wal` feature):
  per-step WAL overhead (no-WAL / sync / async), checkpoint write+load roundtrip,
  full recovery cost, compaction cost.

- **Cross-crate integration tests** ✓ — `crates/graph-engine/tests/integration.rs`:
  16 tests covering the full simulation stack: relationship emergence (chain,
  star, ring), `WorldDiff` change capture / created-vs-updated classification /
  quiescent empty diff, graph traversal post-emergence (`path_between`,
  `reachable_from`, `connected_components`), `step_until` convergence / stimuli
  applied once / max-steps exhaustion, invariant assertions (bounded activity,
  DAG structure, no batch-cap hits), and WAL recovery (relationship count and
  `BatchId` round-trip via `#[cfg(feature = "wal")]`).

- **`WorldDiff` semantics documented** ✓ — module-level doc and
  `relationships_updated` field doc now explicitly state: Hebbian weight
  updates are captured (always co-occur with auto-emerge); lazy activity decay
  is not (correct by design — decay is background, not an event).

- **`step_until` clone eliminated** ✓ — replaced `bool first` guard +
  `stimuli.clone()` with `Option<Vec<ProposedChange>>` + `take()`, so
  the stimulus vector is moved on the first iteration and no copy is made.

### Open

No open roadmap items at this time. The substrate is feature-complete across
all five layers. Future work is driven by measurement on real workloads.

## 9. What this document does *not* do

- It does not list every architectural decision. `architecture.md` is the
  design playground for ideas not yet committed.
- It does not specify performance targets. Performance work is driven by
  measurement on real workloads.
- It does not commit to a specific edge plasticity model. The plasticity
  layer will pick one and document it here when it lands.
