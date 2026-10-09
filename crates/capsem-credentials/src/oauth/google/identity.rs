//! Account trust is issuer signature + registered client + one-use transaction nonce.

use std::collections::BTreeMap;
use std::fmt;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use super::{GoogleOAuthClient, OAuthTokenError, OAuthTokens, Secret};

mod crypto;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OAuthIdentityError {
    NoIdentity,
    InvalidIdentity,
    AccountMismatch,
    InvalidKeys,
    CryptoUnavailable,
    Transport(OAuthTokenError),
}

impl fmt::Display for OAuthIdentityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NoIdentity => "Google identity evidence unavailable",
            Self::InvalidIdentity => "Google identity verification failed",
            Self::AccountMismatch => "Google account does not match the connection",
            Self::InvalidKeys => "Google signing keys invalid",
            Self::CryptoUnavailable => "Google identity verifier unavailable",
            Self::Transport(_) => "Google signing-key retrieval failed",
        })
    }
}
impl std::error::Error for OAuthIdentityError {}

/// Created only after signature/account/transaction checks. Subject, not email,
/// is the account binding; it is sensitive metadata and absent from Debug.
#[derive(Debug)]
pub struct GoogleIdentity {
    subject: Secret,
    expires_at: u64,
}
impl GoogleIdentity {
    pub fn subject(&self) -> &str {
        self.subject.expose()
    }
    pub fn expires_at_unix_seconds(&self) -> u64 {
        self.expires_at
    }
}

impl GoogleOAuthClient {
    /// Verify initial consent evidence before persisting/activating a connection.
    /// A reconnect supplies its existing subject to prohibit silent account switches.
    /// Refresh responses carry no initial nonce and cannot establish a new identity.
    pub async fn verify_identity(
        &self,
        tokens: &OAuthTokens,
        expected_subject: Option<&str>,
    ) -> Result<GoogleIdentity, OAuthIdentityError> {
        self.check_registration(tokens).map_err(OAuthIdentityError::Transport)?;
        let raw = tokens.unverified_id_token().ok_or(OAuthIdentityError::NoIdentity)?;
        let nonce = tokens.expected_nonce.as_ref().ok_or(OAuthIdentityError::NoIdentity)?;
        if tokens.access_token(Instant::now()).is_none() || raw.len() > self.policy.max_response_bytes {
            return Err(OAuthIdentityError::InvalidIdentity);
        }
        let kid = key_id(raw)?;
        let response = self
            .http
            .get(self.identity_keys_endpoint.clone())
            .header("accept", "application/json")
            .send()
            .await
            .map_err(super::network_error)
            .map_err(OAuthIdentityError::Transport)?;
        let body = self
            .read_response(response)
            .await
            .map_err(OAuthIdentityError::Transport)?;
        let raw = Secret::new(raw.to_owned());
        let nonce = Secret::new(nonce.expose().to_owned());
        let access = Secret::new(tokens.access_token.expose().to_owned());
        let client_id = self.registration.client_id.clone();
        let expected_subject = expected_subject.map(|subject| Secret::new(subject.to_owned()));
        // Cryptographic work runs off the async worker. Cancellation cannot publish
        // an identity or mutate a connection; all captured evidence remains owned.
        tokio::task::spawn_blocking(move || {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|_| OAuthIdentityError::InvalidIdentity)?
                .as_secs();
            let key = signing_key(&body, &kid)?;
            verify(&raw, &nonce, &access, &client_id, expected_subject.as_ref(), &key, now)
        })
        .await
        .map_err(|_| OAuthIdentityError::CryptoUnavailable)?
    }
}

fn key_id(token: &str) -> Result<String, OAuthIdentityError> {
    let header = decode_header(token).map_err(|_| OAuthIdentityError::InvalidIdentity)?;
    if header.alg != Algorithm::RS256
        || header.crit.is_some()
        || header.jku.is_some()
        || header.jwk.is_some()
        || header.x5u.is_some()
        || header.enc.is_some()
        || header.zip.is_some()
        || header.typ.as_deref().is_some_and(|value| value != "JWT")
    {
        return Err(OAuthIdentityError::InvalidIdentity);
    }
    header
        .kid
        .filter(|kid| super::valid_token(kid))
        .ok_or(OAuthIdentityError::InvalidIdentity)
}

#[derive(Deserialize)]
struct Keys {
    keys: Vec<Key>,
}
#[derive(Deserialize)]
struct Key {
    kid: String,
    kty: String,
    alg: Option<String>,
    #[serde(rename = "use")]
    usage: Option<String>,
    key_ops: Option<Vec<String>>,
    n: String,
    e: String,
}

fn signing_key(body: &[u8], kid: &str) -> Result<DecodingKey, OAuthIdentityError> {
    let keys: Keys = serde_json::from_slice(body).map_err(|_| OAuthIdentityError::InvalidKeys)?;
    let mut unique = BTreeMap::new();
    for key in keys.keys {
        if !super::valid_token(&key.kid)
            || key.kty != "RSA"
            || key.alg.as_deref().is_some_and(|alg| alg != "RS256")
            || key.usage.as_deref().is_some_and(|usage| usage != "sig")
            || key.key_ops.as_ref().is_some_and(|ops| ops.as_slice() != ["verify"])
        {
            return Err(OAuthIdentityError::InvalidKeys);
        }
        let n = URL_SAFE_NO_PAD
            .decode(&key.n)
            .map_err(|_| OAuthIdentityError::InvalidKeys)?;
        let e = URL_SAFE_NO_PAD
            .decode(&key.e)
            .map_err(|_| OAuthIdentityError::InvalidKeys)?;
        // ring's RSA contract supports 2048..8192-bit moduli; constrain allocation
        // and avoid silently accepting tiny keys or unbounded exponents.
        if !(256..=1024).contains(&n.len())
            || e.is_empty()
            || e.len() > 8
            || unique
                .insert(key.kid, DecodingKey::from_rsa_raw_components(&n, &e))
                .is_some()
        {
            return Err(OAuthIdentityError::InvalidKeys);
        }
    }
    unique.remove(kid).ok_or(OAuthIdentityError::InvalidKeys)
}

#[derive(Deserialize)]
struct Claims {
    iss: String,
    aud: String,
    sub: Secret,
    exp: u64,
    iat: u64,
    nbf: Option<u64>,
    azp: Option<String>,
    nonce: Secret,
    at_hash: Option<Secret>,
}

fn verify(
    token: &Secret,
    nonce: &Secret,
    access: &Secret,
    client_id: &str,
    expected_subject: Option<&Secret>,
    key: &DecodingKey,
    now: u64,
) -> Result<GoogleIdentity, OAuthIdentityError> {
    crypto::install()?;
    let mut validation = Validation::new(Algorithm::RS256);
    validation.set_issuer(&["https://accounts.google.com", "accounts.google.com"]);
    validation.set_audience(&[client_id]);
    validation.set_required_spec_claims(&["exp", "iss", "aud", "sub"]);
    // Use the host's single post-fetch timestamp below, with exact expiry. The
    // library still validates signature/algorithm/issuer/audience/required claims.
    validation.validate_exp = false;
    validation.leeway = 0;
    let claims = decode::<Claims>(token.expose(), key, &validation)
        .map_err(|_| OAuthIdentityError::InvalidIdentity)?
        .claims;
    if !matches!(
        claims.iss.as_str(),
        "https://accounts.google.com" | "accounts.google.com"
    ) || claims.aud != client_id
        || claims.azp.as_deref().is_some_and(|azp| azp != client_id)
        || claims.exp <= now
        || claims.iat > now
        || claims.exp <= claims.iat
        || claims.nbf.is_some_and(|nbf| nbf > now)
        || !super::valid_token(claims.sub.expose())
        || claims.sub.expose().len() > 255
        || !bool::from(claims.nonce.expose().as_bytes().ct_eq(nonce.expose().as_bytes()))
    {
        return Err(OAuthIdentityError::InvalidIdentity);
    }
    if let Some(hash) = claims.at_hash {
        let digest = Sha256::digest(access.expose().as_bytes());
        let expected = URL_SAFE_NO_PAD.encode(&digest[..16]);
        if !bool::from(hash.expose().as_bytes().ct_eq(expected.as_bytes())) {
            return Err(OAuthIdentityError::InvalidIdentity);
        }
    }
    if expected_subject.is_some_and(|expected| expected.expose() != claims.sub.expose()) {
        return Err(OAuthIdentityError::AccountMismatch);
    }
    Ok(GoogleIdentity {
        subject: claims.sub,
        expires_at: claims.exp,
    })
}

#[cfg(test)]
mod tests;
