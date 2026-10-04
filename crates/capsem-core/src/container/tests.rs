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

#[test]
fn the_workload_gets_the_vm_minus_the_runtime_reserve() {
    let resources = workload_resources(2048, 2).unwrap();
    assert_eq!(resources.memory_bytes, (2048 - RUNTIME_RESERVE_MB) * 1024 * 1024);
    assert_eq!(resources.cpu_millis, 2000 - RUNTIME_RESERVE_CPU_MILLIS);
    assert_eq!(resources.pids, WORKLOAD_PIDS);
    // One CPU still leaves the workload most of it.
    assert_eq!(workload_resources(1024, 1).unwrap().cpu_millis, 750);
}

#[test]
fn a_vm_too_small_for_a_workload_is_refused() {
    let smallest = RUNTIME_RESERVE_MB + WORKLOAD_MIN_MB;
    assert!(workload_resources(smallest, 1).is_ok());
    for ram_mb in [0, RUNTIME_RESERVE_MB, smallest - 1] {
        assert!(workload_resources(ram_mb, 1).is_err(), "{ram_mb} MiB accepted");
    }
    assert!(workload_resources(4096, 0).is_err());
}

#[test]
fn the_launch_command_is_unchanged_by_sharing_the_moved_root_prefix() {
    assert_eq!(
        LAUNCH_COMMAND,
        "chmod 555 /root/.capsem-image/launch.py && chroot /proc/1/root /bin/busybox unshare -m /bin/sh -ec \
         'mount --make-rprivate /; cd /newroot; mount --move . /; exec chroot . /usr/bin/python3 \
         /root/.capsem-image/launch.py /root/.capsem-image'"
    );
}

/// The JSON request a `workload_exec_command` carries, hex-decoded.
fn decode_workload_exec(payload: &str) -> serde_json::Value {
    let bytes: Vec<u8> = (0..payload.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(&payload[at..at + 2], 16).unwrap())
        .collect();
    serde_json::from_slice(&bytes).unwrap()
}

/// The workload exec runs the launcher in the same moved-root context as the
/// launch, and a hostile command reaches it byte for byte: quotes, newlines
/// and substitutions are payload, never shell syntax at either level.
#[test]
fn a_workload_exec_carries_any_command_exactly_to_the_launcher() {
    let hostile = "printf '%s' \"$(id -u)\"; echo 'a'\\''b' `uname`\n; exit 7";
    let command = workload_exec_command(hostile);
    let (context, _) = LAUNCH_COMMAND
        .strip_prefix("chmod 555 /root/.capsem-image/launch.py && ")
        .unwrap()
        .split_once("launch.py ")
        .unwrap();
    assert!(command.starts_with(context), "{command}");
    // What the outer shell hands `sh -ec`: run it through a real shell with
    // the chroot replaced by printf and read the inner script back.
    let inner = std::process::Command::new("/bin/sh")
        .arg("-c")
        .arg(command.replacen(
            "chroot /proc/1/root /bin/busybox unshare -m /bin/sh -ec",
            "printf %s",
            1,
        ))
        .output()
        .unwrap();
    let inner = String::from_utf8(inner.stdout).unwrap();
    let payload = inner
        .strip_prefix(
            "mount --make-rprivate /; cd /newroot; mount --move . /; exec chroot . /usr/bin/python3 \
             /root/.capsem-image/launch.py --exec ",
        )
        .unwrap_or_else(|| panic!("unexpected inner script: {inner}"));
    assert!(payload.bytes().all(|byte| byte.is_ascii_hexdigit()), "{payload}");
    assert_eq!(
        decode_workload_exec(payload),
        serde_json::json!({ "command": hostile, "tty": false })
    );
}
