"""Guest launcher policy, exercised without executing an image on the host."""

import importlib.util
from pathlib import Path

import pytest

SOURCE = Path(__file__).resolve().parents[3] / "guest/artifacts/container/launch.py"


@pytest.fixture
def launcher():
    spec = importlib.util.spec_from_file_location("container_launch", SOURCE)
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def image():
    return {
        "config": {
            "Entrypoint": ["docker-entrypoint.sh"],
            "Cmd": ["redis-server"],
            "Volumes": {"/data": {}},
        }
    }


def unpacked():
    return {
        "process": {
            "args": ["docker-entrypoint.sh", "redis-server"],
            "env": ["PATH=/usr/local/bin:/usr/bin:/bin", "A=old"],
            "cwd": "/data",
            "user": {"uid": 0, "gid": 0},
        },
        "hooks": {"prestart": [{"path": "/evil"}]},
    }


def test_default_command_user_and_workdir_survive_hardening(launcher):
    config = launcher.configure(unpacked(), image(), {"args": [], "env": {}})
    process = config["process"]
    assert process["args"] == ["docker-entrypoint.sh", "redis-server"]
    assert process["user"] == {"uid": 0, "gid": 0}
    assert process["cwd"] == "/data"
    assert process["noNewPrivileges"] is True
    assert config["root"] == {"path": "rootfs", "readonly": True}
    hooks = config["hooks"]["prestart"]
    assert len(hooks) == 1 and hooks[0]["path"] == "/usr/bin/python3"
    assert hooks[0]["args"] == ["/usr/bin/python3", str(SOURCE), "--network-ready"]
    assert hooks[0]["timeout"] == 5
    assert {ns["type"] for ns in config["linux"]["namespaces"]} == {
        "pid",
        "mount",
        "ipc",
        "uts",
        "network",
    }
    assert "CAP_SYS_ADMIN" not in process["capabilities"]["bounding"]
    assert "CAP_NET_RAW" not in process["capabilities"]["bounding"]
    assert config["linux"]["resources"]["memory"]["limit"] == 256 * 1024**2
    assert config["linux"]["resources"]["pids"]["limit"] == 256
    assert all(mount["type"] in {"proc", "tmpfs", "bind"} for mount in config["mounts"])
    assert "/data" in {mount["destination"] for mount in config["mounts"]}


def test_container_trusts_capsem_ca_and_resolves_through_the_gateway(launcher):
    options = {"args": [], "env": {"NODE_EXTRA_CA_CERTS": "/mine"}}
    config = launcher.configure(unpacked(), image(), options)
    binds = {
        mount["destination"]: mount
        for mount in config["mounts"]
        if mount["type"] == "bind"
    }
    # Only these two host files ever enter a container, and only read-only.
    assert set(binds) == {"/etc/resolv.conf", launcher.CA_BUNDLE}
    assert binds[launcher.CA_BUNDLE]["source"] == launcher.CA_BUNDLE
    assert binds["/etc/resolv.conf"]["source"] == str(launcher.RUNTIME / "resolv.conf")
    for mount in binds.values():
        assert {"bind", "ro", "nosuid", "nodev", "noexec"} <= set(mount["options"])
    env = dict(entry.split("=", 1) for entry in config["process"]["env"])
    assert env["SSL_CERT_FILE"] == launcher.CA_BUNDLE
    assert env["REQUESTS_CA_BUNDLE"] == launcher.CA_BUNDLE
    assert env["CURL_CA_BUNDLE"] == launcher.CA_BUNDLE
    assert env["NODE_EXTRA_CA_CERTS"] == "/mine", "explicit user environment wins"
    assert launcher.resolv_conf().startswith(f"nameserver {launcher.GATEWAY}\n")


VM_OUTPUT_RULES = """\
-P OUTPUT ACCEPT
-A OUTPUT -p udp -m udp --dport 53 -j REDIRECT --to-ports 1053
-A OUTPUT -p tcp -m tcp --dport 53 -j REDIRECT --to-ports 1053
-A OUTPUT -d 10.128.0.0/9 -j RETURN
-A OUTPUT -p tcp -m tcp --dport 443 -j REDIRECT --to-ports 10443
-A OUTPUT -p tcp -m tcp --dport 80 -j REDIRECT --to-ports 10080
-A OUTPUT -p tcp -m tcp --dport 8080 -j REDIRECT --to-ports 10080
"""


def test_gateway_redirects_mirror_the_vm_interception_rules(launcher):
    assert launcher.derive_redirects(VM_OUTPUT_RULES) == [
        ("udp", 53, 1053, None),
        ("tcp", 53, 1053, None),
        ("tcp", 443, 10443, None),
        ("tcp", 80, 10080, None),
        ("tcp", 8080, 10080, None),
    ]
    assert launcher.derive_redirects("-P OUTPUT ACCEPT\n") == []
    assert launcher.derive_returns(VM_OUTPUT_RULES) == ["10.128.0.0/9"]
    assert launcher.derive_returns("-P OUTPUT ACCEPT\n") == []


class FakeRun:
    """Records every command the hook issues and answers the three it reads."""

    def __init__(self, rules, jump_exists=False):
        self.calls = []
        self.rules = rules
        self.jump_exists = jump_exists

    def __call__(self, *argv, check=True, **kwargs):
        self.calls.append(list(argv))
        stdout = ""
        returncode = 0
        if "-S" in argv:
            stdout = self.rules
        if "-C" in argv:
            assert not check, "chain existence must be probed without raising"
            returncode = 0 if self.jump_exists else 1
        return type("Done", (), {"returncode": returncode, "stdout": stdout})()


def hook_environment(tmp_path):
    (tmp_path / "net/ipv4/conf/capsem0").mkdir(parents=True)
    return tmp_path


def test_network_ready_hook_pins_the_container_to_the_vm_proxies(launcher, tmp_path):
    run = FakeRun(VM_OUTPUT_RULES)
    launcher.network_ready(4242, run=run, sysctl_root=hook_environment(tmp_path))
    calls = run.calls
    ns = ["nsenter", "-t", "4242", "-n"]
    iptables = launcher.IPTABLES
    # The container gets its own end of a veth pair, one address and one route.
    assert [
        "ip",
        "link",
        "add",
        "capsem0",
        "type",
        "veth",
        "peer",
        "name",
        "capsem1",
    ] in calls
    assert ["ip", "link", "set", "capsem1", "netns", "4242"] in calls
    assert [
        *ns,
        "ip",
        "addr",
        "add",
        f"{launcher.CONTAINER_ADDRESS}/30",
        "dev",
        "eth0",
    ] in calls
    assert [*ns, "ip", "route", "add", "default", "via", launcher.GATEWAY] in calls
    assert (tmp_path / "net/ipv4/conf/capsem0/route_localnet").read_text() == "1\n"
    # Every VM redirect is mirrored as a DNAT to the loopback proxy ...
    for proto, dport, target, destination in launcher.derive_redirects(VM_OUTPUT_RULES):
        dnat = [
            iptables,
            "-t",
            "nat",
            "-A",
            launcher.NAT_CHAIN,
            "-i",
            "capsem0",
            "-p",
            proto,
        ]
        if destination:
            dnat += ["-d", destination]
        if dport is not None:
            dnat += ["--dport", str(dport)]
        dnat += ["-j", "DNAT", "--to-destination", f"127.0.0.1:{target}"]
        assert dnat in calls
        accept = [
            iptables,
            "-A",
            launcher.INPUT_CHAIN,
            "-i",
            "capsem0",
            "-d",
            "127.0.0.1",
            "-p",
            proto,
        ]
        accept += ["--dport", str(target), "-j", "ACCEPT"]
        assert accept in calls
    # ... and nothing else in the VM is reachable from the container.
    reject = [iptables, "-A", launcher.INPUT_CHAIN, "-i", "capsem0", "-j", "DROP"]
    assert reject in calls
    accepts = [
        i
        for i, call in enumerate(calls)
        if call[:3] == [iptables, "-A", launcher.INPUT_CHAIN] and call[-1] == "ACCEPT"
    ]
    assert accepts and max(accepts) < calls.index(reject), (
        "accepts must precede the drop"
    )
    assert [iptables, "-I", "FORWARD", "-i", "capsem0", "-j", "DROP"] in calls
    assert [
        iptables,
        "-t",
        "nat",
        "-I",
        "PREROUTING",
        "-j",
        launcher.NAT_CHAIN,
    ] in calls
    assert [iptables, "-I", "INPUT", "-j", launcher.INPUT_CHAIN] in calls


def test_network_ready_hook_does_not_duplicate_chain_jumps(launcher, tmp_path):
    run = FakeRun(VM_OUTPUT_RULES, jump_exists=True)
    launcher.network_ready(7, run=run, sysctl_root=hook_environment(tmp_path))
    iptables = launcher.IPTABLES
    assert [
        iptables,
        "-t",
        "nat",
        "-I",
        "PREROUTING",
        "-j",
        launcher.NAT_CHAIN,
    ] not in run.calls
    assert [iptables, "-I", "INPUT", "-j", launcher.INPUT_CHAIN] not in run.calls
    assert [iptables, "-I", "FORWARD", "-i", "capsem0", "-j", "DROP"] not in run.calls
    # The chains themselves are flushed so a second workload starts clean.
    assert [iptables, "-t", "nat", "-F", launcher.NAT_CHAIN] in run.calls
    assert [iptables, "-F", launcher.INPUT_CHAIN] in run.calls


def test_network_ready_hook_refuses_a_vm_without_interception(launcher, tmp_path):
    run = FakeRun("-P OUTPUT ACCEPT\n")
    with pytest.raises(ValueError, match="interception"):
        launcher.network_ready(7, run=run, sysctl_root=hook_environment(tmp_path))
    assert not any("DNAT" in call for call in run.calls)


def test_command_override_replaces_cmd_but_preserves_entrypoint(launcher):
    options = {"args": ["redis-server", "--save", ""], "env": {"A": "new", "B": "a=b"}}
    config = launcher.configure(unpacked(), image(), options)
    assert config["process"]["args"] == [
        "docker-entrypoint.sh",
        "redis-server",
        "--save",
        "",
    ]
    assert config["process"]["env"] == [
        "PATH=/usr/local/bin:/usr/bin:/bin",
        "A=new",
        f"SSL_CERT_FILE={launcher.CA_BUNDLE}",
        f"REQUESTS_CA_BUNDLE={launcher.CA_BUNDLE}",
        f"CURL_CA_BUNDLE={launcher.CA_BUNDLE}",
        f"NODE_EXTRA_CA_CERTS={launcher.CA_BUNDLE}",
        "B=a=b",
    ]


@pytest.mark.parametrize(
    "volume", ["/", "../escape", "/data/../etc", "/proc", "/dev/x", "/sys", "/usr/bin"]
)
def test_image_cannot_replace_security_mounts_or_root(launcher, volume):
    source = image()
    source["config"]["Volumes"] = {volume: {}}
    with pytest.raises(ValueError):
        launcher.configure(unpacked(), source, {"args": [], "env": {}})


def test_missing_command_fails_before_runtime_launch(launcher):
    config = unpacked()
    config["process"]["args"] = []
    with pytest.raises(ValueError, match="command"):
        launcher.configure(config, {"config": {}}, {"args": [], "env": {}})


def test_uploaded_image_remains_available_for_restart_and_fork(launcher, tmp_path):
    import hashlib
    import json

    stage = tmp_path / "stage"
    stage.mkdir()
    content = b"verified OCI bytes"
    (stage / "0-0").write_bytes(content)
    (stage / "transfer.json").write_text(
        json.dumps(
            [
                {
                    "path": "blob",
                    "key": 0,
                    "parts": 1,
                    "sha256": hashlib.sha256(content).hexdigest(),
                }
            ]
        )
    )
    for name in ("first-boot", "restart"):
        layout = tmp_path / name
        layout.mkdir()
        launcher.assemble(stage, layout)
        assert (layout / "blob").read_bytes() == content
    (stage / "0-0").write_bytes(b"tampered")
    layout = tmp_path / "tampered"
    layout.mkdir()
    with pytest.raises(ValueError, match="digest"):
        launcher.assemble(stage, layout)


def test_network_ready_hook_routes_the_container_through_every_cable(launcher, tmp_path):
    """Whatever cables the VM has, now or plugged later, carry the container
    too: every protocol to a member leaves through the cable that routes it,
    as that cable's own address, and what arrives on a cable lands in the
    container. Member traffic returns before any proxy DNAT, and everything
    else from the container stays dropped."""
    run = FakeRun(VM_OUTPUT_RULES)
    root = hook_environment(tmp_path)
    (root / "net/ipv4").mkdir(parents=True, exist_ok=True)
    launcher.network_ready(4242, run=run, sysctl_root=root)
    calls = run.calls
    iptables = launcher.IPTABLES
    assert (root / "net/ipv4/ip_forward").read_text() == "1\n"
    masquerade = [
        iptables, "-t", "nat", "-A", "POSTROUTING",
        "-o", "cable+", "-s", launcher.CONTAINER_ADDRESS, "-j", "MASQUERADE",
    ]
    assert masquerade in calls
    # Only what is addressed to the VM itself: a packet routed through this
    # VM toward another network is not the container's, and is dropped.
    inbound = [
        iptables, "-t", "nat", "-A", launcher.NAT_CHAIN,
        "-i", "cable+", "-m", "addrtype", "--dst-type", "LOCAL",
        "-j", "DNAT", "--to-destination", launcher.CONTAINER_ADDRESS,
    ]
    assert inbound in calls
    out = [iptables, "-I", "FORWARD", "-i", "capsem0", "-o", "cable+", "-j", "ACCEPT"]
    back = [iptables, "-I", "FORWARD", "-i", "cable+", "-o", "capsem0", "-j", "ACCEPT"]
    assert out in calls and back in calls
    drop = [iptables, "-I", "FORWARD", "-i", "capsem0", "-j", "DROP"]
    # Inserted after the drop, so they sit above it in the chain.
    assert calls.index(out) > calls.index(drop)
    assert calls.index(back) > calls.index(drop)
    pool_return = [
        iptables, "-t", "nat", "-A", launcher.NAT_CHAIN,
        "-i", "capsem0", "-d", "10.128.0.0/9", "-j", "RETURN",
    ]
    assert pool_return in calls
    proxy_dnats = [
        index for index, call in enumerate(calls)
        if launcher.NAT_CHAIN in call and any(arg.startswith("127.0.0.1:") for arg in call)
    ]
    assert proxy_dnats and all(calls.index(pool_return) < index for index in proxy_dnats)
    assert not any(call[:3] == ["ip", "-o", "addr"] for call in calls), "no cable is probed"
    assert not any("tap0" in call for call in calls)


def test_network_ready_hook_never_makes_the_vm_a_router_between_its_networks(
    launcher, tmp_path
):
    """The container needs forwarding on, which would also let a VM plugged
    into two networks carry one member's packets to the other. Being on two
    networks never joins them: cable to cable is dropped outright."""
    run = FakeRun(VM_OUTPUT_RULES)
    root = hook_environment(tmp_path)
    (root / "net/ipv4").mkdir(parents=True, exist_ok=True)
    launcher.network_ready(4242, run=run, sysctl_root=root)
    iptables = launcher.IPTABLES
    no_transit = [iptables, "-I", "FORWARD", "-i", "cable+", "-o", "cable+", "-j", "DROP"]
    assert no_transit in run.calls
    accepts = [
        index for index, call in enumerate(run.calls)
        if call[:3] == [iptables, "-I", "FORWARD"] and call[-1] == "ACCEPT"
    ]
    # Inserted last, so it sits first in the chain.
    assert accepts and all(index < run.calls.index(no_transit) for index in accepts)


def _mounts_at(config, destination):
    return [mount for mount in config["mounts"] if mount["destination"] == destination]


def test_the_workspace_is_mounted_where_the_service_says(launcher):
    """The container sees the VM workspace, so files placed through the API reach it.

    The mount point comes from options.json, which Capsem writes: the service
    and the launcher cannot disagree about where the workspace is.
    """
    options = {"args": [], "env": {}, "workspace": "/workspace"}
    config = launcher.configure(unpacked(), image(), options)
    (mount,) = _mounts_at(config, "/workspace")
    assert mount["type"] == "bind"
    assert mount["source"] == "/root", "the VM's /root is the workspace share"
    assert {"bind", "nosuid", "nodev"} <= set(mount["options"])
    assert "ro" not in mount["options"], "a workload writes its outputs back to the workspace"


def test_the_stage_stays_hidden_from_the_container(launcher):
    """The launcher and its inputs live in the workspace, under .capsem-image.

    The launcher runs as root in the VM on every relaunch, and options.json
    carries the workload's environment. A container that could read or replace
    that directory would read those secrets or escape into the VM, so an empty
    read-only mount covers it, after the workspace mount that would expose it.
    """
    options = {"args": [], "env": {}, "workspace": "/workspace"}
    config = launcher.configure(unpacked(), image(), options)
    destinations = [mount["destination"] for mount in config["mounts"]]
    (mask,) = _mounts_at(config, "/workspace/.capsem-image")
    assert mask["type"] == "tmpfs"
    assert {"ro", "nosuid", "nodev", "noexec"} <= set(mask["options"])
    assert destinations.index("/workspace/.capsem-image") > destinations.index("/workspace")


def test_an_image_volume_cannot_shadow_the_workspace(launcher):
    """An image declaring its own /workspace volume must not hide the real one."""
    declared = image()
    declared["config"]["Volumes"] = {"/workspace": {}}
    config = launcher.configure(unpacked(), declared, {"args": [], "env": {}, "workspace": "/workspace"})
    assert _mounts_at(config, "/workspace")[-1]["type"] == "bind", "the last mount at a path is the one seen"


def test_a_stage_written_before_the_workspace_mount_existed_starts_without_it(launcher):
    """A persistent VM staged by an older Capsem restarts exactly as it did."""
    config = launcher.configure(unpacked(), image(), {"args": [], "env": {}})
    assert not _mounts_at(config, "/workspace")


@pytest.mark.parametrize(
    "workspace",
    ["workspace", "/proc", "/etc/ws", "/a/../b", "/", "/work//space", "/usr/lib/ws"],
)
def test_an_unsafe_workspace_mount_point_is_refused(launcher, workspace):
    """The mount point is launcher input: it must never land on a system path."""
    with pytest.raises(ValueError, match="unsafe workspace mount point"):
        launcher.configure(unpacked(), image(), {"args": [], "env": {}, "workspace": workspace})
