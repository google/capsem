//! Versioned accidental-corruption checks, not authentication. Existing v3
//! generations retain BLAKE3 while new generations use XXH3-128. A v4 file
//! may contain copied v3 blocks, whose segment tags retain their algorithm.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Checksum {
    Blake3 = 0,
    Xxh3 = 1,
}

impl Checksum {
    pub(crate) fn for_version(version: u16) -> Option<Self> {
        match version {
            3 => Some(Self::Blake3),
            4 => Some(Self::Xxh3),
            _ => None,
        }
    }

    pub(crate) fn from_tag(tag: u8) -> Option<Self> {
        match tag {
            0 => Some(Self::Blake3),
            1 => Some(Self::Xxh3),
            _ => None,
        }
    }

    pub(crate) fn hash(self, bytes: &[u8]) -> [u8; 32] {
        match self {
            Self::Blake3 => *capsem_crypto::blake3::hash(bytes).as_bytes(),
            Self::Xxh3 => padded(capsem_crypto::xxh3::hash(bytes)),
        }
    }

    pub(crate) fn start(self) -> Hasher {
        match self {
            Self::Blake3 => Hasher::Blake3(Box::new(capsem_crypto::blake3::Hasher::new())),
            Self::Xxh3 => Hasher::Xxh3(Box::new(capsem_crypto::xxh3::Hasher::new())),
        }
    }
}

pub(crate) enum Hasher {
    Blake3(Box<capsem_crypto::blake3::Hasher>),
    Xxh3(Box<capsem_crypto::xxh3::Hasher>),
}

impl Hasher {
    pub(crate) fn update(&mut self, bytes: &[u8]) {
        match self {
            Self::Blake3(state) => {
                state.update(bytes);
            }
            Self::Xxh3(state) => state.update(bytes),
        }
    }

    pub(crate) fn finish_and_reset(&mut self) -> [u8; 32] {
        match self {
            Self::Blake3(state) => {
                let hash = *state.finalize().as_bytes();
                state.reset();
                hash
            }
            Self::Xxh3(state) => {
                let hash = padded(state.finalize());
                **state = capsem_crypto::xxh3::Hasher::new();
                hash
            }
        }
    }
}

// The fixed on-disk slot is 32 bytes for v3 block-copy compatibility. Only
// the first 16 bytes are a digest for XXH3; the remaining 16 must be zero.
fn padded(hash: u128) -> [u8; 32] {
    let mut bytes = [0; 32];
    bytes[..16].copy_from_slice(&hash.to_le_bytes());
    bytes
}
