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
mod tests {
    use super::*;
    fn authority(policy: &str, ips: &[&str]) -> Authority {
        let resolver = UpstreamResolver::disabled().with_fixed_answer(
            "git.fixture.invalid",
            ips.iter().map(|ip| ip.parse().unwrap()).collect(),
        );
        Authority::new(&toml::from_str(policy).unwrap(), resolver).unwrap()
    }
    fn url() -> reqwest::Url {
        "https://git.fixture.invalid/team/repo/info/refs?service=git-upload-pack"
            .parse()
            .unwrap()
    }
    #[tokio::test]
    async fn public_answers_are_returned_exactly_with_the_requested_port() {
        let authority = authority("", &["8.8.8.8", "2606:4700:4700::1111"]);
        let mut discovery = url();
        discovery.set_port(Some(8443)).unwrap();
        assert_eq!(
            authority.admit(&discovery).await.unwrap(),
            vec![
                "8.8.8.8:8443".parse().unwrap(),
                "[2606:4700:4700::1111]:8443".parse().unwrap()
            ]
        );
    }
    #[tokio::test]
    async fn any_nonpublic_answer_refuses_the_whole_discovery() {
        for ip in [
            "127.0.0.1",
            "10.0.0.1",
            "169.254.169.254",
            "100.64.0.1",
            "192.0.2.1",
            "::1",
            "::ffff:127.0.0.1",
            "fc00::1",
            "64:ff9b::a00:1",
        ] {
            assert_eq!(
                authority("", &["8.8.8.8", ip]).admit(&url()).await,
                Err(Error::Refused),
                "{ip}"
            );
        }
    }
    #[tokio::test]
    async fn corporate_rules_see_exact_discovery_http_and_each_socket() {
        for condition in [
            "http.host == \"git.fixture.invalid\"",
            "http.method == \"GET\"",
            "http.path == \"/team/repo/info/refs\"",
            "http.query == \"service=git-upload-pack\"",
            "tcp.port == \"443\"",
            "ip.value == \"1.1.1.1\"",
        ] {
            for action in ["block", "ask"] {
                let policy = format!(
                    "[corp.rules.discovery]\nname='discovery'\naction='{action}'\npriority=-10\nmatch='{condition}'\n"
                );
                assert_eq!(
                    authority(&policy, &["8.8.8.8", "1.1.1.1"]).admit(&url()).await,
                    Err(Error::Refused),
                    "{condition}: {action}"
                );
            }
        }
        let policy =
            "[corp.rules.other]\nname='other'\naction='block'\npriority=-10\nmatch='http.method == \"POST\"'\n";
        assert!(authority(policy, &["8.8.8.8"]).admit(&url()).await.is_ok());
    }
    #[tokio::test]
    async fn missing_dns_never_grants_discovery() {
        assert_eq!(authority("", &[]).admit(&url()).await, Err(Error::Unavailable));
    }
    #[tokio::test]
    async fn connected_service_source_refuses_host_sockets_and_unsupported_protocols() {
        let sources = Sources(crate::tests::make_test_state());
        for url in [
            "https://127.0.0.1/repo",
            "https://[::1]/repo",
            "https://169.254.169.254/repo",
        ] {
            assert_eq!(
                sources.repository(url, "main").await,
                Err(EnvironmentResolutionError::Refused)
            );
        }
        assert_eq!(
            sources.repository("ssh://git@git.fixture.invalid/repo", "main").await,
            Err(EnvironmentResolutionError::InvalidIntent)
        );
    }
    #[tokio::test]
    async fn plugin_ask_and_block_refuse_discovery() {
        for mode in ["ask", "block"] {
            let policy = format!("[plugins.dummy_post_allow]\nmode='{mode}'\n");
            assert_eq!(
                authority(&policy, &["8.8.8.8"]).admit(&url()).await,
                Err(Error::Refused)
            );
        }
    }
    #[test]
    fn invalid_corporate_rules_never_fall_back_to_open_policy() {
        let corporate =
            toml::from_str("[corp.rules.bad]\nname='bad'\naction='block'\nmatch='no_such_subject.valid'\n").unwrap();
        assert!(Authority::new(&corporate, UpstreamResolver::disabled()).is_err());
    }
}
