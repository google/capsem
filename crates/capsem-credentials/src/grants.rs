//! Host-only admission. Bindings and qualified capabilities come from the
//! service's trusted registry, never a guest or a caller's request body.
use std::collections::{BTreeSet, HashMap};
use std::fmt;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::GoogleConnection;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrantError {
    Invalid,
    Denied,
    Capacity,
    Unavailable,
}
impl fmt::Display for GrantError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Invalid => "invalid credential grant",
            Self::Denied => "credential grant denied",
            Self::Capacity => "credential grant capacity exceeded",
            Self::Unavailable => "credential grant unavailable",
        })
    }
}
impl std::error::Error for GrantError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GrantSession {
    session: [u8; 16],
    ownership: u64,
    runtime: u64,
}
impl GrantSession {
    pub fn new(session: [u8; 16], ownership: u64, runtime: u64) -> Result<Self, GrantError> {
        if session == [0; 16] || ownership == 0 || runtime == 0 {
            return Err(GrantError::Invalid);
        }
        Ok(Self {
            session,
            ownership,
            runtime,
        })
    }
}

/// Exact method and URL, including port, path and query. No redirects or
/// origin/path-prefix inference is authorized by this metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrantTarget {
    method: String,
    url: url::Url,
}
impl GrantTarget {
    pub fn new(method: &str, destination: &str) -> Result<Self, GrantError> {
        if !matches!(method, "GET" | "POST" | "PUT" | "PATCH" | "DELETE" | "HEAD" | "OPTIONS") {
            return Err(GrantError::Invalid);
        }
        let url = url::Url::parse(destination).map_err(|_| GrantError::Invalid)?;
        if url.scheme() != "https"
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
        {
            return Err(GrantError::Invalid);
        }
        Ok(Self {
            method: method.into(),
            url,
        })
    }
}
#[derive(Debug, Clone)]
pub struct GrantCapability {
    id: String,
    scopes: BTreeSet<String>,
    targets: Vec<GrantTarget>,
}
impl GrantCapability {
    pub fn new(id: String, scopes: BTreeSet<String>, targets: Vec<GrantTarget>) -> Result<Self, GrantError> {
        if id.is_empty()
            || id.len() > 128
            || !id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'.' | b'_' | b'-'))
            || scopes.is_empty()
            || scopes
                .iter()
                .any(|s| s.is_empty() || s.len() > 2048 || s.bytes().any(|c| c.is_ascii_control() || c == b' '))
            || targets.is_empty()
        {
            return Err(GrantError::Invalid);
        }
        Ok(Self { id, scopes, targets })
    }
    pub fn id(&self) -> &str {
        &self.id
    }
}
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct GrantHandle([u8; 32]);
impl fmt::Debug for GrantHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("GrantHandle([REDACTED])")
    }
}
#[derive(Debug, Clone, Copy)]
pub struct GrantLimits {
    pub max_grants: usize,
    pub max_lifetime: Duration,
}
#[derive(Debug, Clone, Copy)]
pub enum GrantStopPolicy {
    Revoke,
    Preserve,
}
#[derive(Clone)]
struct Grant {
    connection: GoogleConnection,
    generation: u64,
    session: GrantSession,
    capability: GrantCapability,
    expires: Instant,
    active: bool,
}
pub struct GrantAuthority {
    limits: GrantLimits,
    grants: Mutex<HashMap<GrantHandle, Arc<Grant>>>,
}
impl GrantAuthority {
    pub fn new(limits: GrantLimits) -> Result<Self, GrantError> {
        if limits.max_grants == 0 || limits.max_lifetime.is_zero() {
            return Err(GrantError::Invalid);
        }
        Ok(Self {
            limits,
            grants: Mutex::new(HashMap::new()),
        })
    }
    /// The policy callback supplies the current corporate/user decision.
    /// No consumer qualification or approval is inferred from token presence.
    pub fn issue(
        &self,
        connection: GoogleConnection,
        session: GrantSession,
        capability: GrantCapability,
        lifetime: Duration,
        now: Instant,
        policy: impl Fn(&GrantSession, &GrantCapability) -> bool,
    ) -> Result<GrantHandle, GrantError> {
        if lifetime.is_zero() || lifetime > self.limits.max_lifetime {
            return Err(GrantError::Invalid);
        }
        if !policy(&session, &capability) {
            return Err(GrantError::Denied);
        }
        let authorization = connection.authorization().map_err(|_| GrantError::Denied)?;
        if !capability.scopes.is_subset(&authorization.granted_scopes) {
            return Err(GrantError::Denied);
        }
        let expires = now.checked_add(lifetime).ok_or(GrantError::Invalid)?;
        let mut grants = self.grants.lock().map_err(|_| GrantError::Unavailable)?;
        grants.retain(|_, grant| grant.expires > now);
        if grants.len() >= self.limits.max_grants {
            return Err(GrantError::Capacity);
        }
        let handle = Self::new_handle(&grants)?;
        grants.insert(
            handle,
            Arc::new(Grant {
                connection,
                generation: authorization.generation,
                session,
                capability,
                expires,
                active: true,
            }),
        );
        drop(grants);
        Ok(handle)
    }
    fn new_handle(grants: &HashMap<GrantHandle, Arc<Grant>>) -> Result<GrantHandle, GrantError> {
        let mut bytes = [0; 32];
        getrandom::fill(&mut bytes).map_err(|_| GrantError::Unavailable)?;
        let handle = GrantHandle(bytes);
        if grants.contains_key(&handle) {
            return Err(GrantError::Unavailable);
        }
        Ok(handle)
    }
    fn consent_current(grant: &Grant) -> bool {
        grant
            .connection
            .authorization()
            .is_ok_and(|a| a.generation == grant.generation && grant.capability.scopes.is_subset(&a.granted_scopes))
    }
    fn admits(
        grant: &Grant,
        session: GrantSession,
        target: &GrantTarget,
        now: Instant,
        policy: &impl Fn(&GrantSession, &GrantCapability) -> bool,
    ) -> bool {
        grant.active
            && grant.session == session
            && now < grant.expires
            && grant.capability.targets.contains(target)
            && policy(&session, &grant.capability)
            && Self::consent_current(grant)
    }
    /// The callback must be short synchronous host work and must not reenter
    /// this authority. Its admission is serialized with revoke and detach.
    pub async fn with_token<T>(
        &self,
        handle: GrantHandle,
        session: GrantSession,
        target: &GrantTarget,
        now: Instant,
        policy: impl Fn(&GrantSession, &GrantCapability) -> bool,
        use_token: impl FnOnce(&str) -> T,
    ) -> Result<T, GrantError> {
        let grant = {
            let grants = self.grants.lock().map_err(|_| GrantError::Unavailable)?;
            let grant = grants.get(&handle).ok_or(GrantError::Denied)?;
            if !Self::admits(grant, session, target, now, &policy) {
                return Err(GrantError::Denied);
            }
            let admitted = Arc::clone(grant);
            drop(grants);
            admitted
        };
        let lease = grant.connection.access(now).await.map_err(|_| GrantError::Denied)?;
        let grants = self.grants.lock().map_err(|_| GrantError::Unavailable)?;
        let current = grants.get(&handle).ok_or(GrantError::Denied)?;
        let checked_now = now.max(Instant::now());
        if !Arc::ptr_eq(current, &grant) || !Self::admits(current, session, target, checked_now, &policy) {
            return Err(GrantError::Denied);
        }
        let result = lease
            .with_authorization(checked_now, grant.generation, &grant.capability.scopes, use_token)
            .map_err(|_| GrantError::Denied);
        drop(grants);
        result
    }
    pub fn revoke(&self, handle: GrantHandle) -> Result<(), GrantError> {
        self.grants.lock().map_err(|_| GrantError::Unavailable)?.remove(&handle);
        Ok(())
    }
    pub fn detach(&self, session: GrantSession, policy: GrantStopPolicy) -> Result<(), GrantError> {
        let mut grants = self.grants.lock().map_err(|_| GrantError::Unavailable)?;
        match policy {
            GrantStopPolicy::Revoke => grants.retain(|_, g| g.session != session),
            GrantStopPolicy::Preserve => {
                for grant in grants.values_mut().filter(|g| g.session == session) {
                    let mut detached = (**grant).clone();
                    detached.active = false;
                    *grant = Arc::new(detached);
                }
            }
        }
        Ok(())
    }
    pub fn rebind(
        &self,
        handle: GrantHandle,
        session: GrantSession,
        now: Instant,
        policy: impl Fn(&GrantSession, &GrantCapability) -> bool,
    ) -> Result<GrantHandle, GrantError> {
        let mut grants = self.grants.lock().map_err(|_| GrantError::Unavailable)?;
        let grant = grants.get(&handle).ok_or(GrantError::Denied)?;
        if grant.active
            || grant.session.session != session.session
            || grant.session.ownership != session.ownership
            || grant.session.runtime == session.runtime
            || now >= grant.expires
            || !policy(&session, &grant.capability)
            || !Self::consent_current(grant)
        {
            return Err(GrantError::Denied);
        }
        let mut rebound = (**grant).clone();
        rebound.session = session;
        rebound.active = true;
        let fresh = Self::new_handle(&grants)?;
        grants.remove(&handle);
        grants.insert(fresh, Arc::new(rebound));
        drop(grants);
        Ok(fresh)
    }
}

#[cfg(test)]
mod tests;
