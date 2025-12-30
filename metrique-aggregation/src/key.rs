//! Keys for aggregate metrics

use std::hash::Hash;

/// Defines a default key for a given struct
///
/// This is what is automatically defined when using the `(key)` macros
pub trait DefaultKey: Sized {
    /// Type of the key (Keyer)
    type KeyType: Key<Self>;
}

/// Key defines the aggregation key for a given type `T`
///
/// This allows the same struct to define multiple different keys
///
/// ```
/// struct Metric {
///     operation: String,
///     status: String
/// }
///
/// struct Operation;
///
/// impl Key<Metric> for Operation {
///     Key<'a> = &'a str;
///     fn key<'a>(entry: &'a Metric) -> &'a str { &entry.operation }
/// }
pub trait Key<T> {
    /// Key type that identifies which entries can be aggregated together.
    type Key<'a>: Eq + Hash + Clone
    where
        T: 'a;

    /// Returns the key for this metric
    fn key<'a>(entry: &'a T) -> Self::Key<'a>;
}

/// Creates a new metric from a key
pub trait FromKey<T> {
    /// Creates a new metric from a key
    fn new_from_key(key: T) -> Self;
}
