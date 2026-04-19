# Synapse-closer Engine + Small LLM — Experiment Plan

**Status**: Planning. Not yet started.
**Intended repo**: A copy of this repository (`dynamic_graph_db`) forked
for this experiment. Root engine work continues in the original.
**Planned start commit (original repo)**: `036480c`
(post-Ω5 fixpoint fix + post-validation).
**Budget**: ~3–5 weeks if all three phases run. Phase 1 alone is
~1–2 weeks and serves as the primary go/no-go signal.

---

## 1. Motivation

This plan is the output of a design conversation that mapped the engine
onto **neural substrate** concepts and asked whether its principles
transfer to LLM training / inference as more than a conceptual analogy.

Two observations anchor the experiment:

1. **The engine is structurally homologous to a synaptic substrate.**
   The analogy is not surface-level: Hebbian weight rule, activity
   decay, sediment layering, structural plasticity (Born / Split / Merge
   / Delete), Hebbian-only local learning, declared-vs-observed
   boundary tension — these all correspond to properties of neural
   substrates that LLMs, as static-weight forward-pass machines, do not
   natively carry.

2. **The analogy suggests an architecture, not just a metaphor.**
   If engine ≈ synaptic substrate and LLM inference ≈ electrical
   signalling, then a composed system gets two things current LLMs
   struggle with: **persistent experiential state** (via sediment +
   weathering + entity lifecycle) and **structural self-correction**
   (via `recognize_entities` fixpoint convergence + boundary-tension
   analysis). Current approaches (RAG, MemGPT, long-context hacks)
   simulate this imperfectly by grafting retrieval onto stateless LLMs.

**The experiment is the test of whether that architectural intuition is
empirically meaningful** or just pretty.

Full design-conversation context is in the original repo's session log.
For a written summary of the engine-↔-synapse mapping, see this
document §3.

---

## 2. Hypotheses

Primary (measurable):
**H1.** A small LLM (nano-GPT scale) paired with engine-as-memory
produces **lower perplexity** and/or **higher long-horizon entity
consistency** than the same LLM without the engine on the same corpus.

Secondary (informative regardless of outcome):
**H2.** Among the synaptic additions (STDP, metaplasticity, enhanced
inhibitory support), at most 1–2 are load-bearing for LLM-coupled
tasks. Ablation can name which.

**H3.** The `graph-boundary` declared-vs-observed tension produces a
non-trivial signal during LLM training — e.g., hallucination-likelihood
indicator or a loss modulation.

**Negative result protocol**: if H1 is flat within 2σ across 3+ seeds,
stop at Phase 2 and document as "engine-as-memory shows no advantage at
nano-GPT scale". This is still publishable / valuable.

---

## 3. Three-phase structure

### Phase 1 — Engine synapse-closer upgrade (1–2 weeks)

**Goal**: Bring the engine closer to a neural substrate where
neuroscience literature suggests it matters, without breaking the
current production crate.

**Isolation strategy**:
- Option A: Separate crate `graph-engine-neuro` that depends on
  `graph-engine` and adds the new surface. Preferred — keeps the
  14-knob core clean.
- Option B: Feature flag `"neuro"` on `graph-engine`. Simpler but
  conflates two audiences.
- Decide in the first commit.

**Features to add (in order of evidence strength)**:

1. **STDP (Spike-Timing-Dependent Plasticity)** — reintroduce the
   asymmetric LTP/LTD rule removed in Phase 3 of the complexity audit
   (`docs/complexity-audit.md § Phase 3`). This time, gate it behind a
   per-kind flag so the core remains Hebbian-only. Neuroscience: well
   established; the likeliest load-bearing addition.

2. **Metaplasticity** — per-kind `learning_rate` becomes itself
   observation-driven. A slow plasticity meta-loop (BCM-like sliding
   threshold, or simpler: scale `η` by recent-weight-variance). The
   Phase 9 `PlasticityLearners` primitive in `crates/graph-engine/src/
   plasticity.rs` is the natural hook.

3. **Inhibitory kind as primary citizen** — currently expressible via
   `activity_contribution < 0` but not first-class. Add
   `InfluenceKindConfig::inhibitory: bool` plus required semantics
   (e.g., inhibitory kinds suppress target activity, cannot trigger
   structural proposals, have distinct decay).

4. **(optional) Neural oscillation / refractory enhancement** —
   `refractory_batches` already exists. Extend to a probabilistic
   per-locus refractory with phase coupling. **Skip unless Phase 1
   runway allows** — it compounds complexity quickly.

**Stop/go checkpoint after Phase 1**:
- Regression: all 229 existing tests still pass (229/229).
- Benchmark: HEP-PH 122m, EU email 115w, SocioPatterns, Enron remain
  sane (within ±5% of pre-upgrade entity counts; no explosion).
- New primitive smoke tests (at minimum 1 unit test per new feature).
- Small narrative test: STDP shifts entity lifecycle timing on
  existing HEP-PH trajectory in a documented way (e.g., Dormant/
  Revived rate changes by >20%).

If any benchmark explodes or regression breaks → fix or roll back
before Phase 2. If smoke tests pass but no narrative signal shows up,
**pause here** — Phase 2 likely won't find signal either.

### Phase 2 — LLM integration (1–2 weeks)

**Goal**: Wire a nano-GPT-scale LLM to use the engine as an external
memory substrate.

**Architecture choice — Option C (memory layer, engine not learned)**:

```
Input text
    ↓
Tokenizer
    ↓
nano-GPT forward pass ←─── engine-state feature vector
    ↓                        ↑
Output tokens                │
    ↓                        │
Event extractor ─────────────┘
    ↓
Engine tick (decay, recognize_entities, boundary analysis)
    ↓
Updated engine state (next iteration)
```

Key design choices (confirmed):
- **Engine is fixed, not learned**. Engine ops (Born/Split/Merge) are
  non-differentiable; trying to backprop through them requires RL-style
  updates and blurs the experiment's interpretation. Keep the engine
  as a principled state-machine tool; the LLM learns to use it.
- **Engine-state → feature vector** is the only information
  channel LLM → engine-state (no direct parameter coupling).
- **Event extraction from LLM output** happens outside the forward
  pass. A light rule-based extractor initially (entity mentions,
  sentiment polarity), upgradable to a learned head later.

**Concrete artifact**: `crates/graph-llm-nano/` crate that wraps nano-GPT
(Rust port or Python-bridged via PyO3). Includes:
- A `EngineMemoryAdapter` trait that exposes engine state as a
  fixed-length feature vector (entity counts, recent-change summary,
  active-boundary-tension scalar, etc.).
- An `EventExtractor` that parses LLM output → `ProposedChange`s.
- A training loop wrapper that alternates LLM forward/backward with
  engine ticks.

**Stop/go checkpoint after Phase 2**:
- LLM-with-engine and LLM-alone both train to reasonable perplexity on
  Shakespeare corpus (< 2.0 bpc at nano-GPT capacity, matching
  published nano-GPT numbers).
- Adapter overhead < 2× training step time.
- Engine state remains sane across training (no entity explosion,
  `Δidempotent = +0` at inspection points).

If LLM-with-engine cannot match LLM-alone perplexity, investigate
adapter before Phase 3. If engine state explodes during training,
document as finding and decide whether to continue.

### Phase 3 — Training experiment + ablation (1 week)

**Goal**: Measure H1 / H2 / H3.

**Experimental design**:

- **Model**: nano-GPT (char-level, ~10M params). A 100M-param fallback
  if nano-GPT is too small to exercise long-context effects.
- **Corpus**:
  - Primary: **TinyStories** (Eldan & Li, 2023) — synthetic short
    narratives with clear named-entity structure, ideal for entity
    consistency measurement.
  - Secondary: **Shakespeare** (nano-GPT default) for parity with
    published baselines.
- **Training**: Same optimizer, same seed sweep (3 seeds), same data
  schedule. Only variable: engine coupling on/off.
- **Engine-coupled variants (ablation)**:
  1. Engine with all Phase 1 additions (STDP + metaplasticity +
     inhibitory).
  2. Engine with STDP only.
  3. Engine with metaplasticity only.
  4. Engine baseline (current Ω5 state, no Phase 1 additions).
  5. No engine (control).
- **Metrics**:
  - Perplexity / bits-per-character on held-out corpus.
  - **Entity consistency score**: In TinyStories, for each story,
    extract named entities at position 25%, 50%, 75%, 100% of the
    story. Count consistency (same-entity references agree on gender,
    role, actions). Engine is expected to help here if anywhere.
  - **Boundary flag rate**: Across a hallucination-probe dataset,
    count how often the engine's boundary analysis flags LLM output
    as Ghost (declared but not observed in substrate). If this
    correlates with human-judged hallucination, H3 is supported.
  - Wall-clock training cost.
- **Reporting**: Include the negative-result protocol. Publish all
  ablations regardless of primary-metric outcome.

**Stop/go checkpoint after Phase 3**:
- If H1 holds: write up and identify next-scale experiment
  (100M-1B params).
- If H1 is flat but H3 shows signal: the boundary-tension work is
  valuable independently; plan a Track-J extension.
- If all hypotheses flat: document why and what would need to change
  (scale? task type? adapter design?).

---

## 4. Key decisions already made

These are locked in at plan time. Changing any of them requires
re-planning.

| Decision | Choice | Why |
|----------|--------|-----|
| Isolation | Separate branch + separate repo | Original engine stays clean; experiment can fail without cost |
| Engine differentiability | Fixed, not learned | Non-differentiable Born/Split/Merge + clean interpretation |
| Integration pattern | Memory layer (Option C) | Tool-use-style coupling; most realistic for current LLM ecosystem |
| LLM scale | nano-GPT first, 100M fallback | Cheap, measurable, literature baseline available |
| Corpus | TinyStories primary, Shakespeare secondary | Strong entity structure for the metric that actually differentiates |
| Feature-crate vs fork | Separate `graph-engine-neuro` crate | Semantic clarity; avoids feature-flag coupling |

---

## 5. Open questions for new-session start

These have been deliberately **not** answered yet. The new session
should address them in the first work blocks:

1. **STDP window parameters**: `τ+` / `τ−` / amplitudes. Literature
   default is `τ = 20ms` but that unit doesn't translate to our
   batch-indexed time. Propose a scheme: `τ` expressed in batches,
   defaulted to 1–3 batches, overridable per kind.

2. **Metaplasticity meta-loop rate**: must be much slower than
   base-plasticity rate (BCM suggests ≥10× slower). How exactly?
   Target: avoid oscillation.

3. **Inhibitory semantics boundary**: does an inhibitory kind trigger
   entity emergence? Probably no — suppresses, doesn't create. Define
   the rule before implementing.

4. **Engine-state feature vector schema**: which scalars? Candidates:
   active-entity count, recent Born/Dormant rate, coherence histogram
   summary, boundary-tension scalar. Propose 4–8 scalars, justify
   each.

5. **Event extractor implementation**: rule-based first. Keyword:
   KISS. A small learned head comes later if needed.

6. **Training-time engine tick cadence**: every token? every N tokens?
   every batch? Probably per-batch for nano-GPT; monitor overhead.

---

## 6. Starting steps for new session

On the new repository's first work session, do these in order:

1. **Read context**: read this file, then `CLAUDE.md`, then
   `docs/roadmap.md § 2 Track Ω`, then
   `docs/complexity-audit.md § Finding 5`. The fixpoint fix and the
   `graph-boundary` primitive are load-bearing for the experiment;
   understand them before starting Phase 1.

2. **Create scaffolding**:
   - Create `crates/graph-engine-neuro/` crate.
   - Add a feature-gated STDP config alongside `PlasticityConfig`.
   - Wire a single smoke test showing STDP-on changes weight evolution
     on a 2-locus pair vs Hebbian-only baseline.

3. **First commit**: empty-scaffold structure + smoke test. This lets
   the branch compile and be visible to regression before any real
   work starts.

4. **Phase 1 work**: proceed in the order listed in §3. Every feature
   lands with a unit test. After all three (STDP / metaplasticity /
   inhibitory) are in, run the existing benchmark suite
   (Karate/Davis/LFR/SocioPatterns/Enron/EU email/HEP-PH) and produce
   a comparison document.

5. **Phase 1 stop/go**: look at the comparison. Does any benchmark
   show documented improvement on a metric that matters? If yes:
   Phase 2. If no: write up and stop.

---

## 7. Anti-goals

Holding these explicit so the new session doesn't drift:

- **Not a transformer reimplementation.** nano-GPT stays stock.
  Architectural changes to the LLM are out of scope.
- **Not a claim about general LLM improvement.** The experiment is
  about a specific scale, specific corpus, specific adapter design.
  Generalisation is future work.
- **Not an alignment claim.** Boundary-tension analysis may inform
  hallucination measurement, but "engine makes LLM aligned" is
  unsupported and should not appear in any writeup.
- **Not a neuroscience paper.** Biological fidelity is a design
  inspiration, not a success criterion. STDP and metaplasticity are
  included because they might help, not because they're biologically
  accurate to our substrate.

---

## 8. References (to fetch on new session)

- `docs/redesign.md` — substrate ontology (authoritative framing).
- `docs/complexity-audit.md` — especially Finding 5 (fixpoint),
  Phases 1–3 (for what was tried and removed before).
- `docs/hep-ph-finding.md` — the investigation that led to the
  non-idempotency fix. The fixpoint loop is the most important engine
  property for this experiment.
- `docs/eu-email-finding.md` — the "churn amplifies bugs" lesson.
  The new experiment's long-horizon training corpus will stress
  similar dynamics.
- `CLAUDE.md` — "Feature removal policy" applies. Do not demote any
  of the seven `LayerTransition` variants based on Phase-1 ablation
  alone; the three-axis diversity rule binds this experiment too.
- Original-repo commit `036480c`.

---

**Change log**
- 2026-04-19 — Plan created. Original engine project at commit
  `036480c`. Work to continue in a separate repository fork.
