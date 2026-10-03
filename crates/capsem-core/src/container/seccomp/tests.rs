use super::*;
use std::collections::BTreeSet;

fn filter() -> Value {
    workload_seccomp(super::super::stage::oci_architecture().unwrap()).unwrap()
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
        let filter = workload_seccomp(arch).unwrap();
        assert!(!filter["architectures"].as_array().unwrap().is_empty(), "{arch}");
    }
    assert!(workload_seccomp("mips64").is_err());
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
