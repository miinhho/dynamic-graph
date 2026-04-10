//! Read-only graph traversal and query operations over a `World`.
//!
//! This crate sits above `graph-world` (which owns storage and mutation)
//! and provides the traversal layer: shortest paths, reachability, and
//! connected components. It never mutates the world.
//!
//! ## API
//!
//! All public functions take `&World` as their first argument:
//!
//! ```ignore
//! use graph_query::{path_between, reachable_from, connected_components};
//!
//! let path = path_between(&world, from, to);
//! let nearby = reachable_from(&world, start, 2);
//! let groups = connected_components(&world);
//! ```
//!
//! Kind-filtered variants (`_of_kind`) restrict traversal to relationships
//! of a specific `RelationshipKindId`.

mod traversal;

pub use traversal::{
    connected_components, connected_components_of_kind, path_between, path_between_of_kind,
    reachable_from, reachable_from_of_kind,
};
