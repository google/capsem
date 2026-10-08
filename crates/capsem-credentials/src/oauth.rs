//! One-use native OAuth callback ownership. Enrollment and token storage are separate.
//! PKCE follows RFC 7636; redirect/state binding follows RFC 8252 sections 8.9-8.10.

use std::fmt;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

mod google;
mod listener;
pub use google::{
    GoogleOAuthClient, GoogleRegistration, OAuthAuthorizationUrl, OAuthHttpPolicy, OAuthProviderError, OAuthTokenError,
    OAuthTokens,
};
pub use listener::{OAuthListener, OAuthListenerError, OAuthListenerPolicy};

#[derive(Debug, Clone)]
pub struct LoopbackRedirect {
    uri: String,
}

impl LoopbackRedirect {
    /// Bind to an already selected literal loopback address and exact provider path.
    pub fn new(address: SocketAddr, path: &str) -> Result<Self, OAuthError> {
        if !address.ip().is_loopback()
            || address.port() == 0
            || !path.starts_with('/')
            || path.starts_with("//")
            || !path.bytes().all(|b| b.is_ascii_alphanumeric() || b"/-._~".contains(&b))
            || path.split('/').any(|part| part == "." || part == "..")
        {
            return Err(OAuthError::InvalidRedirect);
        }
        Ok(Self {
            uri: format!("http://{address}{path}"),
        })
    }

    pub fn uri(&self) -> &str {
        &self.uri
    }
}

/// The host owner supplies reviewed limits; this primitive chooses no defaults.
#[derive(Debug, Clone, Copy)]
pub struct OAuthPolicy {
    pub lifetime: Duration,
    pub max_callback_bytes: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OAuthState {
    Pending,
    Consumed,
    Denied,
    Failed,
    Cancelled,
    Expired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OAuthError {
    InvalidRedirect,
    InvalidPolicy,
    EntropyUnavailable,
    InvalidCallback,
    InvalidState,
    Denied,
    ProviderRejected,
    Inactive(OAuthState),
}

impl fmt::Display for OAuthError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::InvalidRedirect => "invalid OAuth loopback redirect",
            Self::InvalidPolicy => "invalid OAuth transaction policy",
            Self::EntropyUnavailable => "OAuth entropy source unavailable",
            Self::InvalidCallback => "invalid OAuth callback",
            Self::InvalidState => "OAuth callback state mismatch",
            Self::Denied => "OAuth consent denied",
            Self::ProviderRejected => "OAuth provider rejected authorization",
            Self::Inactive(_) => "OAuth transaction is no longer pending",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for OAuthError {}

struct Secret(Zeroizing<String>);

impl<'de> serde::Deserialize<'de> for Secret {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        <String as serde::Deserialize>::deserialize(deserializer).map(Self::new)
    }
}

impl Secret {
    fn new(value: String) -> Self {
        Self(Zeroizing::new(value))
    }
    fn expose(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[REDACTED]")
    }
}

#[derive(Debug)]
struct Pending {
    state: Secret,
    verifier: Secret,
    challenge: String,
}

/// Borrowed values for the host's authorization URL builder, never a token grant.
pub struct AuthorizationParameters<'a> {
    pub state: &'a str,
    pub code_challenge: &'a str,
    pub code_challenge_method: &'static str,
    pub redirect_uri: &'a str,
}

impl fmt::Debug for AuthorizationParameters<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthorizationParameters")
            .field("code_challenge_method", &self.code_challenge_method)
            .field("redirect_uri", &self.redirect_uri)
            .finish_non_exhaustive()
    }
}

/// Unclonable exchange material. The token adapter must not log its explicit getters.
#[derive(Debug)]
pub struct CallbackExchange {
    code: Secret,
    verifier: Secret,
    redirect_uri: String,
}

impl CallbackExchange {
    pub fn code(&self) -> &str {
        self.code.expose()
    }
    pub fn verifier(&self) -> &str {
        self.verifier.expose()
    }
    pub fn redirect_uri(&self) -> &str {
        &self.redirect_uri
    }
}

#[derive(Debug)]
pub struct OAuthAttempt {
    redirect: LoopbackRedirect,
    deadline: Instant,
    max_callback_bytes: usize,
    state: OAuthState,
    pending: Option<Pending>,
}

impl OAuthAttempt {
    pub fn new(redirect: LoopbackRedirect, policy: OAuthPolicy, now: Instant) -> Result<Self, OAuthError> {
        if policy.lifetime.is_zero() || policy.max_callback_bytes == 0 {
            return Err(OAuthError::InvalidPolicy);
        }
        let deadline = now.checked_add(policy.lifetime).ok_or(OAuthError::InvalidPolicy)?;
        let verifier = random_secret()?;
        let challenge = s256(verifier.expose());
        let state = random_secret()?;
        Ok(Self {
            redirect,
            deadline,
            max_callback_bytes: policy.max_callback_bytes,
            state: OAuthState::Pending,
            pending: Some(Pending {
                state,
                verifier,
                challenge,
            }),
        })
    }

    pub fn authorization(&mut self, now: Instant) -> Result<AuthorizationParameters<'_>, OAuthError> {
        self.ensure_pending(now)?;
        let pending = self.pending.as_ref().ok_or(OAuthError::Inactive(self.state))?;
        Ok(AuthorizationParameters {
            state: pending.state.expose(),
            code_challenge: &pending.challenge,
            code_challenge_method: "S256",
            redirect_uri: self.redirect.uri(),
        })
    }

    pub fn status(&mut self, now: Instant) -> OAuthState {
        if self.state == OAuthState::Pending && now >= self.deadline {
            self.pending = None;
            self.state = OAuthState::Expired;
        }
        self.state
    }

    pub fn cancel(&mut self) {
        if self.state == OAuthState::Pending {
            self.pending = None;
            self.state = OAuthState::Cancelled;
        }
    }

    /// Accept the exact URI received by the trusted loopback listener. Its base must
    /// come from the actual bound address and raw path, never an untrusted Host header.
    /// Reject malformed/foreign callbacks without consuming a legitimate attempt.
    pub fn accept(&mut self, callback: &str, now: Instant) -> Result<CallbackExchange, OAuthError> {
        self.ensure_pending(now)?;
        if callback.len() > self.max_callback_bytes || callback.contains('#') {
            return Err(OAuthError::InvalidCallback);
        }
        let (base, query) = callback.split_once('?').ok_or(OAuthError::InvalidCallback)?;
        if base != self.redirect.uri() || !valid_percent_encoding(query) {
            return Err(OAuthError::InvalidCallback);
        }
        let fields = CallbackFields::parse(query)?;
        let pending = self.pending.as_ref().ok_or(OAuthError::Inactive(self.state))?;
        let state = fields.state.as_ref().ok_or(OAuthError::InvalidState)?;
        if !bool::from(state.expose().as_bytes().ct_eq(pending.state.expose().as_bytes())) {
            return Err(OAuthError::InvalidState);
        }
        match (fields.code, fields.error) {
            (Some(code), None) if valid_response(code.expose()) => {
                let pending = self.pending.take().ok_or(OAuthError::Inactive(self.state))?;
                self.state = OAuthState::Consumed;
                Ok(CallbackExchange {
                    code,
                    verifier: pending.verifier,
                    redirect_uri: self.redirect.uri.clone(),
                })
            }
            (None, Some(error)) if valid_response(error.expose()) => {
                self.pending = None;
                if error.expose() == "access_denied" {
                    self.state = OAuthState::Denied;
                    Err(OAuthError::Denied)
                } else {
                    self.state = OAuthState::Failed;
                    Err(OAuthError::ProviderRejected)
                }
            }
            _ => Err(OAuthError::InvalidCallback),
        }
    }

    fn ensure_pending(&mut self, now: Instant) -> Result<(), OAuthError> {
        match self.status(now) {
            OAuthState::Pending => Ok(()),
            state => Err(OAuthError::Inactive(state)),
        }
    }
}

#[derive(Default)]
struct CallbackFields {
    state: Option<Secret>,
    code: Option<Secret>,
    error: Option<Secret>,
}

impl CallbackFields {
    fn parse(query: &str) -> Result<Self, OAuthError> {
        let mut fields = Self::default();
        for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
            let field = match key.as_ref() {
                "state" => &mut fields.state,
                "code" => &mut fields.code,
                "error" => &mut fields.error,
                _ => continue, // OAuth response extensions are not authority.
            };
            if field.replace(Secret::new(value.into_owned())).is_some() {
                return Err(OAuthError::InvalidCallback);
            }
        }
        Ok(fields)
    }
}

fn valid_response(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|byte| (0x20..=0x7e).contains(&byte))
}

fn valid_percent_encoding(query: &str) -> bool {
    let bytes = query.as_bytes();
    bytes
        .iter()
        .enumerate()
        .filter(|(_, byte)| **byte == b'%')
        .all(|(index, _)| {
            bytes
                .get(index + 1..index + 3)
                .is_some_and(|digits| digits.iter().all(u8::is_ascii_hexdigit))
        })
}

fn random_secret() -> Result<Secret, OAuthError> {
    let mut bytes = Zeroizing::new([0u8; 32]);
    getrandom::fill(&mut *bytes).map_err(|_| OAuthError::EntropyUnavailable)?;
    Ok(Secret::new(URL_SAFE_NO_PAD.encode(bytes.as_slice())))
}

fn s256(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

#[cfg(test)]
mod tests;
