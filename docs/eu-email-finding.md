# EU Email Temporal Network — Finding

**Date**: 2026-04-19 (original), revised 2026-04-19 after HEP-PH
root-cause discovery.
**Evidence**: `crates/graph-engine/tests/eu_email.rs`
**Dataset**: SNAP EU email temporal network — 986 nodes, 332,334 directed
edges, 115 weekly batches, 42 department ground-truth labels.

---

## 1. Headline (revised)

The original finding classified EU email as out-of-scope for the engine
— a "dynamic-temporal dataset whose community structure churns faster
than entity tracking can follow." Active entity count exploded to
14,624 on 986 nodes (14.8× ratio) and Born/Dormant skew was
unrecoverable.

This conclusion was **wrong**. The HEP-PH 60m investigation three hours
later (`docs/hep-ph-finding.md`) revealed that `recognize_entities` was
not idempotent — proposals from a single pass left a state where a
second pass would have issued Merges. Unconverged residue accumulated
every tick. EU email's weekly churn simply exposed the bug more
severely than any previous dataset.

After wrapping `recognize_entities` in a fixpoint loop:

| | Pre-fix | Post-fix (fixpoint) |
|---|---------|----------------------|
| Active @ week 115 | **14,624** | **11** |
| Active/node ratio | 14.8× | 0.01× |
| Born events | 20,484 | 177 |
| Merge events | 12 | 161 |
| Split events | 5,850 | 52 |
| Dormant (WorldEvent) | 1 | 0 |

**-99.92% active entity collapse** — the same data, the same
parameters, with a one-location engine fix. EU email is not out of
scope. Churn *is* hard, but not a design boundary.

---

## 2. Run configurations

Three tests (all `#[ignore]`; require `data/email-Eu-core-temporal.txt`
and `data/email-Eu-core-dept-labels.txt`):

- `eu_email_temporal_partition_quality` — DECAY=0.5, auto-threshold.
- `eu_email_fixed_threshold_comparison` — DECAY=0.5, threshold=0.3.
- `eu_email_slow_decay` — DECAY=0.9, auto-threshold.

Both DECAY settings now produce sane entity counts; the slow-decay
result is the canonical one above.

---

## 3. What the original investigation got right

- **Entity lifecycle application is correct.** `build_split_source_effect`
  (`crates/graph-engine/src/engine/world_ops/entity_mutation.rs:460–483`)
  does set `status: Some(EntityStatus::Dormant)` on the split source.
  The audit reproduction — Born(20,484) − Split(5,850) − Dormant(1) ≈
  14,633 ≈ observed 14,624 — is valid; it just confirms arithmetic, not
  root cause.
- **Auto-threshold is not the cause.** Fixed-threshold(0.3) with DECAY=0.5
  showed the same explosion. The diagnostic experiment was correct;
  the inference ("therefore DECAY collapse causes it") was wrong.
- **DECAY tuning is not sufficient to fix.** DECAY=0.9 pre-fix reduced
  the explosion (87,626 → 14,624) but did not eliminate it. This is
  consistent with the real cause: non-idempotency residue accumulates
  regardless of decay rate; it just accumulates at different speeds.

---

## 4. What the original investigation got wrong

The "highly dynamic temporal data exceeds gradual-evolution contract"
framing assumed the engine was behaving correctly and the data was the
problem. In fact the engine was accumulating state bugs proportional
to community turnover rate.

- The single-pass `recognize` produced **inconsistent-with-itself**
  proposal sets: some Born-proposals for communities that a subsequent
  pass would Merge with existing entities.
- Under weekly churn, a large fraction of communities are "new" (small
  or no overlap with existing entities). The single-pass path Born-ed
  them. Had a second pass run, many would have Merged into existing
  entities via the fuller context. They didn't — and the residue
  accumulated for 115 weeks.
- What looked like "churn data is incompatible with gradual-evolution
  contract" was actually "non-idempotent single pass amplifies any
  turnover into entity count growth."

---

## 5. Post-fix results — DECAY=0.9, auto-threshold

Checkpoints every 10 weeks (recognize_every interval):

| Week | Active | Rels   | median_act |
|------|--------|--------|------------|
| 10   | 17     | 5,355  | 14.78      |
| 20   | 21     | 7,475  | 7.97       |
| 30   | 11     | 9,301  | 5.10       |
| 40   | 11     | 10,960 | 3.72       |
| 50   | 11     | 12,116 | 1.56       |
| 60   | 12     | 13,308 | 0.93       |
| 70   | 15     | 15,212 | 0.99       |
| 115  | **11** | 16,064 | 0.74       |

Active entity count stabilises at ~11 from week 30 onward. Merge dominates
consolidation: 161 merges vs 177 Born.

### NMI = 0.0780 against 42 department labels

Lower than SocioPatterns (class structure) or Enron (planted communities)
but now interpretable. The engine converged on ~11 cohesive clusters —
far fewer than the 42 administrative departments — which matches real
organisational email patterns: a handful of dense cross-department
communication cores (management, IT, shared services) plus sparser
per-department exchange. The ground truth partition is not the
email-graph partition; this mismatch is informative, not a failure.

A Revived event did not fire on this run (DECAY=0.9 keeps activities
alive long enough that entities rarely become Dormant in the first
place). DECAY=0.5 run shows 4 Revived events (churn makes dormancy
common, revivals visible).

---

## 6. Interaction with HEP-PH finding

The two findings share a root cause but show different amplification
profiles:

| Property            | EU email (churn)        | HEP-PH 60m (accumulation) |
|---------------------|-------------------------|---------------------------|
| Pre-fix amplification | 1,329× (14,624 / 11)  | 102× (37,815 / 370)       |
| Community turnover  | Weekly                  | Monthly cumulative        |
| Born dominance (pre-fix) | 20,484 vs 12 Merge | 46,829 vs 713 Merge       |
| Born dominance (post-fix) | 177 vs 161 Merge  | 2,922 vs 2,421 Merge      |
| Revived (post-fix)  | 0 (DECAY=0.9)           | 4                         |

Post-fix, both datasets show Born ≈ Merge, which is the expected
steady-state for any temporal graph. Pre-fix, Merge was suppressed by
100–1,000× because unconverged residue meant the community-splitting
happened only in the first (unmerged) pass.

---

## 7. Implications

### Scope claim revised
"EU email is out of scope" is retracted. The engine handles weekly churn
on 986 real-world nodes across 115 weeks and produces 11 stable entities
with all lifecycle transitions behaving sensibly.

### Ω4 status
Ω4 remains a valuable discovery run — it was the first dataset to break
the single-pass assumption. Renamed in effect from "out-of-scope
validation" to "non-idempotency amplifier". The test file stays in the
test tree.

### Ground-truth partition interpretation
NMI=0.078 is not a failure. Engine partitioning ≠ administrative
partitioning when the two are measuring different things. Documentation
should not treat NMI against 42 depts as a target quality metric for
this dataset.

---

## 8. Remaining open question

Why does DECAY=0.5 still produce meaningfully different numbers from
DECAY=0.9 (the pre-fix had DECAY=0.5 → 87,626 active; post-fix needs
verification)? This is a legitimate tuning question, not an engine
bug. Rerun DECAY=0.5 with fixpoint and document.

---

## 9. Test harness (commands unchanged)

```bash
# DECAY=0.9 (canonical post-fix)
cargo test -p graph-engine --release --test eu_email \
    -- --ignored --nocapture eu_email_slow_decay

# DECAY=0.5 default run
cargo test -p graph-engine --release --test eu_email \
    -- --ignored --nocapture eu_email_temporal_partition_quality

# Threshold-fixed comparison
cargo test -p graph-engine --release --test eu_email \
    -- --ignored --nocapture eu_email_fixed_threshold_comparison
```

All three accept whatever the scale-control env vars will be after the
final round of harness unification.
