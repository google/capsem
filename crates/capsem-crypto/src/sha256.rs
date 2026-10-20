//! The OCI SHA-256 owner. AWS-LC dispatches to the CPU's SIMD/crypto backend,
//! including AVX2 on x86 hosts without SHA-NI. Digest identities stay SHA-256.

use std::fmt;

pub struct Sha256(aws_lc_rs::digest::Context);
pub struct Output(aws_lc_rs::digest::Digest);

impl Default for Sha256 {
    fn default() -> Self {
        Self::new()
    }
}

impl Sha256 {
    pub fn new() -> Self {
        Self(aws_lc_rs::digest::Context::new(&aws_lc_rs::digest::SHA256))
    }

    pub fn update(&mut self, bytes: impl AsRef<[u8]>) {
        self.0.update(bytes.as_ref());
    }

    pub fn finalize(self) -> Output {
        Output(self.0.finish())
    }

    pub fn digest(bytes: impl AsRef<[u8]>) -> Output {
        Output(aws_lc_rs::digest::digest(&aws_lc_rs::digest::SHA256, bytes.as_ref()))
    }
}

impl AsRef<[u8]> for Output {
    fn as_ref(&self) -> &[u8] {
        self.0.as_ref()
    }
}

impl std::ops::Deref for Output {
    type Target = [u8];
    fn deref(&self) -> &Self::Target {
        self.as_ref()
    }
}

impl fmt::LowerHex for Output {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.as_ref() {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;

/// RFC 6455 mandates SHA-1 for this handshake value. It is not a content hash.
pub fn websocket_accept_digest(key: &[u8]) -> [u8; 20] {
    let mut context = aws_lc_rs::digest::Context::new(&aws_lc_rs::digest::SHA1_FOR_LEGACY_USE_ONLY);
    context.update(key);
    context.update(b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11");
    context.finish().as_ref().try_into().expect("SHA-1 produces 20 bytes")
}
