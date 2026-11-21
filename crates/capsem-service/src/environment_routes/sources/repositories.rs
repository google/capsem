//! Operator metadata discovery uses current corporate HTTP policy and only
//! public sockets. No proposed blueprint rule, Git config or host identity
//! grants access. The HTTPS owner pins this exact answer for its single dial.
use super::*;
use capsem_core::{
    environment::{
        RepositoryAdmission, RepositoryHttpsResolver, RepositoryNetworkAuthority, RepositoryResolutionError,
    },
    net::{
        policy_config::{ActivePolicyFile, SecurityPluginConfig, SecurityRuleSet},
        upstream_address::{is_public_address, UpstreamResolver},
    },
    security_engine::{
        evaluate_security_boundary, HttpRequestSecurityEvent, HttpSecurityEvent, IpSecurityEvent,
        RuntimeSecurityEventType, SecurityEvent, TcpSecurityEvent,
    },
};
use std::collections::BTreeMap;

type Error = RepositoryResolutionError;
struct Authority {
    rules: SecurityRuleSet,
    plugins: Arc<BTreeMap<String, SecurityPluginConfig>>,
    resolver: UpstreamResolver,
}
impl Authority {
    fn new(corporate: &capsem_config::SettingsFile, resolver: UpstreamResolver) -> Result<Self, Error> {
        let active =
            ActivePolicyFile::from_settings_and_corp(&Default::default(), corporate).map_err(|_| Error::Refused)?;
        Ok(Self {
            rules: active.compile_security_rule_set().map_err(|_| Error::Refused)?,
            plugins: Arc::new(active.plugins),
            resolver,
        })
    }
}
impl RepositoryNetworkAuthority for Authority {
    fn admit<'a>(&'a self, discovery: &'a reqwest::Url) -> RepositoryAdmission<'a> {
        Box::pin(async move {
            if discovery.scheme() != "https" || !discovery.username().is_empty() || discovery.password().is_some() {
                return Err(Error::Refused);
            }
            let host = discovery.host_str().ok_or(Error::InvalidIntent)?;
            let resolver_host = host
                .strip_prefix('[')
                .and_then(|host| host.strip_suffix(']'))
                .unwrap_or(host);
            let port = discovery.port_or_known_default().ok_or(Error::InvalidIntent)?;
            let addresses = self
                .resolver
                .resolve(resolver_host, port)
                .await
                .map_err(|_| Error::Unavailable)?;
            if addresses.is_empty() || addresses.iter().any(|address| !is_public_address(address.ip())) {
                return Err(Error::Refused);
            }
            for address in &addresses {
                let mut headers = http::HeaderMap::new();
                headers.insert(
                    http::header::ACCEPT,
                    http::HeaderValue::from_static("application/x-git-upload-pack-advertisement"),
                );
                headers.insert("Git-Protocol", http::HeaderValue::from_static("version=1"));
                headers.insert(http::header::CACHE_CONTROL, http::HeaderValue::from_static("no-cache"));
                let event = SecurityEvent::new(RuntimeSecurityEventType::HttpRequest)
                    .with_http(HttpSecurityEvent {
                        host: Some(host.into()),
                        method: Some("GET".into()),
                        path: Some(discovery.path().into()),
                        query: discovery.query().map(str::to_owned),
                        ..Default::default()
                    })
                    .with_http_request(HttpRequestSecurityEvent::new(
                        host,
                        None,
                        headers,
                        discovery.query().map(str::to_owned),
                    ))
                    .with_ip(IpSecurityEvent {
                        value: Some(address.ip().to_string()),
                        version: Some(if address.is_ipv4() { "4" } else { "6" }.into()),
                    })
                    .with_tcp(TcpSecurityEvent {
                        port: Some(port.to_string()),
                    });
                let evaluated = evaluate_security_boundary(&self.rules, Arc::clone(&self.plugins), event)
                    .map_err(|_| Error::Refused)?;
                // An ask has no approved host metadata continuation. It must
                // never be treated as permission to contact this upstream.
                if !evaluated.enforcement.is_allowed() {
                    return Err(Error::Refused);
                }
            }
            Ok(addresses)
        })
    }
}

pub(super) async fn resolve(
    state: &Arc<ServiceState>,
    repository: &str,
    reference: &str,
) -> Result<String, EnvironmentResolutionError> {
    let budget = std::time::Duration::from_secs(capsem_proto::ENVIRONMENT_MAX_REPOSITORY_RESOLVE_SECONDS as u64);
    tokio::time::timeout(budget, resolve_inner(state, repository, reference))
        .await
        .map_err(|_| EnvironmentResolutionError::Unavailable)?
}
async fn resolve_inner(
    state: &Arc<ServiceState>,
    repository: &str,
    reference: &str,
) -> Result<String, EnvironmentResolutionError> {
    let corporate = state
        .containers
        .environment_catalog_policy()
        .await
        .map_err(|_| EnvironmentResolutionError::Unavailable)?;
    let (authority, corporate) = state
        .off_worker(move |_| {
            Authority::new(&corporate, UpstreamResolver::system()).map(|authority| (authority, corporate))
        })
        .await
        .map_err(|_| EnvironmentResolutionError::Unavailable)?
        .map_err(resolution_error)?;
    let pin = RepositoryHttpsResolver::new(Arc::new(authority))
        .resolve(repository, reference)
        .await
        .map_err(resolution_error)?;
    let current = state
        .containers
        .environment_catalog_policy()
        .await
        .map_err(|_| EnvironmentResolutionError::Unavailable)?;
    // No second DNS lookup or refresh: a policy change requires a new resolve.
    if current != corporate {
        return Err(EnvironmentResolutionError::Refused);
    }
    Ok(pin)
}
fn resolution_error(error: Error) -> EnvironmentResolutionError {
    match error {
        Error::InvalidIntent | Error::UnknownReference | Error::AmbiguousReference => {
            EnvironmentResolutionError::InvalidIntent
        }
        Error::Refused => EnvironmentResolutionError::Refused,
        Error::TooLarge | Error::InvalidAdvertisement => EnvironmentResolutionError::InvalidPin,
        Error::Unavailable | Error::Deadline => EnvironmentResolutionError::Unavailable,
    }
}

#[cfg(test)]
mod tests;
