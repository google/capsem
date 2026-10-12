//! Bounded integrity verification over a caller-held reader, never a path.
use std::io::{self, Read};

enum Backend {
    Blake3(Box<crate::blake3::Hasher>),
    #[cfg(feature = "sha256")]
    Sha256(crate::Sha256),
}

/// The authenticated digest selects an identity; this owner selects its backend.
pub struct Verifier {
    backend: Backend,
    expected: crate::blake3::Hash,
}

impl Verifier {
    pub fn new(digest: &str) -> io::Result<Self> {
        let (algorithm, expected) = digest.split_once(':').ok_or_else(invalid_digest)?;
        let expected = parse_hex(expected)?;
        let backend = match algorithm {
            "blake3" => Backend::Blake3(Box::new(crate::blake3::Hasher::new())),
            #[cfg(feature = "sha256")]
            "sha256" => Backend::Sha256(crate::Sha256::new()),
            _ => return Err(invalid_digest()),
        };
        Ok(Self { backend, expected })
    }

    pub fn update(&mut self, bytes: &[u8]) {
        match &mut self.backend {
            Backend::Blake3(hasher) => {
                hasher.update(bytes);
            }
            #[cfg(feature = "sha256")]
            Backend::Sha256(hasher) => hasher.update(bytes),
        }
    }

    pub fn finish(self) -> bool {
        match self.backend {
            Backend::Blake3(hasher) => hasher.finalize() == self.expected,
            #[cfg(feature = "sha256")]
            Backend::Sha256(hasher) => hasher.finalize().as_ref() == self.expected.as_bytes(),
        }
    }
}

fn invalid_digest() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, "invalid content integrity digest")
}

fn parse_hex(expected: &str) -> io::Result<crate::blake3::Hash> {
    if expected.len() != 64
        || !expected
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(invalid_digest());
    }
    crate::blake3::Hash::from_hex(expected).map_err(|_| invalid_digest())
}

pub fn blake3_digest(digest: &str) -> io::Result<crate::blake3::Hash> {
    parse_hex(digest.strip_prefix("blake3:").ok_or_else(invalid_digest)?)
}

/// Verify a BLAKE3 content identity and its exact byte count. The caller owns
/// admission, size policy, descriptor acquisition and keeping that descriptor
/// through use. No content is retained and the first excess byte is refused.
pub fn blake3(mut source: impl Read, expected: &str, size: u64) -> io::Result<u64> {
    let expected = blake3_digest(expected)?;
    let mut hasher = crate::blake3::Hasher::new();
    let mut bytes = vec![0; 1024 * 1024];
    let mut count = 0u64;
    loop {
        let remaining = size.saturating_sub(count).saturating_add(1).min(bytes.len() as u64) as usize;
        let read = source.read(&mut bytes[..remaining])?;
        if read == 0 {
            break;
        }
        count = count
            .checked_add(read as u64)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "content byte count overflow"))?;
        if count > size {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "content exceeds declared size",
            ));
        }
        hasher.update(&bytes[..read]);
    }
    if count != size || hasher.finalize() != expected {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "content integrity mismatch"));
    }
    Ok(count)
}

#[cfg(test)]
mod tests;
