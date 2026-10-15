use super::*;

#[test]
fn mcp_roles_deny_ambient_seatbelt_authority() {
    for role in [Role::McpAggregator, Role::McpBuiltin, Role::McpExternal] {
        let compiled = super::super::seatbelt::compile(&Policy::new(role)).unwrap();
        let source = compiled.source().to_str().unwrap();
        assert!(
            source.starts_with("(version 1)\n(deny default)\n"),
            "{role:?}: {source}"
        );
        for forbidden in [
            "allow network",
            "allow process-exec",
            "allow signal",
            "allow mach-lookup",
            "allow file",
        ] {
            assert!(!source.contains(forbidden), "{role:?}: {forbidden}");
        }
    }
}

#[test]
fn mcp_external_executable_inherits_its_seatbelt() {
    let compiled =
        super::super::seatbelt::compile(&Policy::new(Role::McpExternal).allow("/approved/server", Access::Executable))
            .unwrap();
    let source = compiled.source().to_str().unwrap();
    assert!(source.contains("(allow process-exec (literal (param \"PATH_0\")))"));
    assert!(
        !source.contains("no-sandbox"),
        "external tools must inherit confinement"
    );
}

#[test]
fn mcp_nonexecuting_roles_refuse_executable_grants() {
    for role in [Role::McpAggregator, Role::McpBuiltin] {
        let error = super::super::seatbelt::compile(&Policy::new(role).allow("/approved/server", Access::Executable))
            .err()
            .expect("MCP relay role accepted execution authority");
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    }
}

#[test]
fn mcp_roles_keep_granted_io_and_deny_files_sockets_execution_and_signals() {
    let directory = tempfile::tempdir().unwrap();
    let secret = directory.path().join("other-session-credential");
    std::fs::write(&secret, b"other session secret").unwrap();
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let control = directory.path().join("other-session-control.sock");
    let _control = UnixListener::bind(&control).unwrap();
    for role in ["aggregator", "builtin", "external"] {
        let (mut report, granted) = UnixStream::pair().unwrap();
        let mut child = crate::isolated_test_command(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "unix::worker_sandbox::tests::mcp::mcp_denial_child",
                "--nocapture",
            ])
            .env("MCP_SANDBOX_PROBE_ROLE", role)
            .env("MCP_SANDBOX_PROBE_SECRET", &secret)
            .env("MCP_SANDBOX_PROBE_CONTROL", &control)
            .env(
                "MCP_SANDBOX_PROBE_PORT",
                listener.local_addr().unwrap().port().to_string(),
            )
            .stdin(Stdio::from(std::os::fd::OwnedFd::from(granted)))
            .spawn()
            .unwrap();
        report.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        report.write_all(b"ping").unwrap();
        let mut reply = [0; 4];
        let result = report.read_exact(&mut reply);
        let status = child.wait().unwrap();
        assert!(status.success(), "{role} sandbox probe: {status}");
        result.unwrap();
        assert_eq!(&reply, b"pong", "{role} did not prove its denials");
    }
}

#[test]
fn mcp_denial_child() {
    let Ok(role) = std::env::var("MCP_SANDBOX_PROBE_ROLE") else {
        return;
    };
    let role = match role.as_str() {
        "aggregator" => Role::McpAggregator,
        "builtin" => Role::McpBuiltin,
        "external" => Role::McpExternal,
        _ => panic!("unknown MCP role"),
    };
    let secret = std::env::var_os("MCP_SANDBOX_PROBE_SECRET").unwrap();
    let control = std::env::var_os("MCP_SANDBOX_PROBE_CONTROL").unwrap();
    let port: u16 = std::env::var("MCP_SANDBOX_PROBE_PORT").unwrap().parse().unwrap();
    let mut granted = UnixStream::from(super::super::super::fd::duplicate(std::io::stdin().as_fd()).unwrap());
    let parent = super::super::super::process::parent_process_id().unwrap();
    confine(&Policy::new(role)).unwrap();
    attest_descriptor_only().unwrap();
    assert!(std::fs::read(&secret).is_err(), "{role:?} read another session");
    assert!(std::fs::write(&secret, b"overwrite").is_err());
    assert!(
        UnixStream::connect(control).is_err(),
        "{role:?} reached service control"
    );
    assert!(TcpStream::connect((Ipv4Addr::LOCALHOST, port)).is_err());
    assert!(Command::new("/usr/bin/true").status().is_err());
    assert_eq!(
        super::super::super::process::probe_signal_authority(parent).unwrap(),
        super::super::super::process::SignalProbe::Denied,
    );
    let mut input = [0; 4];
    granted.read_exact(&mut input).unwrap();
    assert_eq!(&input, b"ping");
    granted.write_all(b"pong").unwrap();
    std::process::exit(0);
}

#[cfg(target_os = "linux")]
#[test]
fn native_mcp_profiles_allow_threads_but_refuse_forked_processes() {
    for role in ["aggregator", "builtin"] {
        let status = crate::isolated_test_command(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "unix::worker_sandbox::tests::mcp::mcp_fork_denial_child",
                "--nocapture",
            ])
            .env("MCP_FORK_PROBE_ROLE", role)
            .status()
            .unwrap();
        assert!(status.success(), "{role} retained fork authority");
    }
}

#[cfg(target_os = "linux")]
#[test]
fn mcp_fork_denial_child() {
    let Ok(role) = std::env::var("MCP_FORK_PROBE_ROLE") else {
        return;
    };
    let role = match role.as_str() {
        "aggregator" => Role::McpAggregator,
        "builtin" => Role::McpBuiltin,
        _ => panic!("unknown fork probe role"),
    };
    confine(&Policy::new(role)).unwrap();
    assert_eq!(
        std::thread::spawn(|| 37).join().unwrap(),
        37,
        "runtime threads must remain usable"
    );
    // SAFETY: isolated test subprocess, after the joined thread has exited;
    // a mistakenly admitted child exits without touching the test runtime.
    match unsafe { nix::unistd::fork() } {
        Err(nix::errno::Errno::EPERM) => {}
        Err(error) => panic!("fork denial returned the wrong errno: {error}"),
        Ok(nix::unistd::ForkResult::Child) => std::process::exit(0),
        Ok(nix::unistd::ForkResult::Parent { child }) => {
            nix::sys::wait::waitpid(child, None).unwrap();
            panic!("native MCP role forked a process");
        }
    }
}
