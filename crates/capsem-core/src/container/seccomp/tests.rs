use super::*;
use std::collections::BTreeSet;

fn filter() -> Value {
    workload_seccomp(super::super::stage::oci_architecture().unwrap(), Surface::Terminal).unwrap()
}

/// Syscalls some rule allows with no argument condition.
fn unconditionally_allowed(filter: &Value) -> BTreeSet<String> {
    filter["syscalls"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|rule| rule["action"] == "SCMP_ACT_ALLOW" && rule.get("args").is_none())
        .flat_map(|rule| {
            rule["names"]
                .as_array()
                .unwrap()
                .iter()
                .map(|name| name.as_str().unwrap().to_string())
        })
        .collect()
}

fn rules_for<'a>(filter: &'a Value, name: &str) -> Vec<&'a Value> {
    filter["syscalls"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|rule| rule["names"].as_array().unwrap().iter().any(|value| value == name))
        .collect()
}

#[test]
fn the_vendored_allowlist_is_the_recorded_file() {
    assert_eq!(
        blake3::hash(MOBY_DEFAULT.as_bytes()).to_hex().as_str(),
        MOBY_DEFAULT_BLAKE3,
        "an edit to the vendored allowlist must update its recorded hash"
    );
}

#[test]
fn the_filter_denies_by_default_and_carries_no_docker_only_keys() {
    let filter = filter();
    assert_eq!(filter["defaultAction"], "SCMP_ACT_ERRNO");
    for rule in filter["syscalls"].as_array().unwrap() {
        for key in ["includes", "excludes", "comment"] {
            assert!(rule.get(key).is_none(), "runc would ignore {key}: {rule}");
        }
    }
}

#[test]
fn escape_and_kernel_surface_syscalls_are_denied() {
    let allowed = unconditionally_allowed(&filter());
    for name in [
        "mount",
        "umount2",
        "pivot_root",
        "setns",
        "unshare",
        "bpf",
        "keyctl",
        "add_key",
        "request_key",
        "kexec_load",
        "init_module",
        "finit_module",
        "open_by_handle_at",
        "io_uring_setup",
        "userfaultfd",
        "perf_event_open",
        "reboot",
        "swapon",
    ] {
        assert!(!allowed.contains(name), "{name} must not be allowed to the workload");
    }
    for name in [
        "read",
        "openat",
        "clone3",
        "execve",
        "futex",
        "epoll_pwait",
        "close_range",
        "statx",
    ] {
        assert!(
            allowed.contains(name) || !rules_for(&filter(), name).is_empty(),
            "{name} must reach a rule"
        );
    }
}

#[test]
fn clone_never_creates_namespaces_and_clone3_falls_back() {
    let filter = filter();
    for rule in rules_for(&filter, "clone") {
        assert!(
            rule.get("args").is_some(),
            "clone is only allowed with its namespace flags masked: {rule}"
        );
    }
    let clone3 = rules_for(&filter, "clone3");
    assert!(
        clone3
            .iter()
            .any(|rule| rule["action"] == "SCMP_ACT_ERRNO" && rule["errnoRet"] == 38),
        "clone3 returns ENOSYS so libc falls back to the filtered clone"
    );
}

#[test]
fn sockets_open_only_the_allowed_families() {
    let filter = filter();
    assert!(!unconditionally_allowed(&filter).contains("socket"));
    let rules = rules_for(&filter, "socket");
    let families: BTreeSet<u64> = rules
        .iter()
        .map(|rule| rule["args"][0]["value"].as_u64().unwrap())
        .collect();
    assert_eq!(families, BTreeSet::from([AF_UNIX, AF_INET, AF_INET6, AF_NETLINK]));
    assert!(!families.contains(&40), "vsock reaches every host service port");
    for rule in &rules {
        let args = rule["args"].as_array().unwrap();
        let indices: BTreeSet<u64> = args.iter().map(|arg| arg["index"].as_u64().unwrap()).collect();
        assert_eq!(
            indices.len(),
            args.len(),
            "two conditions on one argument become an OR in runc: {rule}"
        );
    }
    let netlink = rules
        .iter()
        .find(|rule| rule["args"][0]["value"] == AF_NETLINK)
        .unwrap();
    assert_eq!(
        netlink["args"][1],
        json!({"index": 2, "value": NETLINK_NETFILTER, "op": "SCMP_CMP_NE"})
    );
}

#[test]
fn both_guest_architectures_resolve() {
    for arch in ["arm64", "amd64"] {
        let filter = workload_seccomp(arch, Surface::Terminal).unwrap();
        assert!(!filter["architectures"].as_array().unwrap().is_empty(), "{arch}");
    }
    assert!(workload_seccomp("mips64", Surface::Terminal).is_err());
}

#[test]
fn only_the_native_abi_is_admitted() {
    let filter = filter();
    assert_eq!(
        filter["architectures"].as_array().unwrap().len(),
        1,
        "{}",
        filter["architectures"]
    );
    assert!(
        rules_for(&filter, "socketcall").is_empty(),
        "socketcall opens any family, around the socket rules"
    );
}

fn allowing(filter: &Value, name: &str) -> Vec<Value> {
    filter["syscalls"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|rule| {
            rule["action"] == "SCMP_ACT_ALLOW" && rule["names"].as_array().unwrap().iter().any(|n| n == name)
        })
        .cloned()
        .collect()
}

#[test]
fn a_terminal_workload_creates_no_namespace_and_never_chroots() {
    let filter = workload_seccomp("arm64", Surface::Terminal).unwrap();
    assert!(allowing(&filter, "unshare").is_empty());
    assert!(allowing(&filter, "chroot").is_empty());
    // moby's clone rule refuses every CLONE_NEW* bit.
    for rule in allowing(&filter, "clone") {
        assert_eq!(rule["args"][0]["value"], 0x7E02_0000u64, "{rule}");
    }
}

#[test]
fn an_xpra_workload_may_create_exactly_chromiums_sandbox_namespaces() {
    let filter = workload_seccomp("arm64", Surface::Xpra).unwrap();
    let refused = |name: &str| -> Vec<u64> {
        allowing(&filter, name)
            .iter()
            .map(|rule| rule["args"][0]["value"].as_u64().unwrap())
            .collect()
    };
    // The loosest clone and unshare rules still refuse the mount, IPC, UTS,
    // cgroup and time namespaces: only user, pid and net are added.
    let narrowest = |masks: Vec<u64>| masks.into_iter().min_by_key(|mask| mask.count_ones()).unwrap();
    for name in ["clone", "unshare"] {
        let mask = narrowest(refused(name));
        assert_eq!(mask, 0x0E02_0080, "{name}");
        // CLONE_NEWNS, NEWIPC, NEWUTS, NEWCGROUP, NEWTIME.
        for refused_bit in [0x0002_0000u64, 0x0800_0000, 0x0400_0000, 0x0200_0000, 0x80] {
            assert_ne!(mask & refused_bit, 0, "{name}: {refused_bit:#x} must stay refused");
        }
    }
    assert_eq!(allowing(&filter, "chroot").len(), 1);
    // Still no vsock, setns or keyctl.
    assert!(allowing(&filter, "setns").is_empty());
    assert!(allowing(&filter, "keyctl").is_empty());
}

#[test]
fn surfaces_come_from_the_image_label() {
    assert_eq!(Surface::from_label(None).unwrap(), Surface::Terminal);
    assert_eq!(Surface::from_label(Some("terminal")).unwrap(), Surface::Terminal);
    assert_eq!(Surface::from_label(Some("xpra")).unwrap(), Surface::Xpra);
    assert!(Surface::from_label(Some("vnc")).is_err());
}
