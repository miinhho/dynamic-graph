# Design Reference

This document describes how the engine is actually built — crate responsibilities,
data flow, and the relationship model in depth. It is descriptive, not aspirational.

For *what the project is* and the ontological framing, see [`identity.md`](./identity.md).
For early design exploration (some of which is now superseded), see [`architecture.md`](./architecture.md).
When this document and `identity.md` disagree, `identity.md` wins.

---

## 1. Crate map

```
graph-core          Pure data types and traits. No I/O, no state.
graph-world         In-memory stores. Knows nothing about the batch loop.
graph-engine        Batch loop, registries, Simulation façade.
graph-storage       Persistence via redb (optional feature).
graph-query         Read-only query surface over &World.
graph-testkit       Test programs, fixtures, generators.
```

Dependency direction: `graph-core` ← `graph-world` ← `graph-engine` ← `graph-storage`.
`graph-query` depends on `graph-world` only. `graph-testkit` depends on all of the above.

---

## 2. The five layers

```
Layer 4  Cohere        — clusters under a perspective         ephemeral, recomputed on demand
Layer 3  Entity        — coherent bundles of relationships    sedimentary, never deleted
Layer 2  Relationship  — pairwise coupling between loci       emergent or explicit
Layer 1  Change        — atomic event with causal predecessors  append-only log
Layer 0  Locus         — labeled position with state + program  user-registered
```

Layers 0–1 are primitives; layers 2–4 are derived. The user registers loci and
programs (layer 0), injects stimuli (layer 1), and observes what emerges.

---

## 3. The batch loop

`Engine::tick(world, loci_registry, influence_registry, stimuli) -> TickResult`

Each call drives the world forward until it goes quiescent or hits
`max_batches_per_tick`. One iteration of the inner loop is one *batch*:

```
1. Assign BatchId. Drain pending Vec<PendingChange>.

2. For each pending change:
   a. ChangeSubject::Locus(locus_id)
      - Drop if locus does not exist.
      - Stabilize `after` against the kind's StabilizationConfig.
      - Identify cross-locus predecessors: predecessor changes that targeted
        a *different* locus — these imply an influence edge from that locus
        to the current one.
      - Commit Change to the log. Update locus.state = stabilized_after.
      - Apply property_patch (if any) to PropertyStore.
      - For each cross-locus predecessor (from_locus, pre_signal):
          auto_emerge_relationship(world, from_locus, locus_id, kind, …)
          record Hebbian observation (rel_id, pre_signal, post_signal)
      - Record committed ChangeId in committed_ids_by_locus[locus_id].

   b. ChangeSubject::Relationship(rel_id)
      - Stabilize `after`.
      - Commit Change. Update relationship.state = stabilized_after.
      - If any locus subscribed to rel_id: queue (rel_id, change_id) for
        notification delivery (next step).

3. Resolve subscriber notifications:
   For each queued (rel_id, change_id), add change_id to every
   subscriber locus's inbox for this batch. Subscription changes deliver
   in the *same* batch as the relationship change.

4. Build BatchContext (read-only snapshot of current world state).

5. Dispatch LocusProgram::process and ::structural_proposals for every
   locus that received at least one change this batch (subject to
   refractory check). Collect Vec<ProposedChange> and Vec<StructuralProposal>.

6. Queue follow-up ProposedChanges as the next batch's pending list.

7. Apply structural proposals (CreateRelationship / DeleteRelationship /
   Subscribe / Unsubscribe).

8. Apply Hebbian weight updates: Δweight = η × pre × post for each
   recorded observation.

9. Advance BatchId. Loop back to step 1 if pending is non-empty.
```

### What programs see

`BatchContext` reflects the world *after the previous batch committed* and
*before the current batch's program outputs are committed*. Programs cannot
observe other programs' outputs from the same batch. Relationship state read
via `ctx.relationship_between_kind()` is the decayed-and-committed value from
the end of the last batch.

---

## 4. Relationship model

### 4.1 State vector layout

Every relationship carries a `StateVector`:

```
Slot 0   activity       built-in  incremented on each auto-emerge touch; decayed per batch
Slot 1   weight         built-in  Hebbian learning: Δw = η × pre × post
Slot 2…  extra slots    user-defined via InfluenceKindConfig::extra_slots
```

Extra slots occupy indices 2, 3, 4, … in the order they appear in
`InfluenceKindConfig::extra_slots`. Each has a name, a default value used at
creation, and an optional per-slot decay rate independent of the kind-level
`decay_per_batch`. A slot with `decay: None` never decays — useful for
cumulative counts.

```rust
InfluenceKindConfig::new("conflict")
    .with_decay(0.95)          // applied to activity slot
    .with_extra_slots(vec![
        RelationshipSlotDef::new("hostility", 0.0).with_decay(0.98),
        RelationshipSlotDef::new("engagement_count", 0.0),  // no decay
    ]);
// StateVector layout: [activity, weight, hostility, engagement_count]
```

Reading by name inside a program:

```rust
let h = ctx.relationship_slot(rel_id, CONFLICT_KIND, "hostility").unwrap_or(0.0);
```

Writing a partial update:

```rust
let new_state = rel.state.clone()
    .with_slot(2, new_hostility)
    .with_slot(3, engagements + 1.0);
// slots 0 and 1 are copied from the existing state; only 2 and 3 are changed
```

### 4.2 Directed vs symmetric

```rust
Endpoints::Directed { from, to }   // A → B  order carries meaning
Endpoints::Symmetric { a, b }      // A ↔ B  order is normalized at storage
```

`EndpointKey` is the canonical dedupe key. For `Symmetric`, the two IDs are
sorted (lower first) so `(A, B)` and `(B, A)` hash to the same key. The
relationship store's `by_key` index is `FxHashMap<(EndpointKey, RelationshipKindId), RelationshipId>`,
so the same endpoint pair with different kinds produces independent entries.

Auto-emergence (`auto_emerge_relationship`) always creates **Directed**
relationships (from the predecessor locus toward the successor). `Symmetric`
relationships must be created explicitly via `StructuralProposal::CreateRelationship`
or inserted directly before the engine runs.

### 4.3 Multi-kind between the same two loci

Fully supported. The store key is `(EndpointKey, RelationshipKindId)`, not
just `EndpointKey`. Two loci A and B can simultaneously hold:

- a `conflict` relationship (`Symmetric`, slots: activity, weight, hostility, engagement)
- a `trust` relationship (`Symmetric`, slots: activity, weight, trust_level)
- a `resource_flow` relationship (`Directed` A→B, slots: activity, weight, rate)

Each is an independent `Relationship` with its own `RelationshipId`, `StateVector`,
`RelationshipLineage`, and lazy-decay cursor. Programs query a specific kind
with `ctx.relationship_between_kind(a, b, kind)`.

### 4.4 Auto-emergence in detail

Triggered during the `ChangeSubject::Locus` commit path when a pending change
has predecessors that targeted a *different* locus:

```
pending change: locus=B, kind=K, predecessors=[c1, c2, ...]
  for each predecessor cX where cX.subject == Locus(A), A ≠ B:
    auto_emerge_relationship(world, A, B, K, …)
```

The emerged relationship is `Directed { from: A, to: B }` by default.
If the kind's `InfluenceKindConfig` has `symmetric: true`, the emerged
relationship uses `Endpoints::Symmetric { a: A, b: B }` instead, so
mutual stimulation between two loci produces a single shared edge rather
than two independent directed edges.

```rust
inf_reg.insert(
    CO_OCCURRENCE_KIND,
    InfluenceKindConfig::new("co_occurrence")
        .with_decay(0.95)
        .symmetric(),   // mutual stimulation → one Symmetric edge
);
```

`auto_emerge_relationship` is idempotent via the `by_key` index:
- **Existing**: apply accumulated lazy decay, add 1.0 to activity, update lineage.
- **New**: build `StateVector` from `InfluenceKindConfig::initial_relationship_state()`
  (slots at their `default` values, activity=1.0), insert with `created_by = Some(change_id)`.

The decay application in auto-emerge happens *before* bumping activity so that
the increment lands on the correctly-decayed baseline:

```
new_activity = old_activity × decay^(current_batch - last_decayed_batch) + 1.0
```

### 4.5 Lazy decay

Relationship decay is not applied every batch. It accumulates and is flushed:

1. **On touch** (`auto_emerge_relationship`): before bumping activity.
2. **On flush** (`Engine::flush_relationship_decay`): called before entity
   recognition or on demand. Iterates all live relationships and applies
   `decay^(current_batch - last_decayed_batch)` to activity, weight-decay to
   weight, and per-slot rates to extra slots.

**Consequence**: reading a relationship's raw `state` between touches may show
a stale (un-decayed) value. Always call `flush_relationship_decay` before
inspecting absolute state values outside of a program context.

Programs reading via `ctx.relationship_between_kind()` see the state as
committed at the end of the previous batch — which already had decay applied
(if the relationship was touched) or still has accumulated un-flushed decay
(if it was not touched that batch). For most workloads this is fine because
decay per batch is typically small (0.95–0.99).

### 4.6 Pre-created vs auto-emerged relationships

**Auto-emerged** relationships always start with `initial_relationship_state()`:
`[activity=1.0, weight=0.0, extra_slot_defaults...]`. There is no way to
override the initial values through the emergence path.

**Pre-created** relationships (inserted before the engine runs, or via
`StructuralProposal::CreateRelationship`) can carry arbitrary initial state.
Use `World::add_relationship` for the cleanest insertion — it sets
`last_decayed_batch` to the current batch automatically:

```rust
let ab_rel_id = world.add_relationship(
    Endpoints::Symmetric { a: FORCE_A, b: FORCE_B },
    CONFLICT_KIND,
    StateVector::from_slice(&[1.0, 0.0, 0.3, 0.0]),  // hostility=0.3 at birth
);
```

Use this pattern when the domain requires a relationship to exist with specific
prior state before any change flow has been observed (e.g., a pre-existing
political conflict with known initial hostility).

`StructuralProposal::CreateRelationship` cannot set initial slot values beyond
the kind's defaults — it only specifies `endpoints` and `kind`. If a program
needs to create a relationship with custom state, it should use
`ChangeSubject::Relationship` in a follow-up change after creating it, or
the world-builder pattern above.

### 4.7 Relationship changes and subscriptions

A `ProposedChange` with `ChangeSubject::Relationship(rel_id)` goes through
the **relationship commit path** in the batch loop:

- Stabilizes the proposed `StateVector` against the kind's `StabilizationConfig`.
- Updates `relationship.state`.
- Records `last_touched_by = change_id`, increments `change_count`.
- If any locus subscribed to `rel_id`, delivers the committed `Change` to each
  subscriber's inbox in the **same batch**.

The relationship commit path does **not** trigger cross-locus predecessor detection
and does **not** auto-emerge new relationships. It is purely a state update on
an existing edge. The change is recorded in the `ChangeLog` like any other change,
with `ChangeSubject::Relationship(rel_id)` as its subject.

This means: two `ChangeSubject::Relationship` changes whose predecessor sets
reference each other do **not** cause new relationships to emerge between their
subject edges. Edge-to-edge causal linkage exists in the DAG (via predecessor
IDs) but the engine does not synthesize new relationships from it.

### 4.8 N-ary interactions via the EventLocus pattern

Relationships are strictly pairwise. Multi-party interactions are expressed
with a dedicated **event locus** that accumulates signals from participants
and, once its activation threshold is crossed, creates pairwise participation
edges to all involved parties via `StructuralProposal::CreateRelationship`.

```
Force_A ──signal──▶ Event_Locus ◀──signal── Force_B
                         │
        (threshold crossed — structural_proposals fires)
                         │
           ┌─────────────┼─────────────┐
           ▼             ▼             ▼
       Event→A       Event→B       … (any number of participants)
```

The event locus is a regular `Locus` with a program (`EventLocusProgram`)
that accumulates incoming activation and emits structural proposals when
ready. It can also subscribe to the relationships it creates, enabling it
to continue monitoring participant pairs after the initial event.

**When to use this pattern**:
- Modeling events with variable arity (diplomatic summits, multi-body conflicts,
  joint ventures).
- When the N-way interaction needs its own identity and state history (event
  severity, timestamps, confidence).
- When participants must be able to query "what events have I been part of"
  via graph traversal.

**Alternative for fixed-arity interactions**: if the arity is always 2, just
model it as a direct pairwise relationship. The event locus adds overhead that
is only justified when the N-ary semantics matter.

### 4.9 Subscription model

Loci can subscribe to specific relationship state changes. The subscription
store maps `RelationshipId → Vec<LocusId>`. When a committed change has
`ChangeSubject::Relationship(rel_id)`, the engine delivers its `ChangeId` to
every subscribed locus's inbox for that batch.

Two registration paths:

1. **`initial_subscriptions`** — program method called by
   `Engine::bootstrap_subscriptions` before the first tick. Use when the
   relationship already exists at world-construction time and the locus needs
   to monitor it from batch 0.

2. **`StructuralProposal::SubscribeToRelationship`** — returned from
   `structural_proposals` at runtime. Use when the subscription is contingent
   on some dynamic condition (e.g., subscribe to a newly created edge).

Subscriptions are persisted in `graph-storage` via `SubscriptionStore`. The
`WorldDiff` from `world.diff_since(batch)` includes `subscriptions_added` and
`subscriptions_removed` for auditing.

### 4.10 Relationship store index

```
RelationshipStore {
    by_id:    FxHashMap<RelationshipId, Relationship>
    by_key:   FxHashMap<(EndpointKey, RelationshipKindId), RelationshipId>
    by_locus: FxHashMap<LocusId, Vec<RelationshipId>>
}
```

`by_key` enables O(1) dedup during auto-emergence and idempotent structural
proposals. `by_locus` enables O(k) traversal of a locus's neighborhood
(k = degree). Both indices are kept in sync on every insert and remove.

| Query | Path | Complexity |
|-------|------|-----------|
| `get(id)` | `by_id` lookup | O(1) |
| `lookup(key, kind)` | `by_key` lookup | O(1) |
| `degree(locus)` | `by_locus[locus].len()` | O(1) |
| `relationships_for_locus(locus)` | iterate `by_locus[locus]`, resolve each | O(k) |
| `relationships_from / _to` | filter the above by endpoint direction | O(k) |
| `relationships_between(a, b)` | iterate `by_locus[a]`, filter by `involves(b)` | O(k_a) |

---

## 5. Extension points

### LocusProgram

The primary user hook. Implement `process` to return `Vec<ProposedChange>` and
optionally override `structural_proposals` for topology changes. Programs are
stateless (no `&mut self`) — per-locus state lives in `Locus::state` and in
relationship slots, both accessible via the batch context.

```rust
impl LocusProgram for MyProgram {
    fn process(&self, locus: &Locus, incoming: &[&Change], ctx: &dyn LocusContext)
        -> Vec<ProposedChange>
    { … }

    fn structural_proposals(&self, locus: &Locus, incoming: &[&Change], ctx: &dyn LocusContext)
        -> Vec<StructuralProposal>
    { … }

    fn initial_subscriptions(&self, locus: &Locus) -> Vec<RelationshipId>
    { … }
}
```

### InfluenceKindConfig

Per-kind configuration for decay, stabilization, Hebbian plasticity, and
extra relationship slots. Set once at world-construction time via the registry.

```rust
InfluenceKindConfig::new("kind_name")
    .with_decay(0.95)
    .with_stabilization(StabilizationConfig { alpha: 0.7, … })
    .with_plasticity(PlasticityConfig { learning_rate: 0.05, … })
    .with_extra_slots(vec![
        RelationshipSlotDef::new("slot_name", default_val).with_decay(rate),
    ]);
```

### EmergencePerspective / CoherePerspective

On-demand hooks for layers 3 and 4. Called explicitly by the user between ticks
(or via `Simulation::recognize_entities`). The perspective inspects the current
relationship graph and proposes entity clusters or cohere sets. The engine
applies the proposals but does not drive the perspective automatically.

### EntityWeatheringPolicy

Controls how entity sediment layers erode over time. Applied either on demand
(`Engine::weather_entities`) or automatically on a configured cadence
(`SimulationBuilder::auto_weather(n)`).

---

## 6. Simulation façade

`Simulation` (built via `SimulationBuilder`) is the recommended entry point
for most users. It wires together `Engine`, `InfluenceKindRegistry`,
`LocusKindRegistry`, `AdaptiveGuardRail`, and optional `Storage`.

```rust
let mut sim = SimulationBuilder::new()
    .locus_kind("FORCE", ConflictActorProgram::new())
    .influence("conflict", |cfg| cfg.with_decay(0.95).with_extra_slots(…))
    .default_influence("conflict")
    .auto_weather(100)          // run DefaultEntityWeathering every 100 ticks
    .build();

sim.ingest_named("Force_A", "FORCE", props! { "region" => "north" });
let obs = sim.step(stimuli);    // obs.regime, obs.relationships, …
```

`step()` order of operations:
1. Drain `pending_stimuli` buffered by `ingest()`.
2. Apply guard-rail-scaled alphas to influence configs.
3. `Engine::tick`.
4. Classify regime (`BatchHistory` + `DefaultRegimeClassifier`).
5. Feed regime back to `AdaptiveGuardRail`.
6. Persist committed batches to storage (if configured).
7. Trim `ChangeLog` to `change_retention_batches` (if configured).
8. Evict cold relationships (if `cold_relationship_threshold` configured).
9. Auto-weather entities (if interval reached).
10. Return `StepObservation`.

---

## 7. Persistence (graph-storage)

`Storage::open(path)` opens a `redb` database (exclusive file lock).
Two persistence modes:

- **Full snapshot**: `save_world` / `load_world` — serializes all loci,
  relationships, subscriptions, and the change log. Suitable for checkpoints.
- **Incremental**: `commit_batch(world, batch_id)` — appends one batch's
  committed changes. Skips the subscription table rewrite if `SubscriptionStore::generation()`
  is unchanged since last write.

Schema version is stored as a metadata key. `open_and_migrate` handles v1→v2
(adds `wall_time` and `metadata` to `Change`). Never open the same file from
two `Storage` instances simultaneously.

Hot/cold memory tiering: `Simulation::promote_relationship(rel_id)` and
`promote_relationships_for_locus(locus_id)` restore cold relationships from
storage into the live `RelationshipStore`.

---

## 8. Observed constraints and design notes

### Relationships are always pairwise

By design. There is no hyperedge type. The EventLocus pattern (§4.8) covers
N-ary cases. If a future use case demands native N-ary edges, it would require
a new `Endpoints::Hyperedge(Vec<LocusId>)` variant and matching index changes.
The current design deliberately defers this.

### Auto-emergence endpoint shape

`auto_emerge_relationship` produces `Endpoints::Directed { from: predecessor_locus, to: current_locus }`
by default. When the kind's `InfluenceKindConfig::symmetric` flag is `true`, it
produces `Endpoints::Symmetric { a, b }` instead.

Without `symmetric: true`, mutual stimulation between A and B produces two
independent directed edges (`Directed(A, B)` and `Directed(B, A)`), each with
its own `RelationshipId` and activity cursor. For undirected domains (co-occurrence,
shared resonance, mutual conflict), set `.symmetric()` on the kind config to get
a single shared edge.

`ctx.relationship_between_kind(a, b, kind)` finds relationships in all endpoint
shapes, so a `Directed(A, B)` edge *is* returned when B queries for its edge to A.

### Relationship kind == InfluenceKindId

Every relationship kind is also an influence kind (same numeric dimension).
Registering an influence kind registers it for both relationship storage and
batch-loop decay. There is no concept of a "relationship-only kind" that
doesn't participate in change dispatch.

### Programs see pre-commit relationship state

During dispatch (step 5 of the batch loop), `BatchContext` holds a read-only
reference to the world *before any of this batch's changes have been applied to
relationships*. If the same tick's earlier changes (committed in step 2) included
a `ChangeSubject::Relationship` update, **programs do not see that update** —
they see the relationship state as of the end of the previous batch.

This is intentional (it makes the batch loop deterministic and independent of
program execution order), but it means that relationship state and locus state
*within the same batch* are not coherent. The next batch's programs will see
the fully committed state.

### Structural proposals take effect end-of-batch

`CreateRelationship`, `DeleteRelationship`, `Subscribe`, and `Unsubscribe`
are applied after all programs in the batch have run (step 7). New relationships
created by structural proposals are not visible to any program in the same batch.
They are visible from the next batch onward.

### Subscription delivery is same-batch

Unlike structural proposals, subscription notifications from
`ChangeSubject::Relationship` changes are delivered in the **same batch**
(step 3, before dispatch). A subscriber program that runs in step 5 will
receive the relationship change in its inbox for that batch.

### Relationship changes do not trigger emergence

Only `ChangeSubject::Locus` changes with cross-locus predecessors trigger
`auto_emerge_relationship`. `ChangeSubject::Relationship` changes are recorded
in the log and delivered to subscribers, but they do not produce new
relationship-from-relationship edges. Edge-to-edge causal structure exists in
the `ChangeLog` DAG but the engine does not synthesize topology from it.

### Pre-created relationships and lazy decay

`World::add_relationship` sets `last_decayed_batch = world.current_batch().0`
at insertion time, so the new edge starts with no accumulated decay debt.

If you use `world.relationships_mut().insert()` directly and leave
`last_decayed_batch: 0`, `auto_emerge_relationship` will apply `decay^N` (where
N = current_batch) on the first touch — potentially collapsing activity to near
zero for long-running worlds. Prefer `add_relationship` whenever pre-creating
relationships in a running world.
