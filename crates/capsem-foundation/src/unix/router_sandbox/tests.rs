use super::*;
use std::io::{Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::os::fd::AsFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::process::{Command, Stdio};
use std::time::Duration;

#[test]
fn confinement_denies_ambient_authority_but_preserves_inherited_tcp() {
    let directory = tempfile::tempdir().unwrap();
    let control = directory.path().join("control.sock");
    let _control = UnixListener::bind(&control).unwrap();
    let secret = directory.path().join("secret");
    std::fs::write(&secret, b"hypervisor state").unwrap();
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let socket = super::super::fd::duplicate(listener.as_fd()).unwrap();
    let executable = std::env::current_exe().unwrap();
    #[cfg(target_os = "macos")]
    let executable = {
        // Seatbelt cannot stack. The gate hands off only this named child;
        // the parent test stays inside the gate's network sandbox.
        let child = directory.path().join("capsem-router-confinement-test");
        std::fs::copy(executable, &child).unwrap();
        child
    };
    let mut child = Command::new(executable)
        .args(["--exact", "unix::router_sandbox::tests::sandbox_child", "--nocapture"])
        .env_clear()
        .env("ROUTER_SANDBOX_TEST", &secret)
        .env("ROUTER_CONTROL_TEST", &control)
        .env("ROUTER_PORT_TEST", port.to_string())
        .stdin(Stdio::from(socket))
        .spawn()
        .unwrap();
    drop(listener);
    let mut client = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
    client.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    client.write_all(b"ping").unwrap();
    let mut reply = [0; 4];
    let read = client.read_exact(&mut reply);
    let status = child.wait().unwrap();
    assert!(status.success(), "sandbox child failed: {status}");
    read.unwrap();
    assert_eq!(&reply, b"pong");
}

#[test]
fn sandbox_child() {
    let Some(secret) = std::env::var_os("ROUTER_SANDBOX_TEST") else {
        return;
    };
    let control = std::env::var_os("ROUTER_CONTROL_TEST").unwrap();
    let port = std::env::var("ROUTER_PORT_TEST").unwrap().parse().unwrap();
    let stdin = std::io::stdin();
    let listener = TcpListener::from(super::super::fd::duplicate(stdin.as_fd()).unwrap());
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    confine(port).unwrap();
    runtime.block_on(async {
        tokio::task::spawn(async move {
            assert!(std::fs::read(secret).is_err(), "router read hypervisor state");
            assert!(
                UnixStream::connect(control).is_err(),
                "router reached VM control socket"
            );
            assert!(
                TcpStream::connect((Ipv4Addr::LOCALHOST, port)).is_err(),
                "router opened an outbound connection"
            );
            assert!(
                Command::new("/usr/bin/true").status().is_err(),
                "router executed a process"
            );
            let parent = unsafe { libc::getppid() };
            assert_eq!(unsafe { libc::kill(parent, 0) }, -1, "router could signal its parent");
        })
        .await
        .unwrap();
    });
    let (mut stream, _) = listener.accept().unwrap();
    let mut request = [0; 4];
    stream.read_exact(&mut request).unwrap();
    assert_eq!(&request, b"ping");
    stream.write_all(b"pong").unwrap();
    std::process::exit(0);
}
