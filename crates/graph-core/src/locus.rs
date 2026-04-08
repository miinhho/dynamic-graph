//! Layer 0: the locus primitive.
//!
//! A locus is a labeled position in the substrate. It owns state and runs
//! a program whenever changes touch it. It is *not* the user's mental
//! "entity" — those are recognized later by the emergence layer.
//!
//! Per `docs/redesign.md` §3.1, a locus has identity, a kind tag, and
//! state. The program lives separately in a registry keyed by
//! `LocusKindId` (so multiple loci sharing a kind share a single program
//! implementation).

use crate::ids::{LocusId, LocusKindId};
use crate::state::StateVector;

#[derive(Debug, Clone, PartialEq)]
pub struct Locus {
    pub id: LocusId,
    pub kind: LocusKindId,
    pub state: StateVector,
}

impl Locus {
    pub fn new(id: LocusId, kind: LocusKindId, state: StateVector) -> Self {
        Self { id, kind, state }
    }
}
