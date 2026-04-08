# Dynamic Graph State Engine Architecture

> **Note**: This document is the design playground. For the project's actual
> identity and the framing under which "stability" should be read, see
> [`identity.md`](./identity.md). When the two documents disagree, identity.md
> wins. In particular: §7 of this document treats stability as a first-class
> *requirement*; identity.md restates it as a *guard rail*. They are not
> describing different code, only different framings — but the framing in
> identity.md is the one this project commits to.

## 1. Goal

This project is not a conventional graph database.

The target system is an in-memory `interaction-driven graph state engine` where:

- each node is a stateful entity
- each edge represents an interaction law, not just a relation
- state changes propagate through the graph like influence in a dynamical system
- the engine must remain stable under cyclic, nonlinear, and heterogeneous interactions

The design is inspired by:

- `IndraDB` for a clean embedded graph core and typed graph model
- `Memgraph` for in-memory execution, transaction thinking, and delta-based change tracking
- graph dynamical systems, contraction analysis, consensus dynamics, and topological diagnostics for stabilization

## 2. Product Identity

### 2.1 What This Is

- in-memory graph state engine
- interaction-centric runtime
- deterministic simulation and propagation core
- graph-shaped state machine for dynamic systems

### 2.2 What This Is Not

- not primarily a property graph query engine
- not Cypher-first
- not a distributed graph database in the MVP
- not a reactive property binding framework

## 3. Core Model

The engine treats graph execution as a discrete-time dynamical system over a mutable graph.

At tick `t`:

`X(t + 1) = Step(X(t), G(t), U(t))`

Where:

- `X(t)` is the full graph state
- `G(t)` is the graph topology and interaction definitions
- `U(t)` is external input, commands, or disturbances

The central idea is:

- nodes contain internal state
- edges define how source state influences target state
- state updates happen through bounded, stabilizable propagation

## 4. Design Principles

### 4.1 Separate Storage from Execution

Storage concerns:

- node and edge identity
- indexing
- snapshots
- read inspection and selection
- persistence and replay

Execution concerns:

- interaction evaluation
- propagation scheduling
- damping and stabilization
- convergence detection

### 4.2 Treat Edges as Laws

An edge is not only:

- `source -> target`

It also defines:

- what state is observed
- how influence is transformed
- how much influence is allowed
- when propagation should occur

### 4.3 Determinism First

For the MVP:

- single process
- discrete ticks
- deterministic evaluation order
- explicit convergence and divergence handling

This is more important than raw throughput.

### 4.4 Stability Is a First-Class Requirement

The engine must not assume interactions are naturally well-behaved.

It must provide:

- damping
- clipping
- leak or decay
- scheduling control
- oscillation detection
- divergence detection

## 5. Domain Concepts

### 5.1 Node

A node is a stateful entity.

Each node has:

- identity
- node type
- internal state
- emitted influence or observable projection
- local dynamics parameters

Suggested model:

```rust
pub struct NodeState {
    pub internal: StateVector,
    pub emitted: SignalVector,
    pub retained_energy: f32,
    pub phase: u64,
}
```

### 5.2 Edge

An edge is an interaction channel.

Each edge has:

- source node
- target node
- interaction type
- weight
- attenuation
- optional delay
- activation or guard condition

Suggested model:

```rust
pub struct InteractionEdge {
    pub id: EdgeId,
    pub source: NodeId,
    pub target: NodeId,
    pub law: LawId,
    pub weight: f32,
    pub attenuation: f32,
    pub delay: u32,
    pub enabled: bool,
}
```

### 5.3 Influence

Influence is the transmitted effect from one node to another.

It should be explicit and bounded.

```rust
pub struct Influence {
    pub signal: SignalVector,
    pub magnitude: f32,
    pub cause: CauseId,
}
```

### 5.4 Dynamics

Each node updates itself using:

- its current state
- incoming influences
- external input

```rust
pub trait NodeDynamics {
    fn step(
        &self,
        current: &NodeState,
        incoming: &[Influence],
        external: Option<&ExternalInput>,
    ) -> NodeState;
}
```

### 5.5 Interaction Law

Each edge computes influence from source to target.

```rust
pub trait InteractionLaw {
    fn influence(
        &self,
        source: &NodeState,
        edge: &InteractionEdge,
        target: &NodeState,
    ) -> Influence;
}
```

## 6. Execution Model

### 6.1 Tick-Based Runtime

The MVP uses discrete ticks.

Each tick consists of:

1. collect external commands
2. apply direct mutations into a transaction buffer
3. gather active outbound edges for affected nodes
4. evaluate interaction laws
5. accumulate incoming influences per target node
6. apply stabilization
7. run node dynamics
8. compute deltas
9. detect convergence, oscillation, or divergence
10. commit tick results

This gives:

- deterministic replay
- better observability
- safer cycle handling

### 6.3 Inspection and Query

The engine exposes a read layer for inspection, not a product query language.

Core read contracts:

- `WorldSnapshot` for consistent state reads
- selector-based entity resolution
- neighborhood and channel inspection
- transaction and delta inspection
- combined tick inspection over current snapshot and recent tick deltas

This layer exists to support engine validation, debugging, and adapter integration.
It does not introduce domain semantics.

### 6.2 Transactions

Transactions are not only storage updates.

They are bounded state-transition batches.

A transaction records:

- external intent
- touched nodes and edges
- generated deltas
- causal relationships
- resulting tick span

### 6.3 Deltas

All updates should be represented as deltas.

```rust
pub struct StateDelta {
    pub node: NodeId,
    pub before: NodeState,
    pub after: NodeState,
    pub norm: f32,
    pub cause: CauseId,
}
```

This is useful for:

- rollback or replay
- debugging
- convergence metrics
- causal tracing

## 7. Stabilization Strategy

> **Note**: This section reflects the Phase 1+2 framing and is now
> superseded. `identity.md` §3 establishes the correct framing: stability is
> a **guard rail**, not a goal. The mechanisms below (alpha blending,
> saturation, trust region, decay) are implemented as `StabilizationConfig`
> and `AdaptiveGuardRail`; their purpose is to keep dynamics *observable*,
> not to drive the system toward a fixed point.

The system should combine multiple stabilizers rather than rely on one theorem or heuristic.

### 7.1 Under-Relaxation

Blend the new state with the previous state:

`x_next = (1 - alpha) * x_prev + alpha * x_raw`

Where:

- `0 < alpha <= 1`
- smaller `alpha` reduces overshoot and explosive feedback

### 7.2 Edge Saturation

Influence functions must be bounded.

Recommended saturation forms:

- `tanh`
- sigmoid
- clipped linear
- softsign

This prevents a single edge from generating arbitrarily large response.

### 7.3 Leak and Decay

Nodes should not accumulate influence forever.

`state_next = retain * state_prev + incoming + local_update`

Where:

- `0 <= retain < 1`

This acts as dissipation.

### 7.4 Trust Region

Per tick, each node change should be bounded:

- max state delta
- max emitted magnitude
- max accumulated influence

This is a practical defense against numerical and structural instability.

### 7.5 Adaptive Step Size

When local instability is detected:

- reduce alpha
- lower edge weights
- clamp update size

The adaptation should be local to the unstable region when possible.

### 7.6 SCC-Aware Scheduling

Cycles are expected.

The runtime should:

- decompose the graph into strongly connected components
- update acyclic regions in topological order
- treat cyclic regions as iterative blocks

This gives better control over convergence.

### 7.7 Oscillation and Divergence Detection

The engine must classify runtime behavior:

- fixed-point convergence
- bounded oscillation
- limit-cycle suspicion
- unbounded divergence

Suggested indicators:

- delta norm history
- sign-flipping frequency
- repeated state signatures
- energy growth

## 8. Mathematical Guidance

The architecture is informed by several bodies of theory.

### 8.1 Contraction and Incremental Stability

Use contraction-style reasoning where possible:

- local Jacobian gain should stay bounded
- the effective update map should behave like a contraction in a suitable metric

This is the best direct guide for preventing explosion in nonlinear interaction graphs.

### 8.2 Lyapunov and Dissipativity

Define runtime diagnostics that behave like energy:

- total influence norm
- per-SCC energy
- emitted versus absorbed energy

Prefer update rules where energy tends to decay or remain bounded.

### 8.3 Consensus and Network Dynamics

Graph-local interactions often resemble multi-agent systems.

Useful ideas:

- Laplacian-like normalization
- neighbor averaging
- coupling strength bounds
- synchronization thresholds

### 8.4 Signed Graphs and Frustration

If excitatory and inhibitory interactions mix, contradictory cycles can destabilize the system.

The runtime should compute diagnostics such as:

- signed cycle count
- SCC sign consistency
- approximate frustration score

### 8.5 Topological Diagnostics

Topology is more useful for diagnosis than for the core update rule.

Longer-term tooling can include:

- attractor basin analysis
- Morse decomposition
- Conley-style invariant set diagnostics

## 9. Rust Workspace Structure

```text
crates/
  graph-core/
    src/
      ids.rs
      value.rs
      state.rs
      node.rs
      edge.rs
      influence.rs
      law.rs

  graph-world/
    src/
      world.rs
      snapshot.rs
      index.rs
      metrics.rs

  graph-engine/
    src/
      coordinator.rs
      dynamics.rs
      diagnostics.rs
      stabilizer.rs
      trace.rs
      engine/
        mod.rs
        routing.rs
        dispatch.rs
        aggregation.rs
        source.rs
        state.rs
        provenance.rs

  graph-tx/
    src/
      transaction.rs
      delta.rs
      causal.rs
      replay.rs
      wal.rs

  graph-testkit/
    src/
      fixtures.rs
      simulation.rs
      generators.rs
      assertions.rs
```

## 10. Module Responsibilities

### 10.1 `graph-core`

Pure data model and core traits.

- ids
- values
- states
- edges
- laws
- influence types

### 10.2 `graph-world`

In-memory representation of the live graph.

- node storage
- channel storage
- indices
- snapshot reads
- lightweight world metrics

### 10.3 `graph-engine`

Runtime execution layer.

- tick loop
- source dispatch
- routed propagation
- stabilization hooks
- trace hooks
- diagnostics generation

### 10.4 `graph-tx`

Command and delta flow.

- transactions
- causal records
- state deltas
- append-only WAL
- replay support

### 10.5 `graph-testkit`

Verification layer.

- scenario fixtures
- unstable graph generators
- convergence assertions
- fuzz and property tests

## 11. Core Traits

```rust
pub trait Stabilizer {
    fn stabilize_influence(&self, raw: Influence, ctx: &StepContext) -> Influence;
    fn stabilize_delta(&self, raw: StateDelta, ctx: &StepContext) -> StateDelta;
}

pub trait ConvergencePolicy {
    fn classify(&self, history: &DeltaHistory) -> RuntimeStatus;
}

pub trait Scheduler {
    fn schedule(&self, world: &World, dirty: &[NodeId]) -> Schedule;
}
```

## 12. Storage and Query Strategy

The MVP should stay simple.

Recommended choices:

- `slotmap` or compact indexed IDs for nodes and edges
- adjacency lists for propagation
- separate secondary indices by type
- direct Rust API first
- query layer later

Do not begin with:

- Cypher compatibility
- complex declarative query planner
- disk-first storage engine

## 13. MVP Scope

### 13.1 Include

- in-memory world state
- typed nodes and interaction edges
- discrete tick runtime
- deterministic scheduler
- SCC decomposition
- delta tracking
- damping and clipping
- convergence and divergence classification
- snapshot save and load

### 13.2 Exclude

- distributed runtime
- full MVCC
- ad hoc scripting in edge laws
- custom query language
- plugin marketplace
- online topology mutation during the same critical tick path

## 14. Testing Strategy

The engine should be tested more like a dynamical runtime than a CRUD database.

Required test classes:

- deterministic replay tests
- convergence tests
- bounded oscillation tests
- divergence detection tests
- SCC scheduling tests
- signed-cycle stress tests
- random graph fuzz tests
- snapshot and replay consistency tests

Useful property checks:

- repeated replay yields identical states
- bounded laws with damping do not produce unbounded norm growth
- removing an edge cannot increase influence along that edge path in monotone modes

## 15. Incremental Roadmap

### Phase 1

- world model
- node and edge types
- tick engine
- simple stabilizer
- state delta log

### Phase 2

- SCC-aware scheduler
- convergence classifier
- adaptive damping
- causal trace

### Phase 3

- snapshot and WAL
- richer law library
- diagnostics dashboard
- offline attractor analysis

### Phase 4

- query API
- topology mutation policies
- performance tuning
- optional parallel SCC execution

## 16. Reference Mapping

### 16.1 From IndraDB

Adopt:

- typed graph core
- embedded library-first structure
- clear node and edge identity model

Do not adopt directly:

- edge semantics as simple relation only
- storage-first product identity

### 16.2 From Memgraph

Adopt:

- in-memory-first architecture
- snapshot and WAL thinking
- delta-oriented change tracking
- transaction discipline

Do not adopt directly:

- query-engine-first architecture
- classical graph DB product boundary

### 16.3 From Dynamical Systems

Adopt:

- state transition formalism
- coupling laws
- contraction and dissipation ideas
- graph stability diagnostics
- attractor and oscillation analysis

## 17. Open Questions

These should be resolved before implementation hardens:

- What is the minimal state representation: scalar, vector, enum, or hybrid?
- Are laws static per edge type or customizable per edge instance?
- Can topology change during a tick, or only between ticks?
- How much determinism is required across platforms and compiler versions?
- Should delays be part of the MVP?
- Is the initial engine synchronous only, or must selective async propagation be supported early?

## 18. Recommended Next Step

The next concrete deliverable should be a Rust workspace skeleton with:

- `graph-core`
- `graph-world`
- `graph-engine`
- `graph-stabilizer`

and a minimal end-to-end prototype that:

- creates a small graph
- runs several ticks
- applies damping
- records deltas
- reports convergence status
