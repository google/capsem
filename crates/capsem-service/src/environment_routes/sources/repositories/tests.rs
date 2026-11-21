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
    let policy = "[corp.rules.other]\nname='other'\naction='block'\npriority=-10\nmatch='http.method == \"POST\"'\n";
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
