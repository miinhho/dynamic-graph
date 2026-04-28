# Coalgebra in the substrate

> Status: design memo, 2026-04-28. Companion to `docs/redesign.md`.

This document explains how the substrate's existing 5-layer ontology
maps onto the categorical notion of an **F-coalgebra**, what *new*
primitives the framing suggests, and which of those have actually been
implemented (vs. left as future framing).

The motivation is not theoretical purity — it's that several long-running
informal questions in the codebase ("when are two loci interchangeable?",
"what does Cohere mean precisely?", "what's the relationship between
declared schema and observed world?") have clean, well-studied answers
in coalgebraic terms. Where those answers buy us a runtime primitive
worth shipping, we ship it. Where they only sharpen vocabulary, we stop
at this document.

---

## 1. What is an F-coalgebra (60 seconds)

Given an endofunctor `F: C → C`, an **F-coalgebra** is a pair
`(X, α: X → F(X))`. The carrier `X` is a "state space"; `α` says what
is observable about a state in one step. The shape of `F` determines
what *kind* of system we have:

| Functor `F(X)` | System modeled |
|----------------|----------------|
| `A × X` | Streams over alphabet `A` |
| `X^A` | Deterministic automata, input `A` |
| `(B × X)^A` | Mealy machines, input `A`, output `B` |
| `1 + (A × X)` | Possibly-terminating streams |
| `P(L × X)` | Labeled transition systems |
| `D(X)` | Markov chains |

Two states `x, y : X` are **bisimilar** when there is a relation
`R ⊆ X × X` with `(x, y) ∈ R` and `α(x), α(y)` related by `F(R)`.
Bisimilarity is the largest such relation, and (under mild conditions
on `F`) it coincides with **trace equivalence** — two states are
bisimilar iff every observable behavior they could produce matches.
Bisimilarity is the canonical answer to "when are these two states
*the same* from the outside?".

The dual concept is the **algebra** `α: F(X) → X`, where `F` describes
*ways of constructing* `X`. Algebras and coalgebras meet in the
algebra/coalgebra duality: induction lives on the algebra side,
coinduction on the coalgebra side.

---

## 2. The substrate, layer by layer

### Layer 0: Locus dynamics — Mealy coalgebra

`LocusProgram::process(locus, incoming, ctx) -> Vec<ProposedChange>` has
the shape

```
α : State × Inbox × Context → Output × State′
```

which is a Mealy-machine coalgebra `α : X → (Output × X)^Inbox`. The
"successor state" `X′` is implicit: it is the state the engine
reconstructs by applying `Output` (and any other batch-loop effects) at
commit time. `Context` is the per-batch read view, which means programs
are **not autonomous coalgebras** — they observe the rest of the world
through a one-batch-old window.

Implication: the natural notion of equivalence on loci is *bisimulation
under the Mealy functor with the chosen Context window*. Two loci are
behaviorally equivalent if they emit the same outputs under every inbox
sequence, after weatherproofing for the perspective the user cares
about.

### Layer 1: Change — predecessor coalgebra

The `ChangeLog` carries the coalgebra

```
ChangeId  →  Change × P(ChangeId)
```

i.e. each change has a payload and a predecessor set. This is a
**Kripke frame** in coalgebraic logic terms — exactly the structure a
modal logic operates on. Existing operations are anamorphisms over this
frame:

- `causal_ancestors(id)` — BFS unfold of the predecessor closure.
- `is_ancestor_of(a, b)` — exists-path modality `<*>a` evaluated at `b`.
- `root_stimuli(id)` — the leaves reachable by following predecessors;
  formally an `unfold` that terminates at the trim boundary.

The categorical view here is descriptive only — nothing in the code
needs to change.

### Layer 2: Relationship — autonomous coalgebra

Decay (`flush_relationship_decay`) and Hebbian plasticity together act
as an autonomous coalgebra `α : State → State`, run once per batch.
Auto-emergence (`auto_emerge_relationship`) is a *bialgebraic* hook: it
is the algebra side (a constructor) wired to the coalgebra side (one
batch's worth of observed cross-locus changes).

### Layer 3 & 4: Entity and Cohere — bisimulation quotients

The substrate already says entities are "coherent bundles of
relationships" and coheres are "clusters under a perspective". In
coalgebraic terms, both are **bisimulation quotients** of the locus
coalgebra under a perspective-chosen functor `F'`:

- A `CoherePerspective` picks the granularity of "same".
- The Cohere it produces is the partition of loci/entities into
  `F'`-bisimulation classes.

Naming this connection has a payoff: a custom perspective that wants
*structural* sameness (rather than the activity-based clustering the
default uses) can now reach for `behavioral_partition` directly.

### graph-boundary — algebra/coalgebra duality

This is the cleanest alignment. The schema layer (`graph-schema`) is
the *initial-algebra* side: declarations build up structure with no
decay or observation-driven dynamics. The world layer (`graph-world`)
is the *final-coalgebra* side: observed behavior is recorded, decays,
is recoinduced from history. The four boundary quadrants are exactly
the corners of the algebra×coalgebra product:

```
                 │ in coalgebra (observed) │ not in coalgebra
─────────────────┼─────────────────────────┼───────────────────
in algebra       │ Confirmed               │ Ghost
(declared)       │                         │
not in algebra   │ Shadow                  │ Null (not reported)
```

`prescribe_updates` asks "given a tension at this corner, which side
should yield?" — i.e. should we extend the algebra (declare the shadow
fact) or weaken the coalgebra (let the ghost fact decay further)?
Framing the choice this way clarifies why Confirmed is stable: both
sides agree, no work to do.

---

## 3. What got shipped

### `graph-core::coalgebra`

The vocabulary module. Pure data — no `World` dependency.

- **`BehaviorColor`** (`u64`) — opaque hash-bucket identity used by the
  refinement procedure.
- **`LocusEncoder`, `EdgeEncoder`** — user-pluggable seed colorings.
  This is the "perspective" knob: it decides what counts as
  observably-the-same at depth zero.
- **Defaults**: `KindOnlyEncoder`, `KindAndQuantizedStateEncoder`,
  `KindOnlyEdgeEncoder`, `KindAndStrengthEdgeEncoder`.
- **`EdgeDirection`** — `Outgoing`/`Incoming`/`Symmetric`, folded into
  the per-neighbor signature so direction is not collapsed.
- **`fold_color`** — the canonical refinement combinator, exposed for
  callers who want to roll their own loop bit-identical to the default.

### `graph-query::coalgebra`

The runtime primitive: bounded bisimulation via Weisfeiler-Lehman
color refinement.

- **`behavioral_partition(world, opts) -> Vec<Vec<LocusId>>`** —
  partition every locus into `k`-bisimulation classes. Classes are
  sorted; output is deterministic.
- **`behavior_signature(world, locus, opts) -> Option<BehaviorColor>`**
  — single-locus stable label after `k` rounds. Useful as a hashable
  identity for caching / dedup work.
- **`behaviorally_equivalent(world, a, b, opts) -> bool`** — point
  query.
- **`BisimOptions`** — `rounds`, `locus_encoder`, `edge_encoder`.

The algorithm:

```text
color_0(v)     = encode_locus(v)
color_{k+1}(v) = fold(color_k(v),
                      sorted [
                        (encode_edge(e),
                         color_k(other(e, v)),
                         direction(e, v))
                        for e in edges_touching(v)
                      ])
```

Early termination: when the partition reaches a fixpoint (the map
`old_color → new_color` is functional in both directions), the loop
exits. For finite worlds this is guaranteed within `|V|` rounds and
usually much sooner.

Cost: `O(rounds × (|V| + |E| log Δ))`, where the `log Δ` is the
per-vertex sort of the neighborhood signature. No heap allocation in
the inner loop beyond growing the temporary neighborhood vector.

---

## 4. What did not get shipped (and why not)

- **HKT-style `Coalgebra<F>` trait.** Rust has no native HKTs;
  encoding them via GAT/associated types creates trait gymnastics
  without any concrete payoff in this codebase. The encoder traits we
  shipped cover the practical "perspective" knob.

- **Bialgebraic SOS / distributive laws (Turi-Plotkin).** Provides a
  framework for *proving* the batch loop is compositional. The proof
  obligation isn't standing in the way of any current work; we'd be
  paying abstraction cost for paperware.

- **Modal-logic evaluator over the change frame.** The `causality`
  module already has the queries we'd want (`is_ancestor_of`,
  `root_stimuli`, `causal_ancestors`); a modal-logic compiler would be
  surface area without callers.

- **Final-coalgebra runtime constructions.** Nice for theorem-proving;
  no operational role in a Rust engine.

The throughline: ship a primitive only when it answers a real
ambiguity in the codebase (here, "are these loci interchangeable?")
or makes existing structure legible (here, framing schema/world
duality).

---

## 5. Use cases for `behavioral_partition`

Concrete reasons the new primitive pulls weight:

1. **Entity merge candidate detection.** Two loci that `EmergencePerspective`
   recognizes as separate but that share a deep behavioral color are
   strong candidates for merge. The default emergence pipeline does not
   currently consult bisimulation; this is a future hook.

2. **Behavioral compression for snapshots.** `EntityWeatheringPolicy`
   layers compress entity history; the partition gives a principled
   "minimum-distinguishing-depth" knob — beyond that depth, two
   member loci can be represented by a single color in the layer.

3. **Regression analysis.** After a knob change in the engine
   (decay rate, plasticity, etc.) we ask: did the bisimulation
   classes change? If not, the change was observationally inert at
   the chosen depth. This is a stronger invariant than "tests pass".

4. **Custom CoherePerspective.** A perspective that wants structural
   coheres ("group loci that behave the same in the current topology")
   can call `behavioral_partition` directly with `rounds = 3` or so
   and emit the resulting classes as `CohereMembers::Entities`. The
   default activity-bridge perspective remains for activity-driven
   clustering.

---

## 6. Cross-reference

- Source: `crates/graph-core/src/coalgebra.rs`,
  `crates/graph-query/src/coalgebra.rs`.
- Tests: `tests` modules at the bottom of those files.
- Related design notes: `docs/redesign.md` (5-layer ontology),
  `docs/identity.md` (settled ontology), `docs/architecture.md` (the
  superseded framing).

For the categorical background:

- B. Jacobs, *Introduction to Coalgebra: Towards Mathematics of States
  and Observation*, CUP 2016. Chapters 1–3 cover the framing used here;
  chapter 6 covers bisimulation and the partition refinement
  algorithm.
- J. Rutten, *Universal coalgebra: a theory of systems*, TCS 249, 2000.
  The original survey for the general F-coalgebra framing.
