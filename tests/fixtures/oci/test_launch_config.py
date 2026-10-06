"""Guest launcher policy, exercised without executing an image on the host."""

import contextlib
import hashlib
import importlib.util
import json
import shlex
import subprocess
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


# What the host stages beside every workload (capsem-core container::seccomp).
RESOURCES = {"memory_bytes": 1024 * 1024**2, "cpu_millis": 1750, "pids": 4096}
SECURITY = {
    "capabilities": ["CAP_CHOWN", "CAP_SETUID"],
    "seccomp": {"defaultAction": "SCMP_ACT_ERRNO", "defaultErrnoRet": 1, "syscalls": []},
    "id_map": {"containerID": 0, "hostID": 100000, "size": 65536},
    "resources": RESOURCES,
}


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
    config = launcher.configure(unpacked(), image(), {**SECURITY, "args": [], "env": {}})
    process = config["process"]
    assert process["args"] == ["docker-entrypoint.sh", "redis-server"]
    assert process["user"] == {"uid": 0, "gid": 0}
    assert process["cwd"] == "/data"
    assert process["noNewPrivileges"] is True
    # The image under the session's layer: writable, the image never written.
    assert config["root"] == {"path": str(launcher.MERGED), "readonly": False}
    assert set(config["hooks"]) == {"prestart", "poststart"}
    hooks = config["hooks"]["prestart"]
    assert len(hooks) == 1 and hooks[0]["path"] == "/usr/bin/python3"
    assert hooks[0]["args"] == ["/usr/bin/python3", str(SOURCE), "--network-ready"]
    assert hooks[0]["timeout"] == 5
    assert {ns["type"] for ns in config["linux"]["namespaces"]} == {
        "user",
        "pid",
        "mount",
        "ipc",
        "uts",
        "network",
    }
    assert "CAP_SYS_ADMIN" not in process["capabilities"]["bounding"]
    assert "CAP_NET_RAW" not in process["capabilities"]["bounding"]
    assert config["linux"]["resources"]["memory"]["limit"] == 1024 * 1024**2
    assert config["linux"]["resources"]["pids"]["limit"] == 4096
    assert all(
        mount["type"] in {"proc", "tmpfs", "bind"} or (mount["type"], mount["destination"]) == ("devpts", "/dev/pts")
        for mount in config["mounts"]
    )
    assert "/data" in {mount["destination"] for mount in config["mounts"]}


def test_the_workload_is_marked_running_by_runc_once_it_started(launcher, tmp_path):
    """The service reports `running` from the stage's marker, and an exec into
    the workload needs runc to call it running. The launcher prepares the
    bundle before `runc run` creates anything, so the marker comes from runc's
    poststart hook -- after the workload's process started -- and from nothing
    the launcher writes before it."""
    config = launcher.configure(unpacked(), image(), {**SECURITY, "args": [], "env": {}})
    assert config["hooks"]["poststart"] == [
        {
            "path": "/usr/bin/python3",
            "args": ["/usr/bin/python3", str(SOURCE), "--started"],
            "env": ["PATH=/usr/sbin:/usr/bin:/sbin:/bin"],
            "timeout": 5,
        }
    ]
    stage = tmp_path / "stage"
    stage.mkdir()
    launcher.workload_started(stage)
    assert (stage / "running").read_text() == "1\n"


def test_a_launch_starts_from_no_markers_and_reports_a_workload_that_never_started(
    launcher, tmp_path
):
    stage = tmp_path / "stage"
    stage.mkdir()
    # What the last launch of a named session left behind.
    (stage / "running").write_text("1\n")
    (stage / "failed").write_text("1\n")
    launcher.clear_launch_markers(stage)
    assert not (stage / "running").exists() and not (stage / "failed").exists()

    # runc gave up before the workload ran: say so, instead of starting forever.
    launcher.launch_ended(stage, 1)
    assert (stage / "failed").read_text() == "1\n"

    # A workload that ran and then ended is not a failed start: it exited.
    launcher.clear_launch_markers(stage)
    launcher.workload_started(stage)
    launcher.launch_ended(stage, 0)
    assert not (stage / "failed").exists()
    assert (stage / "exited").read_text() == "0\n"


def test_container_trusts_capsem_ca_and_resolves_through_the_gateway(launcher):
    options = {**SECURITY, "args": [], "env": {"NODE_EXTRA_CA_CERTS": "/mine"}}
    config = launcher.configure(unpacked(), image(), options)
    binds = {
        mount["destination"]: mount
        for mount in config["mounts"]
        if mount["type"] == "bind" and launcher.VOLUMES not in Path(mount["source"]).parents
    }
    # Only these VM files ever enter a container, and only read-only; the
    # image's own volumes are its state (test_image_volumes_live_on_...).
    assert set(binds) == {"/etc/resolv.conf", "/etc/hosts", launcher.CA_BUNDLE}
    assert binds["/etc/hosts"]["source"] == str(launcher.RUNTIME / "hosts")
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


def test_localhost_names_the_containers_own_loopback(launcher):
    """An image's /etc/hosts is whatever its build left, usually empty: a
    runtime supplies it. Without `localhost` a workload that listens on or
    dials it (agy's language server does) fails to resolve its own name
    through the gateway's DNS, which never answers it."""
    hosts = launcher.hosts_file()
    entries = {
        name: address
        for line in hosts.splitlines()
        if line and not line.startswith("#")
        for address, *names in [line.split()]
        for name in names
    }
    assert entries["localhost"] == "127.0.0.1"
    config = launcher.configure(unpacked(), image(), {**SECURITY, "args": [], "env": {}})
    assert entries[config["hostname"]] == "127.0.0.1"
    # Loopback only: nothing here names the VM or any other address.
    assert set(entries.values()) == {"127.0.0.1"}


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
        self.batches = []
        self.ip_batches = []
        self.rules = rules
        self.jump_exists = jump_exists

    def __call__(self, *argv, check=True, **kwargs):
        self.calls.append(list(argv))
        if argv[-3:] == ("ip", "-batch", "-"):
            self.ip_batches.append((list(argv), kwargs))
            for line in kwargs["input"].splitlines():
                self.calls.append([*argv[:-3], "ip", *shlex.split(line)])
        if argv[0] == "iptables-nft-restore":
            self.batches.append((list(argv), kwargs))
            table = None
            for line in kwargs["input"].splitlines():
                if line.startswith("*"):
                    table = line[1:]
                elif line.startswith("-"):
                    # Decode the delivered restore grammar independently of
                    # the producer so existing policy assertions see its rules.
                    prefix = ["iptables-nft", "-t", table] if table == "nat" else ["iptables-nft"]
                    self.calls.append([*prefix, *shlex.split(line)])
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


def test_network_ready_batches_interfaces_once_per_namespace(launcher, tmp_path):
    run = FakeRun(VM_OUTPUT_RULES)
    launcher.network_ready(4242, run=run, sysctl_root=hook_environment(tmp_path))
    assert len(run.ip_batches) == 2
    host, guest = run.ip_batches
    assert host[0] == ["ip", "-batch", "-"]
    assert guest[0] == ["nsenter", "-t", "4242", "-n", "ip", "-batch", "-"]
    assert host[1] == {"text": True, "input": (
        "link add capsem0 type veth peer name capsem1\n"
        "link set capsem1 netns 4242\n"
        "addr add 10.0.1.1/30 dev capsem0\n"
        "link set capsem0 up\n"
    )}
    assert guest[1] == {"text": True, "input": (
        "link set lo up\n"
        "link set capsem1 name eth0\n"
        "addr add 10.0.1.2/30 dev eth0\n"
        "link set eth0 up\n"
        "route add default via 10.0.1.1\n"
    )}


@pytest.mark.parametrize("pid", [None, False, True, 0, -1, "4242\nlink del capsem0", 42.0])
def test_network_ready_refuses_invalid_namespace_pid_before_commands(launcher, tmp_path, pid):
    run = FakeRun(VM_OUTPUT_RULES)
    with pytest.raises(ValueError, match="pid"):
        launcher.network_ready(pid, run=run, sysctl_root=hook_environment(tmp_path))
    assert run.calls == []


@pytest.mark.parametrize("failed_batch", [0, 1])
def test_network_ready_stops_on_ip_batch_failure_before_firewall_changes(launcher, tmp_path, failed_batch):
    recorded = FakeRun(VM_OUTPUT_RULES)

    def run(*argv, **kwargs):
        if argv[-3:] == ("ip", "-batch", "-") and len(recorded.ip_batches) == failed_batch:
            assert kwargs.get("check", True) and "-force" not in argv
            raise subprocess.CalledProcessError(2, argv)
        return recorded(*argv, **kwargs)

    with pytest.raises(subprocess.CalledProcessError):
        launcher.network_ready(4242, run=run, sysctl_root=hook_environment(tmp_path))
    assert not recorded.batches
    assert not any(call[0].startswith("iptables") for call in recorded.calls)


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


def test_network_ready_batches_only_workload_owned_chains(launcher, tmp_path):
    run = FakeRun(VM_OUTPUT_RULES)
    launcher.network_ready(4242, run=run, sysctl_root=hook_environment(tmp_path))
    assert len(run.batches) == 1, "one transaction replaces repeated firewall processes"
    argv, kwargs = run.batches[0]
    assert argv == ["iptables-nft-restore", "--noflush"]
    assert kwargs["text"] is True
    lines = kwargs["input"].splitlines()
    assert [line for line in lines if line.startswith("*")] == ["*nat", "*filter"]
    assert [line.split()[0] for line in lines if line.startswith(":")] == [
        ":" + launcher.NAT_CHAIN, ":" + launcher.INPUT_CHAIN,
    ], "the VM's existing chains and policies belong to their current owner"
    assert [line for line in lines if line.startswith("-F ")] == [
        "-F " + launcher.NAT_CHAIN, "-F " + launcher.INPUT_CHAIN,
    ]
    assert lines.count("COMMIT") == 2


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
    options = {**SECURITY, "args": ["redis-server", "--save", ""], "env": {"A": "new", "B": "a=b"}}
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
        launcher.configure(unpacked(), source, {**SECURITY, "args": [], "env": {}})


def test_missing_command_fails_before_runtime_launch(launcher):
    config = unpacked()
    config["process"]["args"] = []
    with pytest.raises(ValueError, match="command"):
        launcher.configure(config, {"config": {}}, {**SECURITY, "args": [], "env": {}})


def _blob(share, data):
    """Put `data` in the share under its own SHA-256; return its digest."""
    digest = hashlib.sha256(data).hexdigest()
    (share / digest).write_bytes(data)
    return f"sha256:{digest}"


def _share(tmp_path, layers=(b"layer bytes",)):
    """An image share as the host publishes it: only blobs, each named by its
    SHA-256. Returns (share, manifest digest, layer digests)."""
    share = tmp_path / "share"
    share.mkdir()
    config = _blob(share, json.dumps(image()).encode())
    descriptors = [{"digest": _blob(share, data), "size": len(data)} for data in layers]
    manifest = {
        "schemaVersion": 2,
        "mediaType": "application/vnd.oci.image.manifest.v1+json",
        "config": {"digest": config, "size": len(json.dumps(image()).encode())},
        "layers": descriptors,
    }
    return share, _blob(share, json.dumps(manifest).encode()), [entry["digest"] for entry in descriptors]


def test_the_image_is_assembled_from_the_share_and_verified(launcher, tmp_path):
    """Every launch, first boot or relaunch, assembles the same verified
    layout from the read-only share; the workspace stage holds no layer."""
    share, digest, (layer,) = _share(tmp_path)
    for name in ("first-boot", "restart"):
        layout = tmp_path / name
        launcher.assemble(share, digest, layout)
        blobs = layout / "blobs" / "sha256"
        assert (blobs / layer.removeprefix("sha256:")).read_bytes() == b"layer bytes"
        index = json.loads((layout / "index.json").read_text())
        assert [entry["digest"] for entry in index["manifests"]] == [digest]
        assert index["manifests"][0]["annotations"] == {"org.opencontainers.image.ref.name": "image"}
        assert index["manifests"][0]["size"] == (blobs / digest.removeprefix("sha256:")).stat().st_size
        assert json.loads((layout / "oci-layout").read_text()) == {"imageLayoutVersion": "1.0.0"}


def test_runtime_config_layout_copies_metadata_without_layers(launcher, tmp_path):
    share, digest, (layer,) = _share(tmp_path)
    (share / layer.removeprefix("sha256:")).unlink()
    layout = tmp_path / "layout"
    launcher.assemble(share, digest, layout, layers=False)
    manifest = json.loads((layout / "blobs/sha256" / digest.removeprefix("sha256:")).read_bytes())
    assert not (layout / "blobs/sha256" / layer.removeprefix("sha256:")).exists()
    assert (
        layout / "blobs/sha256" / manifest["config"]["digest"].removeprefix("sha256:")
    ).is_file()
    assert json.loads((layout / "index.json").read_text())["manifests"][0]["digest"] == digest


@pytest.mark.parametrize("fail", [False, True])
def test_published_root_uses_read_only_descriptor_and_umoci_then_releases_mounts(
    launcher, tmp_path, monkeypatch, fail
):
    import os

    monkeypatch.setattr(launcher, "RUNTIME", tmp_path / "runtime")
    launcher.RUNTIME.mkdir()
    share, manifest, _ = _share(tmp_path)
    data = b"opaque published EROFS fixture"
    digest = _blob(share, data)
    events, inherited = [], []

    @contextlib.contextmanager
    def mounted():
        events.append("share-open")
        try:
            yield share
        finally:
            events.append("share-close")

    def command(*argv, **kwargs):
        events.append(argv)
        if argv[0] == "mount":
            (fd,) = kwargs["pass_fds"]
            inherited.append(fd)
            assert argv[5] == f"/proc/self/fd/{fd}"
            assert os.read(fd, len(data)) == data
            os.lseek(fd, 0, os.SEEK_SET)
        elif argv[:3] == ("umoci", "raw", "runtime-config"):
            assert argv[argv.index("--rootfs") + 1] == str(launcher.RUNTIME / "bundle/rootfs")
            assert argv[argv.index("--uid-map") + 1] == "0:100000:65536"
            assert argv[argv.index("--gid-map") + 1] == "0:100000:65536"
            if fail:
                raise RuntimeError("converter failed")
            Path(argv[-1]).write_text(json.dumps(unpacked()))

    options = {"manifest": manifest, "rootfs": {"digest": digest, "size": len(data)}}

    def use():
        with launcher.published_root(
            options, SECURITY["id_map"], share=mounted, run=command
        ) as result:
            bundle, metadata, runtime = result
            assert bundle == launcher.RUNTIME / "bundle"
            assert metadata == image() and runtime == unpacked()
            assert events[0] == "share-open" and "share-close" not in events
            assert events[1][:5] == ("mount", "-t", "erofs", "-o", "loop,ro,nosuid,nodev")
            assert not (launcher.RUNTIME / "image").exists()
            assert not any(path.name.endswith(".erofs") for path in launcher.RUNTIME.rglob("*"))

    if fail:
        with pytest.raises(RuntimeError, match="converter failed"):
            use()
    else:
        use()
    assert events[-2] == ("umount", str(launcher.RUNTIME / "bundle/rootfs"))
    assert events[-1] == "share-close"
    with pytest.raises(OSError):
        os.fstat(inherited[0])


@pytest.mark.parametrize(
    "rootfs",
    [
        None,
        {},
        {"digest": "../bad", "size": 1},
        {"digest": "sha256:" + "a" * 64, "size": True},
        {"digest": "sha256:" + "a" * 64, "size": 0},
        {"digest": "sha256:" + "a" * 64, "size": 2**32 + 1},
        {"digest": "sha256:" + "a" * 64, "size": 1, "path": "/foreign"},
    ],
)
def test_invalid_published_root_descriptor_is_refused_before_share_access(launcher, rootfs):
    @contextlib.contextmanager
    def forbidden():
        raise AssertionError("invalid descriptor reached the share")
        yield

    with (
        pytest.raises(ValueError, match="rootfs"),
        launcher.published_root({"rootfs": rootfs}, SECURITY["id_map"], share=forbidden),
    ):
        raise AssertionError("invalid descriptor was accepted")


@pytest.mark.parametrize("victim", ["tampered", "size", "symlink", "fifo"])
def test_published_root_refuses_unverified_or_special_payload_before_mount(
    launcher, tmp_path, monkeypatch, victim
):
    import errno
    import os

    monkeypatch.setattr(launcher, "RUNTIME", tmp_path / "runtime")
    launcher.RUNTIME.mkdir()
    share, manifest, _ = _share(tmp_path)
    data = b"published payload"
    digest = _blob(share, data)
    path = share / digest.removeprefix("sha256:")
    size = len(data)
    if victim == "tampered":
        path.write_bytes(b"x" * size)
    elif victim == "size":
        size += 1
    else:
        path.unlink()
        if victim == "symlink":
            foreign = tmp_path / "foreign"
            foreign.write_bytes(data)
            path.symlink_to(foreign)
        else:
            os.mkfifo(path)
    calls, closed = [], []

    @contextlib.contextmanager
    def mounted():
        try:
            yield share
        finally:
            closed.append(True)

    expected = OSError if victim == "symlink" else ValueError
    with (
        pytest.raises(expected) as error,
        launcher.published_root(
            {"manifest": manifest, "rootfs": {"digest": digest, "size": size}},
            SECURITY["id_map"],
            share=mounted,
            run=lambda *argv, **_: calls.append(argv),
        ),
    ):
        raise AssertionError("invalid payload was mounted")
    if victim == "symlink":
        assert isinstance(error.value, OSError)
        assert error.value.errno == errno.ELOOP
    elif victim == "fifo":
        assert "regular file" in str(error.value)
    else:
        assert "digest mismatch" in str(error.value)
    assert calls == [] and closed == [True]


@pytest.mark.parametrize(
    "id_map",
    [
        {"containerID": 0, "hostID": 200000, "size": 65536},
        {"containerID": 0, "hostID": 100000, "size": 1},
    ],
)
def test_published_root_refuses_a_mapping_different_from_its_inode_contract(launcher, id_map):
    @contextlib.contextmanager
    def forbidden():
        raise AssertionError("mismatched mapping reached the share")
        yield

    with (
        pytest.raises(ValueError, match="mapping"),
        launcher.published_root(
            {"rootfs": {"digest": "sha256:" + "a" * 64, "size": 1}},
            id_map,
            share=forbidden,
        ),
    ):
        raise AssertionError("mismatched mapping was accepted")


@pytest.mark.parametrize("published", [False, True])
@pytest.mark.parametrize("fail", [False, True])
def test_run_selects_its_root_and_releases_overlay_before_published_lower(
    launcher, tmp_path, monkeypatch, published, fail
):
    runtime = tmp_path / "runtime"
    monkeypatch.setattr(launcher, "RUNTIME", runtime)
    monkeypatch.setattr(launcher, "MERGED", tmp_path / "merged")
    monkeypatch.setattr(launcher, "VOLUMES", tmp_path / "volumes")
    stage = tmp_path / "stage"
    stage.mkdir()
    options = {**SECURITY, "manifest": "sha256:" + "a" * 64, "args": [], "env": {}}
    if published:
        options["rootfs"] = {"digest": "sha256:" + "b" * 64, "size": 123}
    (stage / "options.json").write_text(json.dumps(options))
    events = []

    def result():
        bundle = runtime / "bundle"
        (bundle / "rootfs").mkdir(parents=True)
        return bundle, {"config": {}}, unpacked()

    def universal(digest, id_map):
        assert not published, "a published root must not unpack the OCI layers"
        assert digest == options["manifest"] and id_map == SECURITY["id_map"]
        events.append("unpack")
        return result()

    @contextlib.contextmanager
    def selected(selected_options, id_map):
        assert published and selected_options == options and id_map == SECURITY["id_map"]
        events.append("published-lower")
        try:
            yield result()
        finally:
            events.extend(["lower-close", "share-close"])

    class Workload:
        def __init__(self, argv):
            assert argv[argv.index("--bundle") + 1] == str(runtime / "bundle")
            events.append("workload")
            launcher.workload_started(stage)

        def wait(self, timeout=None):
            if fail and timeout is None:
                raise RuntimeError("workload failed")
            return 0

        def poll(self):
            return 0

    monkeypatch.setattr(launcher, "unpacked_root", universal)
    monkeypatch.setattr(launcher, "published_root", selected)
    monkeypatch.setattr(launcher, "mount_layer", lambda lower: events.append("overlay"))
    monkeypatch.setattr(launcher.os.path, "ismount", lambda path: path == launcher.MERGED)
    monkeypatch.setattr(launcher, "command", lambda *argv, **_: events.append("overlay-close"))
    monkeypatch.setattr(launcher.subprocess, "Popen", Workload)
    if fail:
        with pytest.raises(RuntimeError, match="workload failed"):
            launcher.run(stage)
    else:
        assert launcher.run(stage) == 0
    expected = ["published-lower" if published else "unpack", "overlay", "workload", "overlay-close"]
    if published:
        expected.extend(["lower-close", "share-close"])
    assert events == expected
    assert not runtime.exists()
    assert (stage / "ready").read_text() == "1\n"


@pytest.mark.parametrize("victim", ["manifest", "layer"])
def test_a_share_blob_that_is_not_the_one_named_is_refused(launcher, tmp_path, victim):
    share, digest, (layer,) = _share(tmp_path)
    named = digest if victim == "manifest" else layer
    (share / named.removeprefix("sha256:")).write_bytes(b"tampered")
    with pytest.raises(ValueError, match=r"digest mismatch|larger than"):
        launcher.assemble(share, digest, tmp_path / "layout")


def test_a_layer_whose_size_differs_from_its_descriptor_is_refused(launcher, tmp_path):
    share, digest, _ = _share(tmp_path)
    manifest = json.loads((share / digest.removeprefix("sha256:")).read_bytes())
    manifest["layers"][0]["size"] += 1
    forged = _blob(share, json.dumps(manifest).encode())
    with pytest.raises(ValueError, match="digest mismatch"):
        launcher.assemble(share, forged, tmp_path / "layout")


@pytest.mark.parametrize("named", ["sha256:" + "0" * 64, "sha256:../x", "md5:abc"])
def test_a_manifest_naming_a_blob_outside_the_share_is_refused(launcher, tmp_path, named):
    share, _, _ = _share(tmp_path)
    manifest = {"config": {"digest": named, "size": 1}, "layers": []}
    digest = _blob(share, json.dumps(manifest).encode())
    with pytest.raises((ValueError, FileNotFoundError)):
        launcher.assemble(share, digest, tmp_path / "layout")


def test_the_share_is_mounted_read_only_only_while_it_is_read(launcher, tmp_path, monkeypatch):
    monkeypatch.setattr(launcher, "IMAGE_SHARE", tmp_path / "image")
    calls = []
    with launcher.image_share(lambda *argv, **_: calls.append(argv)) as mounted:
        assert mounted == tmp_path / "image"
        assert calls == [("mount", "-t", "virtiofs", "-o", "ro,nosuid,nodev,noexec", "capsem-image", str(mounted))]
    assert calls[-1] == ("umount", str(mounted))


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
    options = {**SECURITY, "args": [], "env": {}, "workspace": "/workspace"}
    config = launcher.configure(unpacked(), image(), options)
    (mount,) = _mounts_at(config, "/workspace")
    assert mount["type"] == "bind"
    # The VM's /root share, seen through the workload's id map: VM uid 0 --
    # every entry the VirtioFS server reports -- is container root there.
    assert mount["source"] == str(launcher.WORKSPACE_VIEW)
    assert launcher.RUNTIME not in launcher.WORKSPACE_VIEW.parents, (
        "the runtime directory is removed recursively on exit; the workspace must not be under it"
    )
    assert {"bind", "nosuid", "nodev"} <= set(mount["options"])
    assert "ro" not in mount["options"], "a workload writes its outputs back to the workspace"


def test_the_stage_stays_hidden_from_the_container(launcher):
    """The launcher and its inputs live in the workspace, under .capsem-image.

    The launcher runs as root in the VM on every relaunch, and options.json
    carries the workload's environment. A container that could read or replace
    that directory would read those secrets or escape into the VM, so an empty
    read-only mount covers it, after the workspace mount that would expose it.
    """
    options = {**SECURITY, "args": [], "env": {}, "workspace": "/workspace"}
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
    config = launcher.configure(unpacked(), declared, {**SECURITY, "args": [], "env": {}, "workspace": "/workspace"})
    assert _mounts_at(config, "/workspace")[-1]["type"] == "bind", "the last mount at a path is the one seen"


def test_a_stage_written_before_the_workspace_mount_existed_starts_without_it(launcher):
    """A persistent VM staged by an older Capsem restarts exactly as it did."""
    config = launcher.configure(unpacked(), image(), {**SECURITY, "args": [], "env": {}})
    assert not _mounts_at(config, "/workspace")


@pytest.mark.parametrize(
    "workspace",
    ["workspace", "/proc", "/etc/ws", "/a/../b", "/", "/work//space", "/usr/lib/ws"],
)
def test_an_unsafe_workspace_mount_point_is_refused(launcher, workspace):
    """The mount point is launcher input: it must never land on a system path."""
    with pytest.raises(ValueError, match="unsafe workspace mount point"):
        launcher.configure(unpacked(), image(), {**SECURITY, "args": [], "env": {}, "workspace": workspace})


def test_the_host_decides_the_capabilities_and_the_syscall_filter(launcher):
    config = launcher.configure(unpacked(), image(), {**SECURITY, "args": [], "env": {}})
    assert config["linux"]["seccomp"] == SECURITY["seccomp"]
    assert config["process"]["capabilities"]["bounding"] == SECURITY["capabilities"]
    assert config["process"]["capabilities"]["ambient"] == []


@pytest.mark.parametrize("missing", ["capabilities", "seccomp", "id_map", "resources"])
def test_a_stage_without_a_filter_or_capabilities_is_refused(launcher, missing):
    options = {**SECURITY, "args": [], "env": {}}
    del options[missing]
    with pytest.raises(ValueError, match=missing):
        launcher.configure(unpacked(), image(), options)


def test_the_workload_runs_in_a_user_namespace_with_the_host_map(launcher):
    config = launcher.configure(unpacked(), image(), {**SECURITY, "args": [], "env": {}})
    assert {"type": "user"} in config["linux"]["namespaces"]
    assert config["linux"]["uidMappings"] == [SECURITY["id_map"]]
    assert config["linux"]["gidMappings"] == [SECURITY["id_map"]]


@pytest.mark.parametrize(
    "id_map",
    [
        {"containerID": 0, "hostID": 0, "size": 65536},
        {"containerID": 0, "hostID": 1, "size": 65536},
        {"containerID": 1, "hostID": 100000, "size": 65536},
        {"containerID": 0, "hostID": 100000, "size": 0},
        {"containerID": 0, "hostID": "100000", "size": 65536},
        {"containerID": 0, "hostID": 100000},
    ],
)
def test_a_map_that_reaches_vm_system_ids_is_refused(launcher, id_map):
    """Container root must never be a uid the VM trusts: the whole map sits
    above the VM's own system and user ids, and starts at container root."""
    with pytest.raises(ValueError, match="id map"):
        launcher.configure(unpacked(), image(), {**SECURITY, "id_map": id_map, "args": [], "env": {}})


def test_the_workload_is_sized_by_the_host(launcher):
    config = launcher.configure(unpacked(), image(), {**SECURITY, "args": [], "env": {}})
    limits = config["linux"]["resources"]
    assert limits["memory"] == {"limit": 1024 * 1024**2, "swap": 1024 * 1024**2}
    assert limits["cpu"] == {"quota": 175000, "period": 100000}
    assert limits["pids"] == {"limit": 4096}


@pytest.mark.parametrize(
    "resources",
    [
        {"memory_bytes": 0, "cpu_millis": 1000, "pids": 1},
        {"memory_bytes": "1", "cpu_millis": 1000, "pids": 1},
        {"memory_bytes": 1, "cpu_millis": 1000},
        {"memory_bytes": 1, "cpu_millis": 1000, "pids": 1, "devices": 1},
    ],
)
def test_malformed_resources_are_refused(launcher, resources):
    with pytest.raises(ValueError, match="resources"):
        launcher.configure(unpacked(), image(), {**SECURITY, "resources": resources, "args": [], "env": {}})


def test_an_image_needing_more_memory_than_the_vm_gives_is_refused(launcher):
    hungry = image()
    hungry["config"]["Labels"] = {launcher.MEMORY_LABEL: "2048"}
    with pytest.raises(ValueError, match="needs 2048 MiB"):
        launcher.configure(unpacked(), hungry, {**SECURITY, "args": [], "env": {}})
    hungry["config"]["Labels"] = {launcher.MEMORY_LABEL: "1024"}
    launcher.configure(unpacked(), hungry, {**SECURITY, "args": [], "env": {}})
    hungry["config"]["Labels"] = {launcher.MEMORY_LABEL: "1G"}
    with pytest.raises(ValueError, match="whole number of MiB"):
        launcher.configure(unpacked(), hungry, {**SECURITY, "args": [], "env": {}})


def test_image_volumes_live_on_the_vm_overlay_not_tmpfs(launcher):
    config = launcher.configure(unpacked(), image(), {**SECURITY, "args": [], "env": {}})
    (data,) = _mounts_at(config, "/data")
    assert data["type"] == "bind"
    assert data["source"] == str(launcher.volume_dir("/data"))
    assert launcher.VOLUMES in launcher.volume_dir("/data").parents
    assert launcher.RUNTIME not in launcher.volume_dir("/data").parents, "RUNTIME is removed on exit"
    assert "ro" not in data["options"]


def test_volume_directories_never_collide(launcher):
    paths = ["/a/b", "/a_b", "/a/b/", "/data", "/var/lib/data"]
    assert len({launcher.volume_dir(path) for path in paths}) == len(paths)


def test_a_volume_is_seeded_from_the_image_once_and_then_kept(launcher, tmp_path, monkeypatch):
    monkeypatch.setattr(launcher, "VOLUMES", tmp_path / "volumes")
    monkeypatch.setattr(launcher.os, "chown", lambda *args: None)
    rootfs = tmp_path / "rootfs"
    (rootfs / "data").mkdir(parents=True)
    (rootfs / "data" / "seed").write_text("from the image")
    launcher.prepare_volumes({"/data": {}, "/empty": {}}, rootfs, SECURITY["id_map"])
    seeded = launcher.volume_dir("/data")
    assert (seeded / "seed").read_text() == "from the image"
    assert launcher.volume_dir("/empty").is_dir()
    # Session state survives a relaunch: the image never overwrites it again.
    (seeded / "seed").write_text("from the session")
    launcher.prepare_volumes({"/data": {}}, rootfs, SECURITY["id_map"])
    assert (seeded / "seed").read_text() == "from the session"


def test_a_volume_is_never_seeded_through_a_symlink_the_workload_planted(launcher, tmp_path, monkeypatch):
    """The root is writable, so by a later launch the workload may have made a
    volume's parent a symlink to the VM's own files. Seeding runs as VM root
    and must not copy what that link reaches into the workload's volume."""
    monkeypatch.setattr(launcher, "VOLUMES", tmp_path / "volumes")
    monkeypatch.setattr(launcher.os, "chown", lambda *args: None)
    rootfs = tmp_path / "rootfs"
    rootfs.mkdir()
    secret = tmp_path / "vm-only"
    (secret / "data").mkdir(parents=True)
    (secret / "data" / "key").write_text("VM secret")
    (rootfs / "var").symlink_to(secret)
    launcher.prepare_volumes({"/var/data": {}}, rootfs, SECURITY["id_map"])
    volume = launcher.volume_dir("/var/data")
    assert volume.is_dir() and not any(volume.iterdir())


def _fake_unpack(launcher, tmp_path, monkeypatch):
    """umoci stubbed, the share a plain directory: count unpacks and share
    mounts, write what umoci would. Returns (unpacks, mounts, share, digest)."""
    monkeypatch.setattr(launcher, "ROOTS", tmp_path / "roots")
    monkeypatch.setattr(launcher, "RUNTIME", tmp_path / "runtime")
    (tmp_path / "runtime").mkdir(exist_ok=True)
    share, digest, _ = _share(tmp_path)
    unpacks, mounts = [], []

    def command(*argv, **_):
        unpacks.append(argv)
        bundle = Path(argv[-1])
        (bundle / "rootfs").mkdir(parents=True)
        (bundle / "config.json").write_text(json.dumps(unpacked()))

    @contextlib.contextmanager
    def mounted():
        mounts.append(share)
        yield share

    monkeypatch.setattr(launcher, "command", command)
    monkeypatch.setattr(launcher, "image_share", mounted)
    return unpacks, mounts, digest


def test_a_named_session_unpacks_its_image_once(launcher, tmp_path, monkeypatch):
    unpacks, mounts, digest = _fake_unpack(launcher, tmp_path, monkeypatch)
    first = launcher.unpacked_root(digest, SECURITY["id_map"], launcher.image_share)
    second = launcher.unpacked_root(digest, SECURITY["id_map"], launcher.image_share)
    assert len(unpacks) == 1, "the relaunch reused the unpacked root"
    assert len(mounts) == 1, "a relaunch never reads the share"
    assert first == second
    bundle, image_config, runtime = first
    assert bundle == launcher.ROOTS / digest.removeprefix("sha256:") / "bundle"
    assert image_config == image() and runtime == unpacked()
    assert "--uid-map" in unpacks[0]
    assert not (launcher.RUNTIME / "image").exists(), "the assembled layout is not kept"


def test_only_the_current_digest_keeps_a_root(launcher, tmp_path, monkeypatch):
    _, _, digest = _fake_unpack(launcher, tmp_path, monkeypatch)
    stale = tmp_path / "roots" / ("d" * 64)
    stale.mkdir(parents=True)
    launcher.unpacked_root(digest, SECURITY["id_map"], launcher.image_share)
    assert not stale.exists()


def test_a_tampered_share_blob_fails_the_unpack(launcher, tmp_path, monkeypatch):
    unpacks, _, digest = _fake_unpack(launcher, tmp_path, monkeypatch)
    (tmp_path / "share" / digest.removeprefix("sha256:")).write_bytes(b'{"config": {}, "layers": []}')
    with pytest.raises(ValueError, match="digest mismatch"):
        launcher.unpacked_root(digest, SECURITY["id_map"], launcher.image_share)
    assert not unpacks, "umoci never saw an unverified blob"


@pytest.mark.parametrize("named", [None, "sha256:short", "sha256:" + "B" * 64, "md5:" + "b" * 64, "sha256:../" + "b" * 61])
def test_options_naming_no_valid_manifest_digest_are_refused(launcher, named):
    with pytest.raises(ValueError, match="manifest digest"):
        launcher.manifest_digest({"manifest": named} if named is not None else {})


@pytest.mark.parametrize(
    ("memory", "shm"),
    [(128 * 1024**2, 64 * 1024**2), (2 * 1024**3, 512 * 1024**2), (16 * 1024**3, 1024**3)],
)
def test_the_workload_has_shared_memory_sized_from_its_memory(launcher, memory, shm):
    options = {**SECURITY, "resources": {**RESOURCES, "memory_bytes": memory}, "args": [], "env": {}}
    config = launcher.configure(unpacked(), image(), options)
    (mount,) = _mounts_at(config, "/dev/shm")
    assert mount["type"] == "tmpfs"
    assert f"size={shm}" in mount["options"]
    assert {"nosuid", "nodev", "noexec"} <= set(mount["options"])
    destinations = [m["destination"] for m in config["mounts"]]
    assert destinations.index("/dev/shm") > destinations.index("/dev"), "mounted inside /dev, after it"


def test_the_workload_has_its_own_pseudo_terminals(launcher):
    """`runc exec -t` (the session terminal) opens /dev/ptmx: the workload gets
    a private devpts instance, never the VM's terminals, with no group option
    the user namespace does not map."""
    config = launcher.configure(unpacked(), image(), {**SECURITY, "args": [], "env": {}})
    (mount,) = _mounts_at(config, "/dev/pts")
    assert mount["type"] == "devpts"
    assert {"newinstance", "ptmxmode=0666", "nosuid", "noexec"} <= set(mount["options"])
    assert not any(option.startswith("gid=") for option in mount["options"])
    destinations = [m["destination"] for m in config["mounts"]]
    assert destinations.index("/dev/pts") > destinations.index("/dev"), "mounted inside /dev, after it"


def _layer(launcher, tmp_path, monkeypatch, mounted):
    monkeypatch.setattr(launcher, "LAYER", tmp_path / "layer")
    monkeypatch.setattr(launcher, "MERGED", tmp_path / "merged")
    (tmp_path / "layer").mkdir()
    monkeypatch.setattr(launcher.os.path, "ismount", lambda path: mounted and Path(path) == tmp_path / "layer")
    monkeypatch.setattr(launcher.os, "chown", lambda *args: None)
    lower = tmp_path / "roots" / "aa" / "rootfs"
    lower.mkdir(parents=True, mode=0o755)
    calls = []
    return lower, calls


def test_the_session_layer_sits_above_whichever_image_runs(launcher, tmp_path, monkeypatch):
    """The workload writes a copy-on-write layer over the image. It belongs to
    the session, not the image: index, redirects and metacopy stay off, so
    the same layer mounts over a different image after the session changes
    image, keeping what the session wrote."""
    lower, calls = _layer(launcher, tmp_path, monkeypatch, mounted=True)
    launcher.mount_layer(lower, run=lambda *argv: calls.append(argv))
    (mount,) = calls
    assert mount[:5] == ("mount", "-t", "overlay", "overlay", "-o")
    options = dict(option.split("=", 1) for option in mount[5].split(","))
    assert options["lowerdir"] == str(lower)
    assert options["upperdir"] == str(tmp_path / "layer" / "upper")
    assert options["workdir"] == str(tmp_path / "layer" / "work")
    assert {options[key] for key in ("index", "redirect_dir", "metacopy")} == {"off"}
    assert mount[6] == str(tmp_path / "merged")
    # The merged / is the image's /: same mode (owner too, chown stubbed here).
    assert (tmp_path / "layer" / "upper").stat().st_mode & 0o7777 == lower.stat().st_mode & 0o7777

    # A new image: the same upper, under a different lower.
    other = tmp_path / "roots" / "bb" / "rootfs"
    other.mkdir(parents=True)
    calls.clear()
    launcher.mount_layer(other, run=lambda *argv: calls.append(argv))
    assert f"upperdir={tmp_path / 'layer' / 'upper'}" in calls[0][5]
    assert f"lowerdir={other}" in calls[0][5]


def test_a_session_without_its_layer_refuses_to_launch(launcher, tmp_path, monkeypatch):
    """Never an overlay upper on the VM's own overlay root, which the kernel
    refuses anyway, and never a silently ephemeral layer: no layer, no launch."""
    lower, calls = _layer(launcher, tmp_path, monkeypatch, mounted=False)
    with pytest.raises(RuntimeError, match="not mounted"):
        launcher.mount_layer(lower, run=lambda *argv: calls.append(argv))
    assert calls == []
