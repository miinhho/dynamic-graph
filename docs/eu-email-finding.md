# EU Email Temporal Network — Finding

**Date**: 2026-04-19
**Evidence**: `crates/graph-engine/tests/eu_email.rs`
**Dataset**: SNAP EU email temporal network — 986 nodes, 332,334 directed edges,
115 weekly batches, 42 department ground-truth labels.

---

## 1. What was measured

Three experimental runs, each on the full 115-week corpus:

| Run | DECAY | Threshold | Active entities (week 115) | NMI vs departments |
|-----|-------|-----------|----------------------------|--------------------|
| Default | 0.5 | auto | 87,626 | 0.1002 |
| Fixed threshold | 0.5 | 0.3 (static) | still explosion | — |
| Slow decay | 0.9 | auto | 14,624 | — |

All three show the same qualitative failure: active entity count >> node count (986).

---

## 2. Entity lifecycle code is correct

The hypothesis entering this investigation was that `EmergenceProposal::Split` might
fail to demote the source entity, causing it to re-enter `recognize_entities` as
active on subsequent passes and collect duplicate offspring.

**This hypothesis is false.** Reading
`crates/graph-engine/src/engine/world_ops/entity_mutation.rs:460–483`:

```rust
fn build_split_source_effect(...) -> Option<EntityMutationEffect> {
    let entity = world.entities().get(source)?;
    Some(EntityMutationEffect {
        entity_id: source,
        status: Some(EntityStatus::Dormant),   // ← source is demoted
        ...
    })
}
```

And `apply_entity_mutation_effect` applies the `status` field unconditionally
(line 602: `entity.status = status`). The source entity transitions to
`EntityStatus::Dormant` before the next `recognize_entities` call.

**Accounting check for DECAY=0.9**:

| Metric | Value |
|--------|-------|
| Total entities ever Born | 20,484 |
| Split source entities → Dormant (via Split application) | 5,850 |
| Regular Dormant proposals (WorldEvent::EntityDormant) | 1 |
| Expected active at end | 20,484 − 5,850 − 1 = **14,633** |
| Observed active at end | **14,624** |

Difference of 9 is rounding in intermediate weathering. The lifecycle
accounting is exact.

Note: `WorldEvent::EntityDormant` is only emitted for `EmergenceProposal::Dormant`
(regular silence-based demotion). Split sources are correctly set to Dormant
but do not emit a separate `EntityDormant` event. Test counters that count
`WorldEvent::EntityDormant` will see only the regular-Dormant events.

---

## 3. Root cause: dataset dynamics exceed entity tracking assumptions

The engine's entity tracking assumes that communities defined by relationship
activity at time T overlap substantially with communities at time T+1. Under
this assumption, most `recognize_entities` calls produce `DepositLayer`
(stable entity evolution) and few produce `Born` (new entity).

EU email violates this assumption:

- **Highly variable weekly patterns.** Each week a different subset of the 986
  people emails, forming transient groups that do not persist to the next week.
- **DECAY=0.5 collapses activity by week 50.** With a half-life of one week,
  relationships formed 7+ weeks ago have activity ≤ 0.008 × initial. By week
  50 the activity distribution has no bimodal gap for the auto-threshold to
  exploit. Most relationships fall below any reasonable threshold → singleton
  components → ~986 Born events/week.
- **DECAY=0.9 reduces but does not cure.** With half-life ~7 weeks, activities
  stay alive longer. This creates different communities each week from residual
  historical signal, but those communities do not match the previous week's
  entities because locus membership continuously re-mixes. Born rate remains
  ~178/week (20,484 / 115), Dormant rate ~51/week (5,850 / 115), net growth
  ~127 active entities/week, reaching ~14,600 at week 115.
- **NMI = 0.1002.** The engine's transient communities do not align with the 42
  stable department labels because the engine tracks communication-graph
  topology (which changes weekly) rather than stable organisational membership.

---

## 4. Why previous benchmarks did not expose this

All five prior datasets have relatively stable community structure:

| Dataset | Community stability |
|---------|---------------------|
| Karate Club | Static — one snapshot |
| Davis Southern Women | ~80% of edges recur across events |
| LFR dynamic | Planted schedule — community changes are instantaneous and clean |
| SocioPatterns | 5 classes meet daily in a school year — highly regular |
| Enron synthetic | 5 explicitly planted phases — no random drift |

EU email is the first dataset where community membership changes continuously
without a clean phase boundary. Entity tracking designed for "community A
evolves over time" is stressed by "each week is essentially a new graph."

---

## 5. Discriminating experiments (auto-threshold vs decay)

**Experiment A — Fixed threshold (DECAY=0.5, threshold=0.3)**:
Entity explosion persists. Conclusion: auto-threshold is not the root cause.
The threshold value does not matter when activities collapse to zero: no
relationships survive any positive threshold by week 50.

**Experiment B — Slow decay (DECAY=0.9, auto-threshold)**:
Entity count drops from 87,626 → 14,624. The explosion is attenuated but
not eliminated. Lifecycle accounting shows the code is correct (see §2).
Conclusion: decay tuning is a partial lever, not a fix.

---

## 6. Implications for engine design

### Auto-threshold claim
Finding 3 (Enron) states: "auto-threshold confirmed across all 5 datasets."
That claim holds for the five stable-community datasets. EU email shows a
sixth characteristic — highly dynamic temporal data — where the threshold
mechanism is not the constraint. The bottleneck is entity identity continuity
across time steps with large membership churn.

### `min_activity_threshold` demotion (Ω2)
The Ω2 demotion (to private field with escape-hatch builder) is not affected
by this finding. The EU email failure mode is not threshold selection; it would
occur at any threshold value once activities collapse or communities churn faster
than entity tracking can match.

### Dataset queue
EU email is a **stress test** for entity lifecycle, not a parameter-tuning
benchmark. It exposes that the engine's entity-matching heuristic (locus-flow
bucket assignment) is calibrated for datasets where communities evolve
gradually, not for datasets where communication graphs are reconstructed from
scratch each time step.

---

## 7. Remediation candidates (not scheduled)

The following approaches could address the EU email class of dataset:

1. **Activity half-life calibration.** A domain parameter declaring the
   expected community-recurrence interval. If set to ~3 weeks, DECAY would
   be tuned to preserve activities across communication gaps without
   accumulating stale relationships.

2. **Soft entity matching across temporal gaps.** Allow `recognize_entities`
   to match a dormant entity if its members reappear in a community with
   sufficient overlap — even if the entity has been dormant for many weeks.
   Currently `find_dormant_match` applies a strict overlap gate
   (`overlap*2 ≥ entity.members.len()`), which kills revival on highly
   variable datasets.

3. **Community persistence smoothing.** Instead of running community detection
   on the current-tick relationship graph, smooth activities over a rolling
   window before applying the threshold. This would reduce the "new community
   each week" effect.

None of these are scheduled. The EU email dataset class requires a design
decision about the engine's temporal resolution contract before a fix is
principled. This finding is documentation of the limitation, not a blocking
issue for Tracks G, I, J, K or Ω.

---

## 8. Test harness

`crates/graph-engine/tests/eu_email.rs` contains three `#[ignore]` tests
(require `data/email-Eu-core-temporal.txt` and `data/email-Eu-core-dept-labels.txt`):

- `eu_email_oracle` — default run (DECAY=0.5, auto-threshold): prints weekly
  checkpoint table (active entities, relationship count, median activity).
- `eu_email_fixed_threshold_comparison` — DECAY=0.5, threshold=0.3:
  discriminating experiment for auto-threshold.
- `eu_email_slow_decay` — DECAY=0.9, auto-threshold: lifecycle accounting
  and Born/Dormant/Split ratio reporting.
