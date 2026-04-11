//! Supply-chain disruption example.
//!
//! A domain-level dogfood that exercises five features from the recent
//! engine iteration:
//!
//! 1. **`changes_of_kind`** (inbox filtering): programs separate ORDER
//!    from FAILURE signals without manual per-change kind checks.
//!
//! 2. **`max_proposals_per_dispatch`**: the factory's per-dispatch cap
//!    prevents fan-out explosion in topologies with many downstream nodes.
//!
//! 3. **`SubscribeToRelationship`** + **`ChangeSubject::Relationship`**:
//!    an analyst locus watches the supply edges. The factory explicitly
//!    updates each edge's "reliability" slot as goods arrive; the analyst
//!    receives those relationship-subject changes via subscription.
//!
//! 4. **`DeleteLocus`**: a supplier failure is modelled by a FAILURE_KIND
//!    stimulus. The supplier's `structural_proposals` emits
//!    `DeleteLocus(locus.id)` on receiving it, atomically removing the
//!    locus, its relationships, and the analyst's subscription at
//!    end-of-batch.
//!
//! 5. **Extra relationship slots**: the SUPPLY kind carries a user-defined
//!    "reliability" slot (index 2) that the factory increments on each
//!    successful delivery and that decays slowly between ticks.  Activity
//!    (slot 0) is managed automatically by the engine's auto-emerge path.
//!
//! Topology:
//!
//! ```text
//!   SUPPLIER_A ──supply──→ FACTORY ──output──→ WAREHOUSE
//!   SUPPLIER_B ──supply──↗
//!
//!   ANALYST subscribed to SUPPLIER_A→FACTORY and SUPPLIER_B→FACTORY edges
//!
//!   (after tick 2: SUPPLIER_B sends failure signal → self-deletes →
//!    its edge and the analyst's subscription to it vanish atomically)
//! ```
//!
//! Run: `cargo run -p graph-engine --example supply_chain`

use graph_core::{
    changes_of_kind, relationship_changes_of_kind, Change, ChangeSubject, Endpoints,
    InfluenceKindId, Locus, LocusContext, LocusId, LocusKindId, LocusProgram, ProposedChange,
    RelationshipId, RelationshipSlotDef, StateVector, StructuralProposal,
};
use graph_engine::{
    DefaultCoherePerspective, DefaultEmergencePerspective, Engine, EngineConfig,
    InfluenceKindConfig, InfluenceKindRegistry, LocusKindConfig, LocusKindRegistry,
};
use graph_world::World;

// ── Kind constants ────────────────────────────────────────────────────────────

const KIND_SUPPLIER:  LocusKindId = LocusKindId(1);
const KIND_FACTORY:   LocusKindId = LocusKindId(2);
const KIND_WAREHOUSE: LocusKindId = LocusKindId(3);
const KIND_ANALYST:   LocusKindId = LocusKindId(4);

/// External order placed with a supplier (trigger only; no decay needed).
const ORDER_KIND:   InfluenceKindId = InfluenceKindId(1);
/// Goods / raw materials in transit — carries a "reliability" extra slot.
const SUPPLY_KIND:  InfluenceKindId = InfluenceKindId(2);
/// Failure signal: instructs a supplier to remove itself from the world.
const FAILURE_KIND: InfluenceKindId = InfluenceKindId(3);

// ── Locus IDs ─────────────────────────────────────────────────────────────────

const SUPPLIER_A: LocusId = LocusId(1);
const SUPPLIER_B: LocusId = LocusId(2);
const FACTORY:    LocusId = LocusId(3);
const WAREHOUSE:  LocusId = LocusId(4);
const ANALYST:    LocusId = LocusId(5);

/// Index of the user-defined "reliability" slot in a SUPPLY_KIND relationship.
/// Built-in slots occupy indices 0 (activity) and 1 (weight); extras start at 2.
const RELIABILITY_SLOT: usize = 2;

// ── Programs ──────────────────────────────────────────────────────────────────

/// Raw-material supplier.
///
/// - On `ORDER_KIND` stimuli: uses `changes_of_kind` to filter order events,
///   then forwards a `SUPPLY_KIND` delivery (quantity = order count) to the
///   factory.
/// - On `FAILURE_KIND` signal: proposes `DeleteLocus(self)` in
///   `structural_proposals`, atomically removing the supplier, its
///   relationships, and all subscriptions at end-of-batch.
struct SupplierProgram {
    factory: LocusId,
}

impl LocusProgram for SupplierProgram {
    fn process(
        &self,
        _locus: &Locus,
        incoming: &[&Change],
        _ctx: &dyn LocusContext,
    ) -> Vec<ProposedChange> {
        let orders = changes_of_kind(incoming, ORDER_KIND);
        if orders.is_empty() {
            return vec![];
        }
        let quantity = orders.len() as f32;
        vec![ProposedChange::new(
            ChangeSubject::Locus(self.factory),
            SUPPLY_KIND,
            StateVector::from_slice(&[quantity]),
        )]
    }

    fn structural_proposals(
        &self,
        locus: &Locus,
        incoming: &[&Change],
        _ctx: &dyn LocusContext,
    ) -> Vec<StructuralProposal> {
        // A FAILURE_KIND signal triggers self-removal. The engine will
        // clean up all touching relationships (and their subscriptions)
        // atomically at end-of-batch via DeleteLocus.
        if changes_of_kind(incoming, FAILURE_KIND).is_empty() {
            return vec![];
        }
        println!("    [SUPPLIER L{}] failure signal received — proposing DeleteLocus", locus.id.0);
        vec![StructuralProposal::delete_locus(locus.id)]
    }
}

/// Manufacturing plant.
///
/// On `SUPPLY_KIND` deliveries from suppliers:
///   1. Tallies total received material.
///   2. For each incoming supply relationship found via `ctx`, bumps the
///      "reliability" extra slot (+0.1 per successful delivery). This emits
///      a `ChangeSubject::Relationship` change that the analyst receives via
///      subscription.
///   3. Reads the warehouse's current stock via `ctx` and sends a cumulative
///      total (`current_stock + output`) rather than a raw delta. This lets
///      the warehouse program return empty and quiesce immediately, avoiding
///      self-reinforcing update loops.
///
/// `max_proposals_per_dispatch = 5` caps fan-out: 1 per supplier edge
/// (reliability update) + 1 to warehouse = 3 in this topology.  In a
/// network with many downstream consumers the cap prevents cascades.
struct FactoryProgram {
    warehouse: LocusId,
    /// Output per unit of received raw material.
    efficiency: f32,
}

impl LocusProgram for FactoryProgram {
    fn process(
        &self,
        locus: &Locus,
        incoming: &[&Change],
        ctx: &dyn LocusContext,
    ) -> Vec<ProposedChange> {
        let deliveries = changes_of_kind(incoming, SUPPLY_KIND);
        if deliveries.is_empty() {
            return vec![];
        }

        let total_supply: f32 = deliveries
            .iter()
            .flat_map(|c| c.after.as_slice())
            .copied()
            .sum();

        let mut proposals: Vec<ProposedChange> = Vec::new();

        // Bump the "reliability" slot on each incoming supply relationship.
        // `incoming_relationships_of_kind` filters to Directed edges arriving at
        // this factory, avoiding the manual endpoint-match pattern.
        // `relationship_patch` applies an additive delta to slot 2 only —
        // Hebbian weight (slot 1) and activity (slot 0) are left untouched.
        for rel in ctx.incoming_relationships_of_kind(locus.id, SUPPLY_KIND) {
            proposals.push(ProposedChange::relationship_patch(
                rel.id,
                SUPPLY_KIND,
                &[(RELIABILITY_SLOT, 0.1)],
            ));
        }

        // Send cumulative stock to warehouse rather than a raw delta.
        // Reading current stock via ctx and adding output means the engine
        // writes the total to warehouse.state and the warehouse program
        // can return empty — no follow-up loop.
        let current_stock = ctx
            .locus(self.warehouse)
            .and_then(|l| l.state.as_slice().first().copied())
            .unwrap_or(0.0);
        let output = total_supply * self.efficiency;
        proposals.push(ProposedChange::new(
            ChangeSubject::Locus(self.warehouse),
            SUPPLY_KIND,
            StateVector::from_slice(&[current_stock + output]),
        ));

        proposals
    }
}

/// Finished-goods warehouse.
///
/// The factory already sends the cumulative stock (`prior + output`) as the
/// proposed state, so the committed change updates `warehouse.state` directly.
/// The warehouse program returns empty and the batch quiesces immediately —
/// no self-reinforcing update loop.
struct WarehouseProgram;

impl LocusProgram for WarehouseProgram {
    fn process(
        &self,
        _locus: &Locus,
        _incoming: &[&Change],
        _ctx: &dyn LocusContext,
    ) -> Vec<ProposedChange> {
        // State is already updated by the factory's committed change.
        // Returning empty here quiesces the batch loop.
        vec![]
    }
}

/// Supply-chain risk analyst.
///
/// Subscribed to the supply edges for the factory (wired before the first
/// tick via `SubscriptionStore::subscribe_at`). When the factory updates
/// a supply edge's "reliability" slot, this locus receives the
/// `ChangeSubject::Relationship` change in its inbox.
///
/// Uses `changes_of_kind(SUPPLY_KIND)` to separate supply-edge updates
/// from any other inbox entries, then logs per-edge reliability scores.
/// Produces no follow-up proposals — it is a pure observer.
struct AnalystProgram {
    sup_a_rel: RelationshipId,
    sup_b_rel: RelationshipId,
}

impl LocusProgram for AnalystProgram {
    fn process(
        &self,
        _locus: &Locus,
        incoming: &[&Change],
        _ctx: &dyn LocusContext,
    ) -> Vec<ProposedChange> {
        // `relationship_changes_of_kind` separates relationship-subscription
        // notifications from locus-originated signals and narrows to
        // SUPPLY_KIND edges in a single pass.
        for change in relationship_changes_of_kind(incoming, SUPPLY_KIND) {
            let edge_label = match change.subject {
                ChangeSubject::Relationship(rid) if rid == self.sup_a_rel => "SUPPLIER_A→FACTORY",
                ChangeSubject::Relationship(rid) if rid == self.sup_b_rel => "SUPPLIER_B→FACTORY",
                _ => "(unknown edge)",
            };
            let reliability = change.after.as_slice().get(RELIABILITY_SLOT).copied().unwrap_or(0.0);
            let activity   = change.after.as_slice().first().copied().unwrap_or(0.0);
            println!(
                "    [ANALYST] {edge_label:<26} reliability={reliability:.3}  activity={activity:.3}"
            );
        }
        vec![]
    }
}

// ── World construction ────────────────────────────────────────────────────────

struct Setup {
    world:      World,
    loci:       LocusKindRegistry,
    influences: InfluenceKindRegistry,
    sup_a_rel:  RelationshipId,
    sup_b_rel:  RelationshipId,
}

fn build_world() -> Setup {
    // Build registries first so `initial_state_for` can seed relationship
    // state from the kind config's extra-slot defaults rather than hard-coding
    // the slot count.
    let mut loci = LocusKindRegistry::new();
    loci.insert(KIND_SUPPLIER, Box::new(SupplierProgram { factory: FACTORY }));
    // Factory: max 5 proposals per dispatch caps fan-out.
    loci.insert_with_config(KIND_FACTORY, LocusKindConfig {
        program: Box::new(FactoryProgram { warehouse: WAREHOUSE, efficiency: 0.8 }),
        refractory_batches: 0,
        encoder: None,
        max_proposals_per_dispatch: Some(5),
    });
    loci.insert(KIND_WAREHOUSE, Box::new(WarehouseProgram));

    let mut influences = InfluenceKindRegistry::new();
    influences.insert(
        ORDER_KIND,
        InfluenceKindConfig::new("order").with_decay(1.0),
    );
    influences.insert(
        SUPPLY_KIND,
        InfluenceKindConfig::new("supply")
            .with_decay(0.90)
            .with_extra_slots(vec![
                // "reliability" accumulates (+0.1) on each successful delivery
                // and erodes slowly between deliveries (decay=0.995/batch).
                // The factory updates this slot via ChangeSubject::Relationship;
                // Hebbian plasticity is intentionally omitted here because
                // relationship state changes from programs (which read the
                // state at dispatch time) would overwrite Hebbian-applied
                // weight updates added at end-of-batch. The sensor_fusion
                // example demonstrates Hebbian separately.
                RelationshipSlotDef::new("reliability", 0.0).with_decay(0.995),
            ]),
    );
    influences.insert(
        FAILURE_KIND,
        InfluenceKindConfig::new("failure").with_decay(1.0),
    );

    let mut world = World::new();

    for id in [SUPPLIER_A, SUPPLIER_B] {
        world.insert_locus(Locus::new(id, KIND_SUPPLIER, StateVector::zeros(1)));
    }
    world.insert_locus(Locus::new(FACTORY,   KIND_FACTORY,   StateVector::zeros(1)));
    world.insert_locus(Locus::new(WAREHOUSE, KIND_WAREHOUSE, StateVector::zeros(1)));
    world.insert_locus(Locus::new(ANALYST,   KIND_ANALYST,   StateVector::zeros(1)));

    // Pre-create supply relationships. `initial_state_for` derives the correct
    // slot count ([activity, weight, reliability]) from the registered config,
    // so the state is consistent with any extra-slot additions to SUPPLY_KIND.
    let initial_supply_state = influences.initial_state_for(SUPPLY_KIND);
    let sup_a_rel = world.add_relationship(
        Endpoints::directed(SUPPLIER_A, FACTORY),
        SUPPLY_KIND,
        initial_supply_state.clone(),
    );
    let sup_b_rel = world.add_relationship(
        Endpoints::directed(SUPPLIER_B, FACTORY),
        SUPPLY_KIND,
        initial_supply_state,
    );

    // Analyst watches both supply edges from the start.
    world.subscriptions_mut().subscribe_at(ANALYST, sup_a_rel, None);
    world.subscriptions_mut().subscribe_at(ANALYST, sup_b_rel, None);

    loci.insert(KIND_ANALYST, Box::new(AnalystProgram { sup_a_rel, sup_b_rel }));

    Setup { world, loci, influences, sup_a_rel, sup_b_rel }
}

// ── Print helpers ─────────────────────────────────────────────────────────────

fn print_relationships(world: &World) {
    let mut rels: Vec<_> = world.relationships().iter().collect();
    rels.sort_by_key(|r| r.id);
    for r in rels {
        let (from, to) = match r.endpoints {
            Endpoints::Directed { from, to } => (from, to),
            Endpoints::Symmetric { a, b } => (a, b),
        };
        let reliability = r.state.as_slice().get(RELIABILITY_SLOT).copied().unwrap_or(0.0);
        println!(
            "  rel#{} L{}→L{}  activity={:.3}  weight={:.4}  reliability={:.3}  touches={}",
            r.id.0, from.0, to.0, r.activity(), r.weight(), reliability, r.lineage.change_count,
        );
    }
}

fn locus_label(id: LocusId) -> &'static str {
    match id {
        SUPPLIER_A => "SUPPLIER_A",
        SUPPLIER_B => "SUPPLIER_B",
        FACTORY    => "FACTORY",
        WAREHOUSE  => "WAREHOUSE",
        ANALYST    => "ANALYST",
        _          => "?",
    }
}

// ── Main ──────────────────────────────────────────────────────────────────────

fn main() {
    let Setup { mut world, loci, influences, sup_a_rel, sup_b_rel } = build_world();
    let engine = Engine::new(EngineConfig { max_batches_per_tick: 16 });

    println!("=== Supply Chain Disruption Example ===\n");
    println!("  SUPPLIER_A ──supply──→ FACTORY ──output──→ WAREHOUSE");
    println!("  SUPPLIER_B ──supply──↗  (fails in tick 2)");
    println!("  ANALYST subscribed to both supply edges\n");
    println!(
        "  supply decay=0.90  reliability_slot decay=0.995  factory efficiency=0.80  max_proposals=5\n"
    );

    // ── Tick 1: both suppliers active ─────────────────────────────────────────

    println!("--- Tick 1: orders to SUPPLIER_A and SUPPLIER_B ---");
    let r1 = engine.tick(
        &mut world,
        &loci,
        &influences,
        vec![
            ProposedChange::stimulus(SUPPLIER_A, ORDER_KIND, &[1.0]),
            ProposedChange::stimulus(SUPPLIER_B, ORDER_KIND, &[1.0]),
        ],
    );
    println!(
        "  batches={} changes={} relationships={}",
        r1.batches_committed, r1.changes_committed, world.relationships().len()
    );
    let stock = world.locus(WAREHOUSE).map(|l| l.state.as_slice()[0]).unwrap_or(0.0);
    println!("  warehouse stock: {stock:.3}");
    println!("  analyst subscriptions active: {}", world.subscriptions().subscription_count());
    print_relationships(&world);
    println!();

    // ── Tick 2: SUPPLIER_B receives failure signal → self-deletes ─────────────
    //
    // The FAILURE_KIND stimulus hits SUPPLIER_B. In `structural_proposals`,
    // SupplierProgram sees a FAILURE_KIND change in its inbox and emits
    // DeleteLocus(SUPPLIER_B). The engine applies this at end-of-batch:
    //   • SUPPLIER_B's locus is removed.
    //   • The SUPPLIER_B→FACTORY relationship is removed.
    //   • The analyst's subscription to that relationship is removed.
    //
    // SUPPLIER_A still receives its normal ORDER_KIND stimulus this tick.

    println!("--- Tick 2: SUPPLIER_B fails — SUPPLIER_A delivers alone ---");
    let r2 = engine.tick(
        &mut world,
        &loci,
        &influences,
        vec![
            ProposedChange::stimulus(SUPPLIER_A, ORDER_KIND,   &[1.0]),
            ProposedChange::stimulus(SUPPLIER_B, FAILURE_KIND, &[1.0]),
        ],
    );
    println!(
        "  batches={} changes={} relationships={}",
        r2.batches_committed, r2.changes_committed, world.relationships().len()
    );
    let stock = world.locus(WAREHOUSE).map(|l| l.state.as_slice()[0]).unwrap_or(0.0);
    println!("  warehouse stock: {stock:.3}");
    println!("  SUPPLIER_B exists: {}", world.locus(SUPPLIER_B).is_some());
    println!("  analyst subscriptions active: {} (SUPPLIER_B edge cleaned up)", world.subscriptions().subscription_count());
    print_relationships(&world);
    println!();

    // ── Tick 3: steady-state single supplier ──────────────────────────────────

    println!("--- Tick 3: single supplier, normal operation ---");
    let r3 = engine.tick(
        &mut world,
        &loci,
        &influences,
        vec![ProposedChange::stimulus(SUPPLIER_A, ORDER_KIND, &[1.0])],
    );
    println!(
        "  batches={} changes={} relationships={}",
        r3.batches_committed, r3.changes_committed, world.relationships().len()
    );
    let stock = world.locus(WAREHOUSE).map(|l| l.state.as_slice()[0]).unwrap_or(0.0);
    println!("  warehouse stock: {stock:.3}");
    print_relationships(&world);
    println!();

    // ── Relationship health after decay flush ─────────────────────────────────

    engine.flush_relationship_decay(&mut world, &influences);
    println!("--- Relationships after decay flush ---");
    print_relationships(&world);
    println!();

    // ── Entity recognition ─────────────────────────────────────────────────────

    let ep = DefaultEmergencePerspective {
        min_activity_threshold: 0.01,
        ..Default::default()
    };
    engine.recognize_entities(&mut world, &influences, &ep);
    println!("--- Entities ({} active) ---", world.entities().active_count());
    for e in world.entities().active() {
        let members: Vec<&str> = e.current.members.iter().map(|l| locus_label(*l)).collect();
        println!(
            "  entity#{} members=[{}] coherence={:.3} layers={}",
            e.id.0,
            members.join(", "),
            e.current.coherence,
            e.layer_count()
        );
    }
    println!();

    // ── Cohere clusters ────────────────────────────────────────────────────────

    let cp = DefaultCoherePerspective { min_bridge_activity: 0.01, ..Default::default() };
    engine.extract_cohere(&mut world, &influences, &cp);
    let coheres = world.coheres().get("default").unwrap_or(&[]);
    println!("--- Coheres ({}) ---", coheres.len());
    for c in coheres {
        let ms = match &c.members {
            graph_core::CohereMembers::Entities(ids) => {
                ids.iter().map(|e| format!("entity#{}", e.0)).collect::<Vec<_>>().join(", ")
            }
            _ => "(mixed)".to_string(),
        };
        println!("  cohere#{} [{}]  strength={:.3}", c.id.0, ms, c.strength);
    }
    if coheres.is_empty() {
        println!("  (none — single-supplier topology lacks bridging after disruption)");
    }
    println!();

    // ── Change log summary ─────────────────────────────────────────────────────
    // Show the final causal picture: number of committed changes per subject.

    println!("--- Change log summary ({} changes total) ---", world.log().len());
    let mut locus_changes = 0u32;
    let mut rel_changes = 0u32;
    for change in world.log().iter() {
        match change.subject {
            ChangeSubject::Locus(_)        => locus_changes += 1,
            ChangeSubject::Relationship(_) => rel_changes += 1,
        }
    }
    println!("  locus-subject changes:        {locus_changes}");
    println!("  relationship-subject changes: {rel_changes}  (analyst received these via subscription)");
    println!();
    println!("  {sup_a_rel:?} = SUPPLIER_A→FACTORY  (surviving edge)");
    println!("  {sup_b_rel:?} = SUPPLIER_B→FACTORY  (deleted in tick 2)");

    println!("\nDone.");
}
