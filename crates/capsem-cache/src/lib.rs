//! Bounded, observable cache primitives.
//!
//! Domain owners decide what makes a value fresh. This crate owns the shared
//! concurrency, capacity and telemetry mechanics used to retain those values.

mod bounded;
mod keyed_identity;
mod snapshot;
mod telemetry;

pub use bounded::BoundedCache;
pub use keyed_identity::KeyedIdentityCache;
pub use snapshot::{GenerationCache, GenerationLookup, IdentityCache};
pub use telemetry::{CacheStats, LoadStatus};

#[cfg(test)]
mod tests;
