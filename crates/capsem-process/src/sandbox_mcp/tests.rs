
use super::*;
use capsem_core::environment::EnvironmentPolicyInputs;
use capsem_core::net::policy_config::ActivePolicyFile;

fn policy(argument: &str, enabled: bool) -> Vec<u8> {
    let boot = toml::from_str(&format!(
        r#"
name = "sandbox-mcp"
[image]
reference = "redis"
resolved_digest = "sha256:{}"
[mcps.servers.fixture]
kind = "sandbox"
executable = "/bin/fixture"
args = ["{argument}"]
workdir = "/workspace"
[mcps.permissions.fixture]
enabled = {enabled}
"#,
        "a".repeat(64)
    ))
    .unwrap();
    let corporate = Default::default();
    let inputs = EnvironmentPolicyInputs::new(&boot, &boot, &corporate);
    toml::to_string(&ActivePolicyFile::from_environment(&inputs, None).unwrap())
        .unwrap()
        .into_bytes()
}

#[tokio::test]
async fn sandbox_owner_relays_exact_stdio_and_retires_when_disabled() {
    let source = crate::runtime_config::RuntimePolicySource::new("active_policy.toml");
    source.load_bytes(&policy("approved", true)).unwrap();
    assert!(source.load_bytes(&policy("replacement", true)).is_err());
    let (hub, mut control) = guest_control::channel(4);
    let jobs = Arc::new(JobStore::new());
    let owner = Owner {
        jobs: jobs.clone(),
        hub,
        policy: source.sandbox_mcps(),
        ready: Arc::new(AtomicBool::new(true)),
    };
    assert!(owner.open(1).await.is_err());
    let opener = tokio::spawn(async move { owner.open(0).await });
    let HostToGuest::Exec { id, command } = control.recv().await.unwrap() else {
        panic!("expected sandbox launcher")
    };
    assert_eq!(command, capsem_core::container::MCP_LAUNCH_COMMAND);
    assert!(!command.contains("approved"));
    let (guest_vsock, mut agent) = UnixStream::pair().unwrap();
    agent.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    jobs.sandbox_mcps.attach(
        id,
        VsockConnection::new(
            guest_vsock.as_raw_fd(),
            capsem_proto::VSOCK_PORT_EXEC,
            Box::new(guest_vsock),
        ),
    );
    let mut peer = opener.await.unwrap().unwrap();
    peer.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    let io = tokio::task::spawn_blocking(move || {
        let ExecInputFrame::Data(preface) = capsem_proto::read_exec_input(&mut agent).unwrap() else {
            panic!("missing setup")
        };
        let size = u32::from_be_bytes(preface[..4].try_into().unwrap()) as usize;
        assert_eq!(size, preface.len() - 4);
        let setup: serde_json::Value = serde_json::from_slice(&preface[4..]).unwrap();
        assert_eq!(setup["argv"], serde_json::json!(["/bin/fixture", "approved"]));
        assert_eq!(setup["workdir"], "/workspace");
        peer.write_all(b"input\n").unwrap();
        assert_eq!(
            capsem_proto::read_exec_input(&mut agent).unwrap(),
            ExecInputFrame::Data(b"input\n".to_vec())
        );
        capsem_proto::write_exec_output_data(&mut agent, ExecOutputChannel::Stderr, b"internal").unwrap();
        capsem_proto::write_exec_output_data(&mut agent, ExecOutputChannel::Stdout, b"output\n").unwrap();
        let mut reply = [0; 7];
        peer.read_exact(&mut reply).unwrap();
        assert_eq!(&reply, b"output\n");
        (peer, agent)
    });
    let (_peer, _agent) = io.await.unwrap();
    source.load_bytes(&policy("approved", false)).unwrap();
    let cancel = tokio::time::timeout(Duration::from_secs(2), control.recv())
        .await
        .unwrap()
        .unwrap();
    assert!(matches!(cancel, HostToGuest::CancelExec { id: target, .. } if target == id));
    assert!(jobs.sandbox_mcps.known(id));
    assert!(jobs.sandbox_mcps.complete(id));
    assert!(!jobs.sandbox_mcps.known(id));
}
