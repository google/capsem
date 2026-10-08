//! Explicitly registered, bounded Google OAuth transport. No storage or grant authority.

use std::collections::BTreeSet;
use std::fmt;
use std::net::SocketAddr;
use std::time::{Duration, Instant};

use serde::Deserialize;
use url::Url;
use zeroize::Zeroizing;

use super::{AuthorizationParameters, CallbackExchange, LoopbackRedirect, Secret};

mod connection;
#[cfg(test)]
pub(crate) use connection::tests::setup as test_connection;
#[cfg(test)]
pub(crate) use tests::fixture::Reply as TestReply;
mod identity;
pub use connection::{
    GoogleAccessLease, GoogleAuthorization, GoogleConnection, GoogleConnectionError, GoogleConnectionState,
    GoogleConnectionStatus, GoogleRevocationOutcome, OAuthConnectionStorage,
};
pub use identity::{GoogleIdentity, OAuthIdentityError};

#[derive(Debug)]
pub struct GoogleRegistration {
    client_id: String,
    client_secret: Option<Secret>,
}

impl GoogleRegistration {
    /// The trusted host supplies an approved owned registration, never a borrowed
    /// vendor identity. This primitive neither discovers nor creates a registration.
    pub fn new(client_id: String, client_secret: Option<String>) -> Result<Self, OAuthTokenError> {
        if !valid_token(&client_id) || client_secret.as_deref().is_some_and(|value| !valid_token(value)) {
            return Err(OAuthTokenError::InvalidRegistration);
        }
        Ok(Self {
            client_id,
            client_secret: client_secret.map(Secret::new),
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub struct OAuthHttpPolicy {
    pub request_timeout: Duration,
    pub max_request_bytes: usize,
    pub max_response_bytes: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OAuthProviderError {
    InvalidGrant,
    InvalidClient,
    InvalidScope,
    InvalidRequest,
    TemporarilyUnavailable,
    ProofRequired,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OAuthTokenError {
    InvalidRegistration,
    InvalidPolicy,
    InvalidParameters,
    RegistrationMismatch,
    MissingRefreshToken,
    ReauthorizationRequired,
    InvalidResponse,
    ScopeDenied,
    RequestTooLarge,
    ResponseTooLarge,
    Network,
    Timeout,
    ProviderRejected {
        status: u16,
        code: Option<OAuthProviderError>,
    },
}

impl fmt::Display for OAuthTokenError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidRegistration => "invalid Google OAuth registration",
            Self::InvalidPolicy => "invalid Google OAuth transport policy",
            Self::InvalidParameters => "invalid Google OAuth authorization parameters",
            Self::RegistrationMismatch => "Google OAuth registration does not own these tokens",
            Self::MissingRefreshToken => "Google OAuth refresh token unavailable",
            Self::ReauthorizationRequired => "Google OAuth authorization required",
            Self::InvalidResponse => "invalid Google OAuth token response",
            Self::ScopeDenied => "Google OAuth response exceeds approved scopes",
            Self::RequestTooLarge => "Google OAuth request exceeds its byte budget",
            Self::ResponseTooLarge => "Google OAuth response exceeds its byte budget",
            Self::Network => "Google OAuth transport failed",
            Self::Timeout => "Google OAuth transport timed out",
            Self::ProviderRejected { .. } => "Google OAuth provider rejected the request",
        })
    }
}
impl std::error::Error for OAuthTokenError {}

#[derive(Debug)]
pub struct OAuthAuthorizationUrl(Secret);
impl OAuthAuthorizationUrl {
    /// Only for trusted browser handoff; do not record this state-bearing URL.
    pub fn as_str(&self) -> &str {
        self.0.expose()
    }
}

#[derive(Debug)]
pub struct OAuthTokens {
    client_id: String,
    access_token: Secret,
    refresh_token: Option<Secret>,
    id_token: Option<Secret>,
    expected_nonce: Option<Secret>,
    expires_at: Instant,
    refresh_expires_at: Option<Instant>,
    scopes: BTreeSet<String>,
}

impl OAuthTokens {
    pub fn access_token(&self, now: Instant) -> Option<&str> {
        (now < self.expires_at).then(|| self.access_token.expose())
    }
    /// Raw material for the host transport/protected store, never a public DTO.
    pub fn refresh_token(&self) -> Option<&str> {
        self.refresh_token.as_ref().map(Secret::expose)
    }
    /// No account/subject trust follows from parsing a token response. The
    /// connection owner must validate signature, issuer, audience and expiry.
    pub fn unverified_id_token(&self) -> Option<&str> {
        self.id_token.as_ref().map(Secret::expose)
    }
    pub fn expires_at(&self) -> Instant {
        self.expires_at
    }
    pub fn refresh_expires_at(&self) -> Option<Instant> {
        self.refresh_expires_at
    }
    pub fn granted_scopes(&self) -> &BTreeSet<String> {
        &self.scopes
    }
}

#[derive(Debug)]
pub struct GoogleOAuthClient {
    registration: GoogleRegistration,
    requested_scopes: BTreeSet<String>,
    approved_scopes: BTreeSet<String>,
    policy: OAuthHttpPolicy,
    http: reqwest::Client,
    token_endpoint: Url,
    revoke_endpoint: Url,
    identity_keys_endpoint: Url,
}

impl GoogleOAuthClient {
    pub fn new(
        registration: GoogleRegistration,
        scopes: BTreeSet<String>,
        policy: OAuthHttpPolicy,
    ) -> Result<Self, OAuthTokenError> {
        if scopes.is_empty() || scopes.iter().any(|scope| !valid_scope(scope)) {
            return Err(OAuthTokenError::InvalidRegistration);
        }
        if policy.request_timeout.is_zero()
            || Instant::now().checked_add(policy.request_timeout).is_none()
            || policy.max_request_bytes == 0
            || policy.max_response_bytes == 0
        {
            return Err(OAuthTokenError::InvalidPolicy);
        }
        let metadata_bytes = registration
            .client_id
            .len()
            .checked_add(
                registration
                    .client_secret
                    .as_ref()
                    .map_or(0, |secret| secret.expose().len()),
            )
            .and_then(|initial| {
                scopes
                    .iter()
                    .try_fold(initial, |total, scope| total.checked_add(scope.len()))
            });
        if metadata_bytes.is_none_or(|bytes| bytes > policy.max_request_bytes) {
            return Err(OAuthTokenError::RequestTooLarge);
        }
        let approved_scopes = scopes.iter().map(|scope| canonical_scope(scope).to_owned()).collect();
        Ok(Self {
            registration,
            requested_scopes: scopes,
            approved_scopes,
            policy,
            http: build_http(policy, true)?,
            token_endpoint: Url::parse("https://oauth2.googleapis.com/token").expect("fixed Google token URL"),
            revoke_endpoint: Url::parse("https://oauth2.googleapis.com/revoke").expect("fixed Google revocation URL"),
            identity_keys_endpoint: Url::parse("https://www.googleapis.com/oauth2/v3/certs")
                .expect("fixed Google JWK URL"),
        })
    }

    pub fn authorization_url(
        &self,
        params: AuthorizationParameters<'_>,
    ) -> Result<OAuthAuthorizationUrl, OAuthTokenError> {
        let redirect = Url::parse(params.redirect_uri).map_err(|_| OAuthTokenError::InvalidParameters)?;
        let ip = match redirect.host() {
            Some(url::Host::Ipv4(ip)) => ip.into(),
            Some(url::Host::Ipv6(ip)) => ip.into(),
            _ => return Err(OAuthTokenError::InvalidParameters),
        };
        let port = redirect
            .port_or_known_default()
            .ok_or(OAuthTokenError::InvalidParameters)?;
        let bound = LoopbackRedirect::new(SocketAddr::new(ip, port), redirect.path())
            .map_err(|_| OAuthTokenError::InvalidParameters)?;
        if params.redirect_uri != bound.uri()
            || params.code_challenge_method != "S256"
            || !base64url_nonce(params.state)
            || !base64url_nonce(params.nonce)
            || !base64url_nonce(params.code_challenge)
        {
            return Err(OAuthTokenError::InvalidParameters);
        }
        let mut url =
            Url::parse("https://accounts.google.com/o/oauth2/v2/auth").expect("fixed Google authorization URL");
        url.query_pairs_mut()
            .append_pair("client_id", &self.registration.client_id)
            .append_pair("response_type", "code")
            .append_pair("access_type", "offline")
            .append_pair(
                "scope",
                &self.requested_scopes.iter().cloned().collect::<Vec<_>>().join(" "),
            )
            .append_pair("redirect_uri", params.redirect_uri)
            .append_pair("state", params.state)
            .append_pair("nonce", params.nonce)
            .append_pair("code_challenge", params.code_challenge)
            .append_pair("code_challenge_method", "S256");
        if url.as_str().len() > self.policy.max_request_bytes {
            return Err(OAuthTokenError::RequestTooLarge);
        }
        Ok(OAuthAuthorizationUrl(Secret::new(url.into())))
    }

    /// Consumes one-use callback material. No mutation is automatically replayed.
    pub async fn exchange(&self, exchange: CallbackExchange) -> Result<OAuthTokens, OAuthTokenError> {
        let started = Instant::now();
        let fields = [
            ("grant_type", "authorization_code"),
            ("client_id", self.registration.client_id.as_str()),
            ("code", exchange.code()),
            ("code_verifier", exchange.verifier()),
            ("redirect_uri", exchange.redirect_uri()),
        ];
        let body = self.post_form(&self.token_endpoint, &fields, true).await?;
        let mut tokens = self.tokens(&body, started, &self.approved_scopes, true)?;
        tokens.expected_nonce = Some(exchange.nonce);
        Ok(tokens)
    }

    /// Returns detached replacement material. The connection owner must serialize
    /// refresh and durably commit rotation before publishing it; this mutates no store.
    pub async fn refresh(&self, previous: &OAuthTokens) -> Result<OAuthTokens, OAuthTokenError> {
        self.check_registration(previous)?;
        if previous.scopes.is_empty() || !previous.scopes.is_subset(&self.approved_scopes) {
            return Err(OAuthTokenError::ScopeDenied);
        }
        let started = Instant::now();
        if previous.refresh_expires_at.is_some_and(|expiry| started >= expiry) {
            return Err(OAuthTokenError::ReauthorizationRequired);
        }
        let refresh = previous.refresh_token().ok_or(OAuthTokenError::MissingRefreshToken)?;
        let scopes = previous.scopes.iter().cloned().collect::<Vec<_>>().join(" ");
        let fields = [
            ("grant_type", "refresh_token"),
            ("client_id", self.registration.client_id.as_str()),
            ("refresh_token", refresh),
            ("scope", scopes.as_str()),
        ];
        let body = self.post_form(&self.token_endpoint, &fields, true).await?;
        let mut replacement = self.tokens(&body, started, &previous.scopes, false)?;
        if replacement.refresh_token.is_none() {
            replacement.refresh_token = Some(Secret::new(refresh.to_owned()));
        }
        if replacement.refresh_token() == previous.refresh_token() {
            replacement.refresh_expires_at = match (previous.refresh_expires_at, replacement.refresh_expires_at) {
                (Some(old), Some(new)) => Some(old.min(new)),
                (Some(old), None) => Some(old),
                (_, new) => new,
            };
        }
        Ok(replacement)
    }

    /// Provider acknowledgement is not a local grant-revocation barrier. The
    /// connection owner fences grants first; Google may propagate revocation later.
    pub async fn revoke(&self, tokens: &OAuthTokens) -> Result<(), OAuthTokenError> {
        self.check_registration(tokens)?;
        let token = tokens.refresh_token().unwrap_or_else(|| tokens.access_token.expose());
        self.post_form(&self.revoke_endpoint, &[("token", token)], false)
            .await?;
        Ok(())
    }

    fn check_registration(&self, tokens: &OAuthTokens) -> Result<(), OAuthTokenError> {
        if tokens.client_id == self.registration.client_id {
            Ok(())
        } else {
            Err(OAuthTokenError::RegistrationMismatch)
        }
    }

    async fn post_form(
        &self,
        endpoint: &Url,
        fields: &[(&str, &str)],
        include_secret: bool,
    ) -> Result<Zeroizing<Vec<u8>>, OAuthTokenError> {
        if fields
            .iter()
            .any(|(_, value)| value.len() > self.policy.max_request_bytes)
        {
            return Err(OAuthTokenError::RequestTooLarge);
        }
        let form = {
            let mut serializer = url::form_urlencoded::Serializer::new(String::new());
            serializer.extend_pairs(fields.iter().copied());
            if include_secret {
                if let Some(secret) = &self.registration.client_secret {
                    serializer.append_pair("client_secret", secret.expose());
                }
            }
            SecretBody(Zeroizing::new(serializer.finish().into_bytes()))
        };
        if form.as_ref().len() > self.policy.max_request_bytes {
            return Err(OAuthTokenError::RequestTooLarge);
        }
        let bytes = bytes::Bytes::from_owner(form);
        let response = self
            .http
            .post(endpoint.clone())
            .header("content-type", "application/x-www-form-urlencoded")
            .body(bytes)
            .send()
            .await
            .map_err(network_error)?;
        self.read_response(response).await
    }

    async fn read_response(&self, mut response: reqwest::Response) -> Result<Zeroizing<Vec<u8>>, OAuthTokenError> {
        if response
            .content_length()
            .is_some_and(|length| length > self.policy.max_response_bytes as u64)
        {
            return Err(OAuthTokenError::ResponseTooLarge);
        }
        let status = response.status();
        let mut body = Zeroizing::new(Vec::new());
        while let Some(chunk) = response.chunk().await.map_err(network_error)? {
            if chunk.len() > self.policy.max_response_bytes.saturating_sub(body.len()) {
                return Err(OAuthTokenError::ResponseTooLarge);
            }
            body.extend_from_slice(&chunk);
        }
        if status.as_u16() != 200 {
            let code = serde_json::from_slice::<ProviderFailure>(&body)
                .ok()
                .and_then(|failure| failure.error)
                .and_then(|error| provider_error(error.expose()));
            return Err(OAuthTokenError::ProviderRejected {
                status: status.as_u16(),
                code,
            });
        }
        Ok(body)
    }

    fn tokens(
        &self,
        body: &[u8],
        started: Instant,
        allowed: &BTreeSet<String>,
        require_scope: bool,
    ) -> Result<OAuthTokens, OAuthTokenError> {
        let response: TokenResponse = serde_json::from_slice(body).map_err(|_| OAuthTokenError::InvalidResponse)?;
        if !response.token_type.eq_ignore_ascii_case("Bearer")
            || !valid_token(response.access_token.expose())
            || response
                .refresh_token
                .as_ref()
                .is_some_and(|value| !valid_token(value.expose()))
            || response
                .id_token
                .as_ref()
                .is_some_and(|value| !valid_token(value.expose()))
        {
            return Err(OAuthTokenError::InvalidResponse);
        }
        let scopes = match response.scope {
            Some(scope) => {
                if scope.bytes().any(|b| b.is_ascii_control()) {
                    return Err(OAuthTokenError::InvalidResponse);
                }
                let mut result = BTreeSet::new();
                for value in scope.split(' ').filter(|scope| !scope.is_empty()) {
                    if !valid_scope(value) {
                        return Err(OAuthTokenError::InvalidResponse);
                    }
                    result.insert(canonical_scope(value).to_owned());
                }
                result
            }
            None if !require_scope => allowed.clone(),
            None => return Err(OAuthTokenError::InvalidResponse),
        };
        if !scopes.is_subset(allowed) {
            return Err(OAuthTokenError::ScopeDenied);
        }
        let expires_at = expiry(started, response.expires_in)?;
        let refresh_expires_at = response
            .refresh_token_expires_in
            .map(|seconds| expiry(started, seconds))
            .transpose()?;
        Ok(OAuthTokens {
            client_id: self.registration.client_id.clone(),
            access_token: response.access_token,
            refresh_token: response.refresh_token,
            id_token: response.id_token,
            expected_nonce: None,
            expires_at,
            refresh_expires_at,
            scopes,
        })
    }
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: Secret,
    expires_in: u64,
    token_type: String,
    scope: Option<String>,
    refresh_token: Option<Secret>,
    id_token: Option<Secret>,
    refresh_token_expires_in: Option<u64>,
}

#[derive(Deserialize)]
struct ProviderFailure {
    error: Option<Secret>,
}

struct SecretBody(Zeroizing<Vec<u8>>);
impl AsRef<[u8]> for SecretBody {
    fn as_ref(&self) -> &[u8] {
        self.0.as_slice()
    }
}

fn build_http(policy: OAuthHttpPolicy, https_only: bool) -> Result<reqwest::Client, OAuthTokenError> {
    let builder = reqwest::Client::builder()
        .https_only(https_only)
        .redirect(reqwest::redirect::Policy::none())
        .retry(reqwest::retry::never())
        .referer(false)
        .timeout(policy.request_timeout);
    #[cfg(test)]
    let builder = builder.no_proxy();
    builder.build().map_err(|_| OAuthTokenError::Network)
}

fn network_error(error: reqwest::Error) -> OAuthTokenError {
    if error.is_timeout() {
        OAuthTokenError::Timeout
    } else {
        OAuthTokenError::Network
    }
}

fn expiry(started: Instant, seconds: u64) -> Result<Instant, OAuthTokenError> {
    if seconds == 0 {
        return Err(OAuthTokenError::InvalidResponse);
    }
    started
        .checked_add(Duration::from_secs(seconds))
        .ok_or(OAuthTokenError::InvalidResponse)
}

fn valid_token(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|b| (0x21..=0x7e).contains(&b))
}
fn valid_scope(value: &str) -> bool {
    valid_token(value) && !value.contains(['"', '\\'])
}
fn base64url_nonce(value: &str) -> bool {
    value.len() == 43 && value.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
}
fn canonical_scope(value: &str) -> &str {
    match value {
        "email" => "https://www.googleapis.com/auth/userinfo.email",
        "profile" => "https://www.googleapis.com/auth/userinfo.profile",
        other => other,
    }
}
fn provider_error(value: &str) -> Option<OAuthProviderError> {
    match value {
        "invalid_grant" => Some(OAuthProviderError::InvalidGrant),
        "invalid_client" => Some(OAuthProviderError::InvalidClient),
        "invalid_scope" => Some(OAuthProviderError::InvalidScope),
        "invalid_request" => Some(OAuthProviderError::InvalidRequest),
        "temporarily_unavailable" => Some(OAuthProviderError::TemporarilyUnavailable),
        "invalid_dpop_proof" | "use_dpop_nonce" => Some(OAuthProviderError::ProofRequired),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
