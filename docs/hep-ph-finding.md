# HEP-PH Citation Network — Finding

**Date**: 2026-04-19
**Evidence**: `crates/graph-engine/tests/hep_ph.rs`
**Dataset**: SNAP ArXiv HEP-PH citation network — 34,546 papers,
421,578 citation edges, 122 monthly batches (1992-02 ~ 2002-03).

---

## 1. Motivation

EU email (`docs/eu-email-finding.md`) exposed a **dynamic-temporal**
failure mode: community membership churns faster than entity tracking
can match, producing Born floods on highly variable weekly data.

HEP-PH tests the opposite end of the engine's contract: citations are
**accumulative** (once paper A cites paper B, that link is permanent),
so physics subfield structure is expected to evolve gradually. This run
checks whether the engine **excels on its designed contract** at a scale
35× larger than EU email.

---

## 2. Scale guard

To avoid OOM at full window (34K papers × 122 months can produce
hundreds of thousands of entities), the test accepts env vars:

- `HEP_PH_MAX_MONTHS` (default 24) — time window
- `HEP_PH_MAX_ENTITIES` (default 50,000) — early-abort guard

Runs are split into a **contract region** (24 months) and a **stress
region** (60 months) for comparison.

---

## 3. Results — 24-month window (contract region)

Three DECAY settings, all with auto-threshold (`min_activity_threshold: None`):

| DECAY | Half-life | Active | Ratio | Born | Split | Merge | Dormant | Revived | MembershipΔ | CoherenceShift |
|-------|-----------|--------|-------|------|-------|-------|---------|---------|-------------|----------------|
| 0.50  | 1 month   | 294    | 0.18× | 364  | 26    | 39    | **23**  | **4**   | 60          | 20             |
| 0.90  | 7 months  | 256    | 0.15× | 310  | 29    | 31    | 7       | 0       | 66          | 36             |
| 0.98  | 34 months | 252    | 0.15× | 305  | 30    | 30    | 7       | 0       | 62          | 36             |

Workload at 24 months: 1,674 papers, 3,279 citation edges, 23 non-empty batches.

### 3a — Engine contract holds

All three configs produce `active/node ratio` ≤ 0.18×. Compare EU email
(the failure case): `active/node = 14.8×` at 115 weeks.

This is the first real-dataset confirmation that the engine's
gradual-evolution assumption is not merely a design abstraction — on
accumulative temporal data, entity count stays **below** node count
(entities represent multi-paper subfields).

### 3b — First natural exercise of MembershipDelta and CoherenceShift

Both transitions were flagged as "dead weight" in Finding 2a (LFR):
`MembershipDelta` fired ≤ 1× across 6 LFR tests; `CoherenceShift` never
fired. Enron fired `CoherenceShift = 1` (Finding 3) on the 5-phase chain.

HEP-PH fires them at scale in every 24-month run:

- `MembershipDelta`: 60–66 events (entity members drift as new citations
  reshape subfield overlap)
- `CoherenceShift`: 20–36 events (entity coherence gate triggered by
  accumulation of strong intra-subfield citations)

**Implication**: both transitions were suspected of being vestigial.
HEP-PH confirms they fire on real accumulative data. Do not demote.

### 3c — Revived first natural exercise outside Enron

LFR: 0 Revived. Enron synthetic: 1 Revived (planted phase-4). HEP-PH
DECAY=0.5: **4 Revived** — the first uncurated dataset where the
transition fires on its own. Mechanism: short half-life (1 month) lets
entities fall Dormant, and strong future citation bursts re-match.

### 3d — Auto-threshold confirmed on sixth dataset class (accumulative)

Auto-threshold picks a sensible cut across all three DECAY values.
Median activity spans 0.01 (fast decay) to 5.38 (very slow), yet the
gap-detector identifies a workable threshold in each regime. The Ω2
demotion (`min_activity_threshold` → private) is not affected.

---

## 4. Results — 60-month window (stress region)

Two DECAY settings, auto-threshold. Both hit the
`HEP_PH_MAX_ENTITIES=30000` guard before completing the window.

| DECAY | Active@month 48 | Ratio | Born | Split | Merge | Dormant | Entity members |
|-------|-----------------|-------|------|-------|-------|---------|----------------|
| 0.50  | 45,814          | 4.40× | 55,558 | 9,225 | 454 | 146     | 7,685,572      |
| 0.90  | 37,815          | 3.63× | 46,829 | 8,289 | 713 | 76      | 10,831,513     |

Workload at 60 months: 10,408 papers, 61,608 edges, 59 non-empty batches.

### 4a — Growth trajectory

DECAY=0.9 checkpoint series:

| Month | Active | Rels | median_act |
|-------|--------|------|------------|
| 6     | 13     | 25     | 1.80  |
| 12    | 56     | 202    | 1.80  |
| 18    | 132    | 1,130  | 2.70  |
| 24    | 256    | 3,273  | 2.39  |
| 30    | 547    | 7,251  | 2.32  |
| 36    | 2,072  | 12,479 | 1.94  |
| 42    | 9,303  | 21,434 | 1.83  |
| 48    | 37,815 | 31,742 | 1.53  |

Active entity count is stable and sub-linear up to month 30 (547 / 4K
nodes), then super-linear from month 36 (2,072 → 9,303 → 37,815 ≈ 4×
per 6-month step).

### 4b — Root cause: hub-node multi-entity membership

The striking number is `Entity members total = 10,831,513` for DECAY=0.9
at 37,815 active entities over 10,408 underlying nodes. Average: each
node is a member of **~1,041 active entities** (total_members / n_nodes).

This is a new failure mode, distinct from EU email. After the
`overlap_threshold` knob was removed (Phase 1 of the complexity sweep,
`docs/complexity-audit.md`), entity-community matching became locus-flow
based — each component's members are assigned to existing entities by
bucket-majority, without a hard exclusivity constraint. On accumulative
citation data:

- High-degree papers (survey articles, seminal results) accumulate
  citations across many subfields.
- Each subfield forms a community; the hub paper lands in the "significant
  bucket" of each.
- Locus-flow matcher says "this entity's members are in community X" for
  many X simultaneously. Instead of forcing exclusivity, a new entity
  `Born` fires for each subfield that doesn't fully overlap an existing
  one, and the hub paper gets re-listed as a member.
- Over time, membership count grows super-linearly in node count.

### 4c — Distinct from EU email failure mode

| Property | EU email (Ω4) | HEP-PH 60m (Ω5) |
|----------|---------------|-----------------|
| Data character | Churn (weekly turnover) | Accumulation (permanent links) |
| Born source | New communities vs. prior week | Hub re-labelled into new subfield |
| Relationship count trajectory | Stable (most decay to ~0) | Monotone growth |
| Member churn | High | Low per-entity, high cross-entity |
| Fix lever | Activity half-life calibration | Membership exclusivity / hub cap |

The two failure modes require different remediations. They should not
be conflated into "the engine fails on real data."

---

## 5. Dataset queue position

HEP-PH is the **sixth real dataset**, first accumulative temporal. It
plays two roles:

1. **Contract confirmation** on its designed regime (24m, ratio 0.15×)
2. **New failure class** identified at scale (60m, hub-membership blowup)

Previous datasets tested community detection on stable snapshots
(karate, Davis) or curated dynamic schedules (LFR, Enron). SocioPatterns
and EU email tested temporal regimes with weekly/daily cadence. HEP-PH
is the first to expose the **accumulation axis** as an independent
stress vector.

---

## 6. Implications for the roadmap

### Does not block Ω2 demotion
`min_activity_threshold` / `min_bridge_activity` Ω2 demotion (2026-04-19)
remains correct. Auto-threshold worked across all three DECAY values on
HEP-PH 24m. The 60m failure is a membership-matcher issue, not a threshold
issue.

### Does not invalidate `overlap_threshold` removal (Phase 1)
Phase 1 removed `overlap_threshold` for a clear reason: karate-tuned
constant, evidence-based. HEP-PH 60m shows the locus-flow replacement has
an unbounded-membership edge case, but the fix is not "re-introduce
overlap_threshold." Candidate remediations:

- **Hub cap**: enforce per-locus max entity membership (e.g., 3–5
  entities/locus). Over-cap forces a merge proposal.
- **Entity identity dominance**: each locus declares a "primary"
  entity based on dominant flow; secondary memberships are weighted
  down and don't count towards significance.
- **Member decay**: expire members that haven't flowed into the entity's
  community in N batches.

### Suggests two `LayerTransition` survivors
`MembershipDelta` and `CoherenceShift` fire naturally on HEP-PH 24m. Before
HEP-PH they were on the demotion shortlist (Finding 2a). Remove from
shortlist.

---

## 7. Test harness

`crates/graph-engine/tests/hep_ph.rs` — three `#[ignore]` tests (require
`data/cit-HepPh.txt` and `data/cit-HepPh-dates.txt`):

- `hep_ph_slow_decay_auto` — DECAY=0.9 (default)
- `hep_ph_fast_decay_auto` — DECAY=0.5 (first Revived)
- `hep_ph_very_slow_decay_auto` — DECAY=0.98 (accumulation ceiling)

Each test honours `HEP_PH_MAX_MONTHS` (default 24) and
`HEP_PH_MAX_ENTITIES` (default 50,000). Recommended invocations:

```bash
# Contract region (sane)
HEP_PH_MAX_MONTHS=24 cargo test -p graph-engine --release --test hep_ph \
    -- --ignored --nocapture hep_ph_slow_decay_auto

# Stress region (documents failure)
HEP_PH_MAX_MONTHS=60 HEP_PH_MAX_ENTITIES=50000 \
  cargo test -p graph-engine --release --test hep_ph \
    -- --ignored --nocapture hep_ph_slow_decay_auto
```

Full-window (122 months) is not recommended without raising the entity
guard; expect hundreds of thousands of entities and multi-GB RSS.
