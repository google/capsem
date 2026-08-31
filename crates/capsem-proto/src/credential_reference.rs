/// Prefix for opaque credential references shared by configuration, storage,
/// telemetry, and MCP transport boundaries.
pub const CREDENTIAL_REF_PREFIX: &str = "credential:blake3:";

const CREDENTIAL_REF_DOMAIN: &[u8] = b"capsem.credential.v1";

/// Build the canonical, domain-separated reference for a provider credential.
pub fn credential_reference(provider: &str, raw_credential: &str) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(CREDENTIAL_REF_DOMAIN);
    hasher.update(&[0]);
    hasher.update(provider.as_bytes());
    hasher.update(&[0]);
    hasher.update(raw_credential.as_bytes());
    format!("{CREDENTIAL_REF_PREFIX}{}", hasher.finalize().to_hex())
}

/// Return whether `value` has the canonical opaque credential-reference shape.
pub fn is_credential_reference(value: &str) -> bool {
    value
        .strip_prefix(CREDENTIAL_REF_PREFIX)
        .is_some_and(|hex| hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
}

#[cfg(test)]
mod tests;
