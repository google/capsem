use super::*;

#[test]
fn prepared_publication_state_does_not_reopen_the_session_root() {
    let directory = tempfile::tempdir().unwrap();
    let absent_session = directory.path().join("absent-session");
    Publisher::for_prepared_session(&absent_session, capsem_config::router::RouterConfig::default()).unwrap();
}
