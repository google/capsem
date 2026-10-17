//! OS-seeded randomness. Failure is explicit when the caller can recover.
use rand::Rng;

pub fn fill(bytes: &mut [u8]) -> Result<(), getrandom::Error> {
    getrandom::fill(bytes)
}

pub fn alphanumeric(length: usize) -> String {
    rand::rng()
        .sample_iter(&rand::distr::Alphanumeric)
        .take(length)
        .map(char::from)
        .collect()
}

pub fn u64() -> u64 {
    rand::random()
}

pub fn range_u32(range: std::ops::Range<u32>) -> u32 {
    rand::rng().random_range(range)
}

/// UUID generation is also entropy use, including UUID-derived capabilities.
pub fn uuid_v4() -> uuid::Uuid {
    uuid::Uuid::new_v4()
}
