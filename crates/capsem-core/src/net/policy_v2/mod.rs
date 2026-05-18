//! Policy V2 runtime rule config and CEL evaluation.
//!
//! This module is the only home for runtime policy callbacks, rule config,
//! subject lookup, and the small CEL subset used by the MITM, DNS, MCP, model,
//! service, and process enforcement paths.

mod condition;
mod types;

pub use types::*;

#[cfg(test)]
#[allow(unused_imports)]
mod tests;
