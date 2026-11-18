use super::super::advertisement::tests::advertisement;
use super::*;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Mutex,
};
mod server;
use server::{Reply, Server};

struct Authority {
    addresses: Vec<SocketAddr>,
    observed: Arc<Mutex<Vec<String>>>,
    refuse: bool,
}
impl RepositoryNetworkAuthority for Authority {
    fn admit<'a>(&'a self, url: &'a Url) -> RepositoryAdmission<'a> {
        self.observed.lock().unwrap().push(url.to_string());
        let result = if self.refuse {
            Err(Error::Refused)
        } else {
            Ok(self.addresses.clone())
        };
        Box::pin(async move { result })
    }
}
fn resolver(server: &Server, refuse: bool) -> (RepositoryHttpsResolver, Arc<Mutex<Vec<String>>>) {
    let observed = Arc::new(Mutex::new(Vec::new()));
    let authority = Arc::new(Authority {
        addresses: vec![server.address],
        observed: observed.clone(),
        refuse,
    });
    (
        RepositoryHttpsResolver::new(authority)
            .with_root_certificate(server.pem.as_bytes())
            .unwrap(),
        observed,
    )
}
fn bytes() -> Vec<u8> {
    advertisement(
        &[
            ("HEAD", 'a'),
            ("refs/heads/main", 'a'),
            ("refs/tags/release", 'b'),
            ("refs/tags/release^{}", 'c'),
        ],
        "sha1",
    )
}

#[tokio::test]
async fn native_https_uses_only_admitted_addresses_and_sends_anonymous_git_discovery() {
    let server = Server::start(Reply::Bytes(bytes())).await;
    let (client, observed) = resolver(&server, false);
    assert_eq!(
        client.resolve(&server.repository(), "release").await.unwrap(),
        "c".repeat(40)
    );
    assert_eq!(
        observed.lock().unwrap().as_slice(),
        [format!("{}/info/refs?service=git-upload-pack", server.repository())]
    );
    let (method, uri, headers) = {
        let mut requests = server.requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        requests.pop().unwrap()
    };
    assert_eq!(method, http::Method::GET);
    assert_eq!(uri, "/team/project.git/info/refs?service=git-upload-pack");
    assert_eq!(headers["git-protocol"], "version=1");
    assert!(!headers.contains_key("authorization"));
    assert!(!headers.contains_key("proxy-authorization"));
    assert!(!headers.contains_key("cookie"));
}

#[tokio::test]
async fn invalid_or_unadmitted_targets_never_contact_the_registry() {
    let server = Server::start(Reply::Bytes(bytes())).await;
    let (client, observed) = resolver(&server, true);
    assert_eq!(client.resolve(&server.repository(), "main").await, Err(Error::Refused));
    assert_eq!(observed.lock().unwrap().len(), 1);
    for invalid in [
        "http://public.example/repo",
        "ssh://git@public.example/repo",
        "file:///repository",
        "https://user:secret@public.example/repo",
        "https://public.example/repo?token=secret",
        "https://public.example/repo#fragment",
    ] {
        assert_eq!(client.resolve(invalid, "main").await, Err(Error::InvalidIntent));
    }
    assert_eq!(
        client.resolve(&server.repository(), "-c bad").await,
        Err(Error::InvalidIntent)
    );
    assert_eq!(observed.lock().unwrap().len(), 1);
    assert!(server.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn redirects_cannot_bypass_admission_or_forward_to_another_target() {
    let destination = Server::start(Reply::Bytes(bytes())).await;
    let redirect = Server::start(Reply::Redirect(destination.repository())).await;
    let (client, observed) = resolver(&redirect, false);
    assert_eq!(
        client.resolve(&redirect.repository(), "main").await,
        Err(Error::Refused)
    );
    assert_eq!(observed.lock().unwrap().len(), 1);
    assert_eq!(redirect.requests.lock().unwrap().len(), 1);
    assert!(destination.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn content_type_and_http_failures_are_explicit_without_fallback_or_retry() {
    for (reply, expected) in [
        (Reply::WrongType(bytes()), Error::InvalidAdvertisement),
        (Reply::Status(401), Error::Refused),
        (Reply::Status(403), Error::Refused),
        (Reply::Status(404), Error::Unavailable),
        (Reply::Status(500), Error::Unavailable),
    ] {
        let server = Server::start(reply).await;
        let (client, _) = resolver(&server, false);
        assert_eq!(client.resolve(&server.repository(), "main").await, Err(expected));
        assert_eq!(server.requests.lock().unwrap().len(), 1);
    }
}

#[tokio::test]
async fn declared_and_streamed_oversize_are_refused_before_any_pin_is_returned() {
    let maximum = capsem_proto::ENVIRONMENT_MAX_REPOSITORY_ADVERTISEMENT_BYTES;
    for reply in [
        Reply::Bytes(vec![b'x'; maximum + 1]),
        Reply::Stream(vec![b'x'; maximum + 1]),
    ] {
        let server = Server::start(reply).await;
        let (client, _) = resolver(&server, false);
        assert_eq!(client.resolve(&server.repository(), "main").await, Err(Error::TooLarge));
        assert_eq!(server.requests.lock().unwrap().len(), 1);
    }
}

#[tokio::test]
async fn the_total_deadline_includes_a_body_that_never_finishes() {
    let ready = Arc::new(tokio::sync::Notify::new());
    let server = Server::start(Reply::PendingBody(bytes(), ready.clone())).await;
    let (client, _) = resolver(&server, false);
    let url = server.repository();
    let task = tokio::spawn(async move { client.resolve(&url, "main").await });
    ready.notified().await;
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(
        capsem_proto::ENVIRONMENT_MAX_REPOSITORY_RESOLVE_SECONDS as u64,
    ))
    .await;
    tokio::time::resume();
    assert_eq!(task.await.unwrap(), Err(Error::Deadline));
    assert_eq!(server.requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn the_total_deadline_includes_network_admission_without_any_http_attempt() {
    struct Pending {
        entered: Arc<tokio::sync::Notify>,
        calls: Arc<AtomicUsize>,
    }
    impl RepositoryNetworkAuthority for Pending {
        fn admit<'a>(&'a self, _: &'a Url) -> RepositoryAdmission<'a> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.entered.notify_one();
            Box::pin(std::future::pending())
        }
    }
    let entered = Arc::new(tokio::sync::Notify::new());
    let calls = Arc::new(AtomicUsize::new(0));
    let client = RepositoryHttpsResolver::new(Arc::new(Pending {
        entered: entered.clone(),
        calls: calls.clone(),
    }));
    let task = tokio::spawn(async move { client.resolve("https://public.example/project.git", "main").await });
    entered.notified().await;
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(
        capsem_proto::ENVIRONMENT_MAX_REPOSITORY_RESOLVE_SECONDS as u64,
    ))
    .await;
    tokio::time::resume();
    assert_eq!(task.await.unwrap(), Err(Error::Deadline));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn encoded_repository_paths_keep_the_exact_admitted_discovery_target() {
    let server = Server::start(Reply::Bytes(bytes())).await;
    let (client, observed) = resolver(&server, false);
    for path in ["/team/space%20name.git", "/team/encoded%2Fname.git/"] {
        let base = format!("https://git.fixture.invalid:{}{path}", server.address.port());
        client.resolve(&base, "main").await.unwrap();
        let expected = format!("{}/info/refs?service=git-upload-pack", base.trim_end_matches('/'));
        assert_eq!(observed.lock().unwrap().last(), Some(&expected));
        let expected_path = format!("{}/info/refs?service=git-upload-pack", path.trim_end_matches('/'));
        assert_eq!(server.requests.lock().unwrap().last().unwrap().1, expected_path);
    }
}

#[tokio::test]
async fn pinned_admission_does_not_disable_tls_certificate_verification() {
    let server = Server::start(Reply::Bytes(bytes())).await;
    let observed = Arc::new(Mutex::new(Vec::new()));
    let client = RepositoryHttpsResolver::new(Arc::new(Authority {
        addresses: vec![server.address],
        observed,
        refuse: false,
    }));
    assert_eq!(
        client.resolve(&server.repository(), "main").await,
        Err(Error::Unavailable)
    );
    assert!(server.requests.lock().unwrap().is_empty());
}

#[tokio::test]
async fn isolated_environment_proxy_and_git_configuration_are_not_consulted() {
    let directory = tempfile::tempdir().unwrap();
    let configuration = directory.path().join("git-config");
    std::fs::write(
        &configuration,
        "[http]\nextraHeader = Authorization: Bearer fixture-config-canary\n",
    )
    .unwrap();
    let output = tokio::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "environment::repositories::https::tests::proxy_environment_child_probe",
            "--nocapture",
        ])
        .env("CAPSEM_REPOSITORY_PROXY_TEST_CHILD", "1")
        .env("HTTPS_PROXY", "http://127.0.0.1:1")
        .env("https_proxy", "http://127.0.0.1:1")
        .env("HTTP_PROXY", "http://127.0.0.1:1")
        .env("http_proxy", "http://127.0.0.1:1")
        .env("ALL_PROXY", "http://127.0.0.1:1")
        .env("all_proxy", "http://127.0.0.1:1")
        .env("NO_PROXY", "")
        .env("no_proxy", "")
        .env("GIT_CONFIG_GLOBAL", configuration)
        .kill_on_drop(true)
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
}

#[tokio::test]
async fn proxy_environment_child_probe() {
    if std::env::var("CAPSEM_REPOSITORY_PROXY_TEST_CHILD").as_deref() != Ok("1") {
        return;
    }
    let server = Server::start(Reply::Bytes(bytes())).await;
    let (client, _) = resolver(&server, false);
    assert_eq!(
        client.resolve(&server.repository(), "main").await.unwrap(),
        "a".repeat(40)
    );
    let requests = server.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert!(!requests[0].2.contains_key("authorization"));
    drop(requests);
}
