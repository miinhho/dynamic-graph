//! State- and property-based filtering of loci and relationships.
//!
//! All functions take `&World` and return `Vec` of references valid for
//! the lifetime of the world borrow.

mod entities;
mod entity_bridge;
mod loci;
mod lookup;
mod metrics;
mod relationships;

pub use self::entities::*;
pub use self::loci::*;
pub use self::lookup::{lookup_loci, lookup_relationships};
pub use self::relationships::*;

#[cfg(test)]
mod tests;
