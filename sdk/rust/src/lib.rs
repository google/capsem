//! Typed, asynchronous access to the Capsem HTTP gateway.
//! Clients take an explicit gateway URL and bearer token.

pub use capsem_api as models;
mod error;
pub mod operations;
pub mod transport;
pub use error::{Error, Result};

#[cfg(test)]
mod test_gateway;
