"""Run one verified OCI layout; session workspace retains its restart inputs."""

import hashlib
import json
import re
import shutil
import subprocess
import sys
from pathlib import Path, PurePosixPath

RUNTIME = Path("/var/tmp/capsem-container")
CONTAINER = "workload"

# The VM trusts the Capsem CA through this bundle; the container gets the same
# file read-only, so TLS it opens terminates at the host MITM like VM traffic.
CA_BUNDLE = "/etc/ssl/certs/ca-certificates.crt"
IPTABLES = "iptables-nft"
# One veth pair per workload: the VM end is the container's only gateway.
HOST_LINK = "capsem0"
CONTAINER_LINK = "eth0"
GATEWAY = "10.0.1.1"
CONTAINER_ADDRESS = "10.0.1.2"
NAT_CHAIN = "CAPSEM_CONTAINER_NAT"
INPUT_CHAIN = "CAPSEM_CONTAINER_IN"
REDIRECT_RULE = re.compile(
    r"^-A OUTPUT -p (udp|tcp) -m \1 --dport (\d+) -j REDIRECT --to-ports (\d+)$"
)


def configure(unpacked, image, options):
    process = unpacked["process"]
    metadata = image.get("config") or {}
    if options["args"]:
        process["args"] = (metadata.get("Entrypoint") or []) + options["args"]
    if not process.get("args") or not process["args"][0]:
        raise ValueError("image has no command")
    environment = dict(entry.split("=", 1) for entry in process.get("env", []))
    environment.update(
        {
            key: CA_BUNDLE
            for key in (
                "SSL_CERT_FILE",
                "REQUESTS_CA_BUNDLE",
                "CURL_CA_BUNDLE",
                "NODE_EXTRA_CA_CERTS",
            )
        }
    )
    environment.update(options["env"])
    process.update(
        terminal=False,
        env=[f"{key}={value}" for key, value in environment.items()],
        noNewPrivileges=True,
        capabilities={
            key: [
                "CAP_CHOWN",
                "CAP_DAC_OVERRIDE",
                "CAP_FOWNER",
                "CAP_SETGID",
                "CAP_SETUID",
            ]
            if key in {"bounding", "effective", "permitted"}
            else []
            for key in ("bounding", "effective", "permitted", "inheritable", "ambient")
        },
        rlimits=[{"type": "RLIMIT_NOFILE", "hard": 4096, "soft": 4096}],
    )
    volumes = metadata.get("Volumes") or {}
    if len(volumes) > 32:
        raise ValueError("image declares too many volumes")
    for volume in volumes:
        path = PurePosixPath(volume)
        if (
            not path.is_absolute()
            or ".." in path.parts
            or str(path) != volume
            or len(path.parts) < 2
            or len(volume) > 4096
            or path.parts[1]
            in {"proc", "dev", "sys", "usr", "bin", "sbin", "lib", "lib64", "etc"}
        ):
            raise ValueError(f"unsafe image volume: {volume}")
    mounts = [
        {
            "destination": "/proc",
            "type": "proc",
            "source": "proc",
            "options": ["nosuid", "nodev", "noexec"],
        },
        {
            "destination": "/dev",
            "type": "tmpfs",
            "source": "tmpfs",
            "options": ["nosuid", "noexec", "mode=755", "size=1m"],
        },
    ]
    mounts.extend(
        {
            "destination": path,
            "type": "tmpfs",
            "source": "tmpfs",
            "options": ["nosuid", "nodev", "noexec", "mode=1777", "size=64m"],
        }
        for path in sorted({"/scratch", "/tmp", "/run", *volumes})
    )
    mounts.extend(
        {
            "destination": destination,
            "type": "bind",
            "source": source,
            "options": ["bind", "ro", "nosuid", "nodev", "noexec"],
        }
        for destination, source in (
            ("/etc/resolv.conf", str(RUNTIME / "resolv.conf")),
            (CA_BUNDLE, CA_BUNDLE),
        )
    )
    return {
        "ociVersion": "1.0.2",
        "root": {"path": "rootfs", "readonly": True},
        "hostname": "container",
        "process": process,
        "mounts": mounts,
        "hooks": {
            "prestart": [
                {
                    "path": "/usr/bin/python3",
                    "args": [
                        "/usr/bin/python3",
                        str(Path(__file__).resolve()),
                        "--network-ready",
                    ],
                    "env": ["PATH=/usr/sbin:/usr/bin:/sbin:/bin"],
                    "timeout": 5,
                }
            ]
        },
        "linux": {
            "namespaces": [
                {"type": name} for name in ("pid", "mount", "ipc", "uts", "network")
            ],
            "cgroupsPath": "/capsem-container",
            "resources": {
                "memory": {"limit": 256 * 1024**2, "swap": 256 * 1024**2},
                "pids": {"limit": 256},
                "cpu": {"quota": 100000, "period": 100000},
            },
            "maskedPaths": [
                "/proc/kcore",
                "/proc/keys",
                "/proc/timer_list",
                "/proc/scsi",
            ],
            "readonlyPaths": [
                "/proc/sys",
                "/proc/sysrq-trigger",
                "/proc/irq",
                "/proc/bus",
            ],
            "seccomp": {
                "defaultAction": "SCMP_ACT_ALLOW",
                "syscalls": [
                    {
                        "names": ["socket"],
                        "action": "SCMP_ACT_ERRNO",
                        "errnoRet": 1,
                        "args": [{"index": 0, "value": 40, "op": "SCMP_CMP_EQ"}],
                    }
                ],
            },
        },
    }


def command(*args, check=True, **kwargs):
    return subprocess.run(args, check=check, timeout=30, **kwargs)


def resolv_conf():
    """Same resolver contract as the VM, pointed at the container's gateway."""
    return f"nameserver {GATEWAY}\noptions timeout:6 attempts:2\n"


def derive_redirects(output_rules):
    """(protocol, destination port, proxy port) for every VM interception rule.

    The VM's `nat OUTPUT` chain is the single statement of which ports are
    intercepted and where; mirroring it keeps the container on exactly the
    VM's policy when that list changes.
    """
    redirects = []
    for line in output_rules.splitlines():
        match = REDIRECT_RULE.match(line.strip())
        if match:
            redirects.append((match.group(1), int(match.group(2)), int(match.group(3))))
    return redirects


def network_ready(pid, run=command, sysctl_root=Path("/proc/sys")):
    """Prestart hook: give the container a gateway that leads only to the VM's
    interception proxies.

    Runs in the VM's network namespace with the container's pid. Traffic from
    the container's end of the veth is DNAT'ed to the loopback proxies the VM
    already uses; everything else that reaches the VM from that interface is
    dropped, so the container cannot talk to the VM's other listeners, its
    dummy address, or any peer the VM might later be attached to.
    """
    namespace = ["nsenter", "-t", str(pid), "-n"]
    run("ip", "link", "add", HOST_LINK, "type", "veth", "peer", "name", "capsem1")
    run("ip", "link", "set", "capsem1", "netns", str(pid))
    run("ip", "addr", "add", f"{GATEWAY}/30", "dev", HOST_LINK)
    run("ip", "link", "set", HOST_LINK, "up")
    # DNAT to 127.0.0.1 from another interface is only routable with this.
    (sysctl_root / "net/ipv4/conf" / HOST_LINK / "route_localnet").write_text("1\n")
    run(*namespace, "ip", "link", "set", "lo", "up")
    run(*namespace, "ip", "link", "set", "capsem1", "name", CONTAINER_LINK)
    run(*namespace, "ip", "addr", "add", f"{CONTAINER_ADDRESS}/30", "dev", CONTAINER_LINK)
    run(*namespace, "ip", "link", "set", CONTAINER_LINK, "up")
    run(*namespace, "ip", "route", "add", "default", "via", GATEWAY)

    rules = run(IPTABLES, "-t", "nat", "-S", "OUTPUT", capture_output=True, text=True).stdout
    redirects = derive_redirects(rules)
    if not redirects:
        raise ValueError("VM has no interception rules for the container to mirror")
    for table, chain in (("nat", NAT_CHAIN), ("filter", INPUT_CHAIN)):
        table_args = ["-t", table] if table == "nat" else []
        run(IPTABLES, *table_args, "-N", chain, check=False)
        run(IPTABLES, *table_args, "-F", chain)
    for protocol, port, proxy in redirects:
        run(
            IPTABLES, "-t", "nat", "-A", NAT_CHAIN, "-i", HOST_LINK, "-p", protocol,
            "--dport", str(port), "-j", "DNAT", "--to-destination", f"127.0.0.1:{proxy}",
        )
        run(
            IPTABLES, "-A", INPUT_CHAIN, "-i", HOST_LINK, "-d", "127.0.0.1", "-p", protocol,
            "--dport", str(proxy), "-j", "ACCEPT",
        )
    run(IPTABLES, "-A", INPUT_CHAIN, "-i", HOST_LINK, "-j", "DROP")
    for table_args, parent, target in (
        (["-t", "nat"], "PREROUTING", ["-j", NAT_CHAIN]),
        ([], "INPUT", ["-j", INPUT_CHAIN]),
        ([], "FORWARD", ["-i", HOST_LINK, "-j", "DROP"]),
    ):
        if run(IPTABLES, *table_args, "-C", parent, *target, check=False).returncode != 0:
            run(IPTABLES, *table_args, "-I", parent, *target)


def assemble(stage, layout):
    """Reassemble bounded uploads and verify each file before umoci sees it."""
    transfer = json.loads((stage / "transfer.json").read_text())
    for entry in transfer:
        relative = PurePosixPath(entry["path"])
        if relative.is_absolute() or ".." in relative.parts:
            raise ValueError("unsafe OCI transfer path")
        destination = layout / relative
        destination.parent.mkdir(parents=True, exist_ok=True)
        digest = hashlib.sha256()
        with destination.open("xb") as output:
            for number in range(entry["parts"]):
                part = stage / f"{entry['key']}-{number}"
                data = part.read_bytes()
                output.write(data)
                digest.update(data)
        if digest.hexdigest() != entry["sha256"]:
            raise ValueError("OCI upload digest mismatch")


def run(stage):
    RUNTIME.mkdir(mode=0o700)  # A second workload cannot overwrite live state.
    state = RUNTIME / "state"
    runc = ["runc", "--rootless=true", "--root", str(state)]
    process = None
    try:
        layout = RUNTIME / "image"
        layout.mkdir()
        assemble(stage, layout)
        bundle = RUNTIME / "bundle"
        command("umoci", "unpack", "--image", f"{layout}:image", str(bundle))
        index = json.loads((layout / "index.json").read_text())
        manifest = json.loads(
            (
                layout / "blobs/sha256" / index["manifests"][0]["digest"].split(":")[1]
            ).read_text()
        )
        image = json.loads(
            (
                layout / "blobs/sha256" / manifest["config"]["digest"].split(":")[1]
            ).read_text()
        )
        config_path = bundle / "config.json"
        config = configure(
            json.loads(config_path.read_text()),
            image,
            json.loads((stage / "options.json").read_text()),
        )
        config_path.write_text(json.dumps(config))
        (RUNTIME / "resolv.conf").write_text(resolv_conf())
        (stage / "ready").write_text("1\n")
        pid_file = RUNTIME / "workload.pid"
        process = subprocess.Popen(
            [
                *runc,
                "run",
                "--no-new-keyring",
                "--pid-file",
                str(pid_file),
                "--bundle",
                str(bundle),
                CONTAINER,
            ]
        )
        return process.wait()
    finally:
        if (state / CONTAINER).exists():
            command(*runc, "delete", "--force", CONTAINER)
        if process is not None and process.poll() is None:
            process.wait(timeout=5)
        shutil.rmtree(RUNTIME)


if __name__ == "__main__":
    if sys.argv[1:] == ["--network-ready"]:
        pid = int(json.load(sys.stdin)["pid"])
        if pid <= 1:
            raise ValueError("invalid container network namespace pid")
        network_ready(pid)
    else:
        sys.exit(run(Path(sys.argv[1])))
