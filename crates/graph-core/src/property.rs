//! Domain-level property storage for loci.
//!
//! The engine operates on `StateVector` (f32 arrays), but users work with
//! named, typed properties — strings, numbers, booleans, lists, and nested
//! maps. `Properties` bridges the gap: it travels alongside `LocusId` in
//! the `PropertyStore` (graph-world) and is fed to an `Encoder` to produce
//! the `StateVector` the engine consumes.

use std::fmt;

/// A single property value. Intentionally kept lightweight and free of
/// external dependencies (no serde_json).
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum PropertyValue {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
    List(Vec<PropertyValue>),
    Map(Properties),
}

impl fmt::Display for PropertyValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Null => write!(f, "null"),
            Self::Bool(b) => write!(f, "{b}"),
            Self::Int(i) => write!(f, "{i}"),
            Self::Float(v) => write!(f, "{v}"),
            Self::String(s) => write!(f, "\"{s}\""),
            Self::List(l) => write!(f, "[{}]", l.iter().map(|v| v.to_string()).collect::<Vec<_>>().join(", ")),
            Self::Map(m) => write!(f, "{m:?}"),
        }
    }
}

// ── From impls for ergonomic construction ─────────────────────────────

impl From<bool> for PropertyValue {
    fn from(v: bool) -> Self { Self::Bool(v) }
}
impl From<i64> for PropertyValue {
    fn from(v: i64) -> Self { Self::Int(v) }
}
impl From<i32> for PropertyValue {
    fn from(v: i32) -> Self { Self::Int(v as i64) }
}
impl From<f64> for PropertyValue {
    fn from(v: f64) -> Self { Self::Float(v) }
}
impl From<f32> for PropertyValue {
    fn from(v: f32) -> Self { Self::Float(v as f64) }
}
impl From<&str> for PropertyValue {
    fn from(v: &str) -> Self { Self::String(v.to_owned()) }
}
impl From<String> for PropertyValue {
    fn from(v: String) -> Self { Self::String(v) }
}
impl<T: Into<PropertyValue>> From<Vec<T>> for PropertyValue {
    fn from(v: Vec<T>) -> Self {
        Self::List(v.into_iter().map(Into::into).collect())
    }
}

/// A bag of named properties attached to a locus.
///
/// Internally a `Vec<(String, PropertyValue)>` to keep insertion order
/// and stay allocation-friendly for small property counts (typically < 20).
#[derive(Debug, Clone, Default, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Properties {
    entries: Vec<(String, PropertyValue)>,
}

impl Properties {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert or overwrite a property.
    pub fn set(&mut self, key: impl Into<String>, value: impl Into<PropertyValue>) {
        let key = key.into();
        if let Some(entry) = self.entries.iter_mut().find(|(k, _)| *k == key) {
            entry.1 = value.into();
        } else {
            self.entries.push((key, value.into()));
        }
    }

    pub fn get(&self, key: &str) -> Option<&PropertyValue> {
        self.entries.iter().find(|(k, _)| k == key).map(|(_, v)| v)
    }

    /// Convenience: get a string property.
    pub fn get_str(&self, key: &str) -> Option<&str> {
        match self.get(key) {
            Some(PropertyValue::String(s)) => Some(s),
            _ => None,
        }
    }

    /// Convenience: get an f64 property (accepts both Float and Int).
    pub fn get_f64(&self, key: &str) -> Option<f64> {
        match self.get(key) {
            Some(PropertyValue::Float(v)) => Some(*v),
            Some(PropertyValue::Int(v)) => Some(*v as f64),
            _ => None,
        }
    }

    /// Convenience: get as f32.
    pub fn get_f32(&self, key: &str) -> Option<f32> {
        self.get_f64(key).map(|v| v as f32)
    }

    pub fn remove(&mut self, key: &str) -> Option<PropertyValue> {
        if let Some(pos) = self.entries.iter().position(|(k, _)| k == key) {
            Some(self.entries.swap_remove(pos).1)
        } else {
            None
        }
    }

    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.entries.iter().map(|(k, _)| k.as_str())
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &PropertyValue)> {
        self.entries.iter().map(|(k, v)| (k.as_str(), v))
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Merge all entries from `other` into `self`, overwriting on key collision.
    pub fn extend(&mut self, other: &Properties) {
        for (key, value) in other.iter() {
            self.set(key, value.clone());
        }
    }
}

/// Builder macro for `Properties`. Usage:
///
/// ```
/// use graph_core::props;
/// let p = props! {
///     "name" => "Apple",
///     "type" => "ORG",
///     "confidence" => 0.95_f64,
/// };
/// assert_eq!(p.get_str("name"), Some("Apple"));
/// ```
#[macro_export]
macro_rules! props {
    ($($key:expr => $val:expr),* $(,)?) => {{
        let mut p = $crate::Properties::new();
        $( p.set($key, $val); )*
        p
    }};
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_and_get_various_types() {
        let mut p = Properties::new();
        p.set("name", "Alice");
        p.set("age", 30_i64);
        p.set("score", 0.95_f64);
        p.set("active", true);
        assert_eq!(p.get_str("name"), Some("Alice"));
        assert_eq!(p.get_f64("age"), Some(30.0));
        assert_eq!(p.get_f64("score"), Some(0.95));
        assert_eq!(p.get("active"), Some(&PropertyValue::Bool(true)));
    }

    #[test]
    fn overwrite_preserves_position() {
        let mut p = Properties::new();
        p.set("a", "first");
        p.set("b", "second");
        p.set("a", "updated");
        assert_eq!(p.len(), 2);
        assert_eq!(p.get_str("a"), Some("updated"));
    }

    #[test]
    fn props_macro_works() {
        let p = props! {
            "name" => "Apple",
            "type" => "ORG",
            "confidence" => 0.95_f64,
        };
        assert_eq!(p.get_str("name"), Some("Apple"));
        assert_eq!(p.get_str("type"), Some("ORG"));
        assert!((p.get_f64("confidence").unwrap() - 0.95).abs() < 1e-10);
    }

    #[test]
    fn remove_returns_value() {
        let mut p = props! { "x" => 42_i64 };
        assert_eq!(p.remove("x"), Some(PropertyValue::Int(42)));
        assert!(p.is_empty());
    }

    #[test]
    fn list_property() {
        let mut p = Properties::new();
        p.set("aliases", vec!["AAPL", "Apple Inc."]);
        match p.get("aliases") {
            Some(PropertyValue::List(items)) => assert_eq!(items.len(), 2),
            other => panic!("expected List, got {other:?}"),
        }
    }
}
