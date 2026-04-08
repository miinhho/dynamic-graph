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
| Relationship subject in changes | Deferred — `ChangeSubject` is currently `Locus(LocusId)` only |
| Structural mutation (topology change) | Between ticks only, for now |

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
Relationship, Entity, Cohere) are implemented across the five crates
(`graph-core`, `graph-world`, `graph-engine`, `graph-tx`, `graph-testkit`).

### Near-term

- **Weathering** — entity layer compression and change log trimming.
  `EntityLayer` already has `CompressionLevel`; the engine needs a
  `WeatheringPolicy` trait and a default implementation.
- **Relationship subjects in changes** — currently `ChangeSubject` is
  `Locus(LocusId)` only. Lifting this restriction enables richer
  higher-layer programs.

### Medium-term

- **Edge plasticity** — relationships adapt their parameters (weight,
  attenuation) based on signal flow. Hebbian-style learning as the first
  kernel. The stabilization guard rail is reused on the relationship
  parameters themselves.
- **Structural mutation** — relationship creation/deletion becomes part of
  tick output. Programs can issue topology-change proposals alongside state
  changes.

### Longer-term

- **Causal lineage queries** — a query API over the change DAG once the log
  format and topology mutation semantics are stable.

## 9. What this document does *not* do

- It does not list every architectural decision. `architecture.md` is the
  design playground for ideas not yet committed.
- It does not specify performance targets. Performance work is driven by
  measurement on real workloads.
- It does not commit to a specific edge plasticity model. The plasticity
  layer will pick one and document it here when it lands.
