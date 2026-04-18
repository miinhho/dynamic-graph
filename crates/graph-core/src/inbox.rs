//! Helpers for processing a locus program's incoming change slice.
//!
//! `LocusProgram::process` receives `incoming: &[&Change]` — all changes
//! committed in the current batch that targeted the locus. Two patterns
//! come up constantly:
//!
//! 1. **Filter by influence kind** — programs that handle multiple kinds
//!    need to route changes to different handlers. `of_kind` provides a
//!    zero-allocation iterator for that.
//!
//! 2. **Sum the first state slot** — most single-slot locus programs want
//!    the total "activation" arriving this batch. `locus_signals` sums
//!    `change.after[0]` for every locus-subject change in the inbox.
//!
//! # Examples
//!
//! ```rust,ignore
//! use graph_core::inbox::{of_kind, locus_signals};
//!
//! fn process(&self, locus: &Locus, incoming: &[&Change], ctx: &dyn LocusContext)
//!     -> Vec<ProposedChange>
//! {
//!     let excite: f32 = locus_signals(of_kind(incoming, EXCITE_KIND));
//!     let inhibit: f32 = locus_signals(of_kind(incoming, INHIBIT_KIND));
//!     let net = excite - inhibit;
//!     // ...
//! }
//! ```

use crate::change::{Change, ChangeSubject};
use crate::ids::InfluenceKindId;

/// Iterate over the subset of `incoming` that carry influence kind `kind`.
///
/// Returns a lazy iterator — nothing is allocated. Pass the result
/// directly to `locus_signals` or collect it yourself.
pub fn of_kind<'a>(
    incoming: &'a [&'a Change],
    kind: InfluenceKindId,
) -> impl Iterator<Item = &'a Change> + 'a {
    incoming
        .iter()
        .filter_map(move |c| if c.kind == kind { Some(*c) } else { None })
}

/// Sum the first state slot (`after[0]`) over all locus-subject changes
/// in `iter`.
///
/// Relationship-subject changes are skipped (they modify edge state, not
/// locus activation). Changes with an empty `after` vector (zero-slot state)
/// contribute `0.0` — they are counted as zero activation, not dropped.
/// Returns `0.0` for an empty iterator.
///
/// Works with any `impl Iterator<Item = &Change>`, so it composes
/// naturally with `of_kind`:
///
/// ```rust,ignore
/// let total = locus_signals(of_kind(incoming, MY_KIND));
/// ```
pub fn locus_signals<'a>(iter: impl Iterator<Item = &'a Change>) -> f32 {
    iter.filter_map(|c| {
        if matches!(c.subject, ChangeSubject::Locus(_)) {
            c.after.as_slice().first().copied()
        } else {
            None
        }
    })
    .sum()
}
