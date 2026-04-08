# Project Identity

This document is the source of truth for *what this project is* and *what
stability means in it*. When `architecture.md` and this file disagree, this
file wins. `architecture.md` is the design playground; this file is the
contract.

## 1. What this is

A graph dynamics engine where:

- nodes hold state
- edges hold interaction laws that transform state
- both nodes **and** edges evolve over time
- topology itself can change as part of normal operation
- the value of the system is in observing **how things change**, not in the
  values themselves at any single tick

The unit of interest is the *transformation* — input received, transformation
applied, resulting change — not the post-transformation value alone. Causal
chains between transformations should be reconstructable from the engine's
output (this falls out of recorded transactions plus history; it does not
need to be a first-class object in the type system).

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
  programs and laws supplied by callers, not in the engine.

## 3. Stability is a guard rail, not a goal

This is the most important reframing in this document.

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

- `BasicStabilizer` (alpha blending, decay, saturation, trust region) is the
  guard rail. It survives unchanged from phase 1.
- `AdaptiveStabilizer` raises the guard rail when external shocks are pushing
  the system toward divergence and lowers it when nothing is happening. It
  does **not** treat oscillation or limit cycles as problems to suppress.
- The regime classifier (formerly the "convergence classifier") sorts the
  current behaviour into observation regimes, of which only one — `Diverging`
  — calls for the guard rail to push back.

## 4. Regime classification

`DynamicsRegime` (formerly `RuntimeStatus`) is a classification of the
*current observation regime*, not a verdict of success or failure:

| Regime | Meaning | Guard rail action |
|---|---|---|
| `Initializing` | Not enough history to classify yet | none |
| `Settling` | Per-tick deltas are decreasing — system is in a transient | none |
| `Quiescent` | Per-tick deltas are at the noise floor — currently no observable change | relax (allow guard rail to weaken) |
| `Oscillating` | Bounded sign-flipping behaviour | none — this is a valid regime |
| `LimitCycleSuspect` | Recent samples show a repeated pattern | none — this is a valid regime |
| `Diverging` | Energy or per-tick delta is growing past the configured ratio | tighten (shrink alpha) |

`Quiescent` and `Diverging` are the only two regimes the guard rail acts on.
Everything else is a regime to *observe*, not a regime to *correct*.

## 5. Phase 1+2 retrospective under this framing

| What we built | Survives? | Notes |
|---|---|---|
| `BasicStabilizer` (saturation, trust region) | yes | This is the guard rail. Exact match for the new framing. |
| Regime classifier (was "convergence classifier") | yes, renamed | `RuntimeStatus` → `DynamicsRegime`, `Converging` → `Settling`, `Converged` → `Quiescent`, etc. The doc comments and the meaning of variants changed; the structure did not. |
| `AdaptiveStabilizer` | yes, behaviour adjusted | Old behaviour shrunk on `Oscillating`/`LimitCycleSuspect`. New behaviour leaves both alone — they are valid regimes. Only `Diverging` triggers shrink; `Quiescent` triggers recovery. |
| SCC primitive | yes | When topology becomes mutable (Layer C) this becomes more important, not less, because the SCC plan will need to be recomputed per tick. |
| Scheduled iterative driver | parked | Its purpose was to "iterate cyclic blocks toward convergence". Under the new framing, convergence is not a goal, so iterative settling is not generally desirable. We keep the code for now in case a workload wants it, but it is not on the critical path. |

## 6. Resolved questions from architecture.md §17

These are the answers committed by this document. `architecture.md` §17
should be considered superseded by these.

| Question | Answer |
|---|---|
| Minimal state representation: scalar/vector/enum/hybrid? | **Vector (`StateVector`)**, generic over component count. The engine does not branch on state shape; programs do. |
| Are laws static per edge type or customizable per edge instance? | **Per edge instance.** Each `Channel` carries its own `LawId` and parameters. Laws themselves are static functions in the `LawCatalog`, but their parameterisation is per-channel. |
| Can topology change during a tick, or only between ticks? | **Between ticks only**, for now. Layer C will introduce structural mutation as a tick-level operation that takes effect at the next tick boundary, never mid-tick. |
| How much determinism across platforms? | **Bit-identical replay on the same platform/toolchain.** Cross-platform determinism is not promised and is explicitly out of scope. |
| Should delays be part of the MVP? | **No.** Out of scope for now. The current `cooldown` field is the only time-distance concept and that suffices. |
| Synchronous only or selective async? | **Synchronous tick boundaries**, with parallel compute inside a tick. No async propagation. |

## 7. Roadmap (post-phase-2)

Layers in order, with rationale:

### Layer A — Identity alignment (this document + rename)
*In progress.* Code change is small. Purpose is to align the mental model
before any new work, so subsequent layers do not inherit the old "stability
is the goal" framing.

### Layer B — Edge plasticity (smallest form)
Channels can adapt their parameters (weight, attenuation) based on the signal
that has flowed through them. Hebbian-style is the easiest first kernel. The
existing stabilization layer is reused as the guard rail on the channel
parameters themselves: max weight, decay, trust region per tick. Topology is
**not** touched in this layer — only edge parameters evolve.

### Layer D — Causal logging
`TickTransaction` history is retained in `graph-tx`'s WAL (which finally has
a real purpose). The user can reconstruct causal chains by walking the log
backwards from a delta of interest. This is **logging**, not a query API:
the log format is structured enough to enable post-hoc analysis, but the
engine does not provide a `LineageQuery` interface in this layer.

This is intentional: building the query API before topology is mutable would
constrain it in ways we would later regret.

### Layer C — Structural mutation as a first-class operation
The biggest layer. Channel creation/deletion/rewiring becomes part of normal
tick output, recorded in `TickTransaction` alongside state deltas. Programs
can issue mutation commands the same way they currently emit signals. SCC
plans must be recomputed each tick, which is the performance concern noted
during the phase-2 design discussion. Layer A→B→D must be in place first
because (a) the regime framing must be settled before adapting topology, (b)
the plasticity guard rails will be reused as topology guard rails, and (c)
the causal log format must already handle "the channel that produced this
delta no longer exists at the current tick".

## 8. What this document does *not* do

- It does not list every architectural decision. `architecture.md` is still
  the design playground for ideas not yet committed.
- It does not specify performance targets. Performance work is driven by
  measurement on real workloads, which we do not yet have.
- It does not commit to a specific edge plasticity model. Layer B will pick
  one and document it; this file just commits to the existence of the layer.
