//! Accelerated XXH3-128 for non-security fingerprints, quick corruption checks
//! and deduplication. This is not authentication or adversarial integrity proof.
//! Fingerprints are probabilistic identities; external digest contracts keep
//! their required algorithm. The backend selects NEON/SIMD at compilation.

/// Hash existing bytes without allocation or copying.
#[inline]
pub fn hash(bytes: &[u8]) -> u128 {
    xxhash_rust::xxh3::xxh3_128(bytes)
}

/// An incremental 128-bit fingerprint with the same result as [`hash`].
#[derive(Clone, Default)]
pub struct Hasher(xxhash_rust::xxh3::Xxh3);

impl Hasher {
    pub fn new() -> Self {
        Self::default()
    }

    #[inline]
    pub fn update(&mut self, bytes: &[u8]) {
        self.0.update(bytes);
    }

    #[inline]
    pub fn finalize(&self) -> u128 {
        self.0.digest128()
    }
}

impl std::io::Write for Hasher {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.update(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests;
