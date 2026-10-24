//! Shared bounded writer; protocol framing owns its implementation.
pub use capsem_proto::framing::LimitedWriter;

#[cfg(test)]
mod tests;
