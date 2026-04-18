//! [`DeclaredFact`]: a time-versioned assertion between two loci.

use graph_core::LocusId;

/// A predicate category for a declared relationship.
///
/// Intentionally a newtype over `String` rather than an enum so that users
/// can introduce domain-specific predicates without modifying this crate.
/// Typical values: `"reports_to"`, `"is_member_of"`, `"contracted_with"`,
/// `"is_same_as"`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct DeclaredRelKind(pub String);

impl DeclaredRelKind {
    pub fn new(s: impl Into<String>) -> Self {
        DeclaredRelKind(s.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for DeclaredRelKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Stable ID for a [`DeclaredFact`] within a [`DeclarationStore`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct DeclaredFactId(pub u64);

/// A single time-versioned assertion: "subject `predicate` object".
///
/// ## Versioning
///
/// `asserted_at` is the [`DeclarationStore`] version at which this fact was
/// first recorded. `retracted_at` is the version at which it was withdrawn,
/// or `None` if still active.
///
/// Use [`DeclarationStore::facts_at`] to query facts active at a given version.
#[derive(Debug, Clone, PartialEq)]
pub struct DeclaredFact {
    pub id: DeclaredFactId,
    pub subject: LocusId,
    pub predicate: DeclaredRelKind,
    pub object: LocusId,
    /// Store version when this fact was asserted.
    pub asserted_at: u64,
    /// Store version when this fact was retracted, or `None` if still active.
    pub retracted_at: Option<u64>,
}

impl DeclaredFact {
    /// Returns `true` if this fact is currently active (not retracted).
    #[inline]
    pub fn is_active(&self) -> bool {
        self.retracted_at.is_none()
    }

    /// Returns `true` if this fact was active at the given store version.
    #[inline]
    pub fn active_at(&self, version: u64) -> bool {
        self.asserted_at <= version && self.retracted_at.is_none_or(|r| version < r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_fact(asserted: u64, retracted: Option<u64>) -> DeclaredFact {
        DeclaredFact {
            id: DeclaredFactId(0),
            subject: LocusId(1),
            predicate: DeclaredRelKind::new("reports_to"),
            object: LocusId(2),
            asserted_at: asserted,
            retracted_at: retracted,
        }
    }

    #[test]
    fn active_fact_is_active_at_or_after_asserted() {
        let f = make_fact(3, None);
        assert!(!f.active_at(2));
        assert!(f.active_at(3));
        assert!(f.active_at(100));
    }

    #[test]
    fn retracted_fact_is_inactive_at_and_after_retraction() {
        let f = make_fact(3, Some(7));
        assert!(!f.active_at(2));
        assert!(f.active_at(3));
        assert!(f.active_at(6));
        assert!(!f.active_at(7));
        assert!(!f.active_at(100));
    }

    #[test]
    fn is_active_reflects_retraction() {
        assert!(make_fact(1, None).is_active());
        assert!(!make_fact(1, Some(2)).is_active());
    }
}
