use super::*;
use std::process::Command;

const CHILD_NAME: &str = "CAPSEM_TEST_ACTIVATION_BUS_NAME";

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn second_process_delivers_exact_deep_link_and_primary_can_restart() {
    let name = format!("com.capsem.activation_test.p{}", std::process::id());
    let (sender, mut receiver) = tokio::sync::mpsc::unbounded_channel();
    let primary = admit(&name, Vec::new(), String::new(), move |args, cwd| {
        sender.send((args, cwd)).unwrap();
    })
    .await
    .unwrap();
    let Admission::Primary {
        connection,
        name: owned_name,
    } = primary
    else {
        panic!("private test name should be acquired");
    };
    let executable = std::env::current_exe().unwrap();
    let status = Command::new(&executable)
        .args(["--exact", "platform_impl::tests::second_launch_child", "--nocapture"])
        .env(CHILD_NAME, &name)
        .status()
        .unwrap();
    assert!(status.success());
    let (args, cwd) = tokio::time::timeout(Duration::from_secs(5), receiver.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        args,
        ["capsem-app", "--connect", "vm with spaces", "--action", "inspect"]
    );
    assert_eq!(cwd, "/tmp/workspace with spaces");
    assert!(receiver.try_recv().is_err(), "activation must not be retried");
    assert!(connection.release_name(owned_name.as_str()).await.unwrap());
    assert!(matches!(
        admit(&name, vec![], String::new(), |_, _| {}).await.unwrap(),
        Admission::Primary { .. }
    ));
}

#[tokio::test]
async fn second_launch_child() {
    let Ok(name) = std::env::var(CHILD_NAME) else {
        return;
    };
    let result = admit(
        &name,
        ["capsem-app", "--connect", "vm with spaces", "--action", "inspect"]
            .map(String::from)
            .to_vec(),
        "/tmp/workspace with spaces".into(),
        |_, _| panic!("secondary must never own the activation callback"),
    )
    .await
    .unwrap();
    assert!(matches!(result, Admission::Forwarded));
}

#[tokio::test]
async fn invalid_bus_identity_fails_instead_of_starting_a_second_instance() {
    assert!(admit("invalid identifier", vec![], String::new(), |_, _| {})
        .await
        .is_err());
}
