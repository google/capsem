//! Native archive codec owned exclusively by the confined ledger executable.

mod implementation;

pub(crate) use implementation::archive_codecs;

#[cfg(test)]
mod tests;
