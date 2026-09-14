use super::*;

#[test]
fn publications_are_explicit_loopback_port_pairs() {
    assert_eq!(
        "16379:6379".parse::<PortMapping>().unwrap(),
        PortMapping {
            host: 16379,
            guest: 6379
        }
    );
    assert_eq!("0:6379".parse::<PortMapping>().unwrap().host, 0);
    for invalid in ["6379", "0.0.0.0:6379:6379", "1:0", "65536:6379", "1:2/udp"] {
        assert!(invalid.parse::<PortMapping>().is_err(), "accepted {invalid}");
    }
}

#[test]
fn the_detached_launch_is_the_launch_command_backgrounded_to_the_console() {
    let command = detached_launch_command();
    assert!(command.starts_with("setsid /bin/sh -c '"), "{command}");
    assert!(command.ends_with("' </dev/null >/dev/console 2>&1 &"), "{command}");
    // The quoting survives a real shell: the inner command comes back exact.
    let echoed = std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(
            command
                .replacen("setsid /bin/sh -c", "printf %s", 1)
                .replace(" </dev/null >/dev/console 2>&1 &", ""),
        )
        .output()
        .unwrap();
    assert_eq!(String::from_utf8(echoed.stdout).unwrap(), LAUNCH_COMMAND);
}
