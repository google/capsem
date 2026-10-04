"""Run one verified OCI layout; session workspace retains its restart inputs."""

import ctypes
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
from pathlib import Path, PurePosixPath

RUNTIME = Path("/var/tmp/capsem-container")
# The VM workspace seen through the workload's id map. Outside RUNTIME, which
# is removed recursively on exit and must never reach into the workspace.
WORKSPACE_VIEW = Path("/var/tmp/capsem-workspace")
# An image's VOLUMEs: on the VM's ext4 system overlay, so they die with an
# ephemeral VM and persist with a named one. Outside RUNTIME for the same
# reason as the workspace view.
VOLUMES = Path("/var/lib/capsem/volumes")
# Unpacked image roots, one per digest, on the same overlay: a named session
# unpacks its image once instead of on every launch.
ROOTS = Path("/var/lib/capsem/roots")
DIGEST = re.compile(r"^sha256:[0-9a-f]{64}$")
# Every id the workload maps lies at or above this, clear of the VM's own
# system and user ids, so container root is no uid the VM trusts.
LOWEST_MAPPED_ID = 65536
# An image declares the memory it cannot run without, in MiB.
MEMORY_LABEL = "org.capsem.memory.min"
CPU_PERIOD = 100000
# The VM workspace (the host-visible share) and, inside it, this launcher's
# own stage. The service names where the container sees the workspace.
VM_WORKSPACE = "/root"
STAGE = ".capsem-image"
CONTAINER = "workload"
# The one runtime state root: the launch and every later exec name it.
RUNC = ("runc", "--rootless=true", "--root", str(RUNTIME / "state"))

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
# The VM's network cables: one tap per network it is plugged into, each to
# that network's switch (capsem-tun names them cable<N>). They come and go
# while the container runs, so rules name them all by prefix.
CABLES = "cable+"
# `-m tcp` appears only with a port match; a destination-only rule has none.
REDIRECT_RULE = re.compile(
    r"^-A OUTPUT (?:-d (\S+) )?-p (udp|tcp) (?:-m \2 --dport (\d+) )?"
    r"-j REDIRECT --to-ports (\d+)$"
)
RETURN_RULE = re.compile(r"^-A OUTPUT -d (\S+) -j RETURN$")

CLONE_NEWUSER = 0x10000000
# The mount API's syscalls have one number on every architecture.
OPEN_TREE, MOVE_MOUNT, MOUNT_SETATTR = 428, 429, 442
OPEN_TREE_CLONE = 1
AT_FDCWD = -100
AT_EMPTY_PATH = 0x1000
MOVE_MOUNT_F_EMPTY_PATH = 0x4
MOUNT_ATTR_IDMAP = 0x00100000


class MountAttr(ctypes.Structure):
    _fields_ = [(name, ctypes.c_uint64) for name in ("attr_set", "attr_clr", "propagation", "userns_fd")]


def _safe_mount_point(value):
    """An absolute, normalized path the container may mount something at."""
    path = PurePosixPath(value)
    return (
        isinstance(value, str)
        and path.is_absolute()
        and ".." not in path.parts
        and str(path) == value
        and len(path.parts) >= 2
        and len(value) <= 4096
        and path.parts[1] not in {"proc", "dev", "sys", "usr", "bin", "sbin", "lib", "lib64", "etc"}
    )


def configure(unpacked, image, options):
    for key in ("capabilities", "seccomp", "id_map", "resources"):
        if key not in options:
            raise ValueError(f"stage options carry no {key}; refusing to run the workload without it")
    process = unpacked["process"]
    metadata = image.get("config") or {}
    id_map = checked_id_map(options["id_map"])
    resources = checked_resources(options["resources"])
    needs = (metadata.get("Labels") or {}).get(MEMORY_LABEL)
    if needs is not None:
        if not (isinstance(needs, str) and needs.isdigit()):
            raise ValueError(f"image label {MEMORY_LABEL} must be a whole number of MiB")
        has = resources["memory_bytes"] // (1024 * 1024)
        if int(needs) > has:
            raise ValueError(f"image needs {needs} MiB of memory; this VM gives its workload {has} MiB")
    if options["args"]:
        process["args"] = (metadata.get("Entrypoint") or []) + options["args"]
    if not process.get("args") or not process["args"][0]:
        raise ValueError("image has no command")
    environment = dict(entry.split("=", 1) for entry in process.get("env", []))
    environment.update(
        dict.fromkeys(("SSL_CERT_FILE", "REQUESTS_CA_BUNDLE", "CURL_CA_BUNDLE", "NODE_EXTRA_CA_CERTS"), CA_BUNDLE)
    )
    environment.update(options["env"])
    process.update(
        terminal=False,
        env=[f"{key}={value}" for key, value in environment.items()],
        noNewPrivileges=True,
        # The host decides what the workload holds and which syscalls it may
        # make (capsem-core container::seccomp); a stage without them is
        # refused rather than run with the runtime's defaults.
        capabilities={
            key: list(options["capabilities"]) if key in {"bounding", "effective", "permitted"} else []
            for key in ("bounding", "effective", "permitted", "inheritable", "ambient")
        },
        rlimits=[{"type": "RLIMIT_NOFILE", "hard": 4096, "soft": 4096}],
    )
    volumes = metadata.get("Volumes") or {}
    if len(volumes) > 32:
        raise ValueError("image declares too many volumes")
    for volume in volumes:
        if not _safe_mount_point(volume):
            raise ValueError(f"unsafe image volume: {volume}")
    workspace = options.get("workspace")
    if workspace is not None and not _safe_mount_point(workspace):
        raise ValueError(f"unsafe workspace mount point: {workspace}")
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
        {
            "destination": "/dev/shm",
            "type": "tmpfs",
            "source": "shm",
            "options": ["nosuid", "nodev", "noexec", "mode=1777", f"size={shm_bytes(resources)}"],
        },
    ]
    mounts.extend(
        {
            "destination": path,
            "type": "tmpfs",
            "source": "tmpfs",
            "options": ["nosuid", "nodev", "noexec", "mode=1777", "size=64m"],
        }
        for path in sorted({"/scratch", "/tmp", "/run"} - set(volumes))
    )
    # The image's declared state, on the VM's system overlay (volume_dir).
    mounts.extend(
        {
            "destination": path,
            "type": "bind",
            "source": str(volume_dir(path)),
            "options": ["bind", "rw", "nosuid", "nodev"],
        }
        for path in sorted(volumes)
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
    if workspace is not None:
        # After the volumes, so an image's own volume at this path cannot hide
        # it. Writable: a workload leaves its outputs here for the host.
        mounts.append(
            {
                "destination": workspace,
                "type": "bind",
                "source": str(WORKSPACE_VIEW),
                "options": ["bind", "rw", "nosuid", "nodev"],
            }
        )
        # The stage holds this launcher, which runs as root in the VM on every
        # relaunch, and options.json with the workload's environment: the
        # container must neither read nor replace it.
        mounts.append(
            {
                "destination": f"{workspace}/{STAGE}",
                "type": "tmpfs",
                "source": "tmpfs",
                "options": ["ro", "nosuid", "nodev", "noexec", "mode=000", "size=4k"],
            }
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
                {"type": name} for name in ("user", "pid", "mount", "ipc", "uts", "network")
            ],
            "uidMappings": [id_map],
            "gidMappings": [id_map],
            "cgroupsPath": "/capsem-container",
            "resources": {
                "memory": {"limit": resources["memory_bytes"], "swap": resources["memory_bytes"]},
                "pids": {"limit": resources["pids"]},
                "cpu": {"quota": resources["cpu_millis"] * CPU_PERIOD // 1000, "period": CPU_PERIOD},
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
            "seccomp": options["seccomp"],
        },
    }


def checked_id_map(id_map):
    """The host's map, refused unless container root and every id after it
    land at or above LOWEST_MAPPED_ID."""
    if (
        not isinstance(id_map, dict)
        or set(id_map) != {"containerID", "hostID", "size"}
        or not all(type(value) is int for value in id_map.values())
        or id_map["containerID"] != 0
        or id_map["hostID"] < LOWEST_MAPPED_ID
        or not 0 < id_map["size"] <= 65536
    ):
        raise ValueError(f"refusing id map {id_map!r}: container root must map above the VM's own ids")
    return id_map


def volume_dir(path):
    """Where the VM keeps the image volume mounted at `path`: readable, and
    distinct for every path (`/a/b` and `/a_b` never share)."""
    digest = hashlib.sha256(path.encode()).hexdigest()[:16]
    return VOLUMES / f"{path.strip('/').replace('/', '_')}-{digest}"


def prepare_volumes(volumes, rootfs, id_map):
    """Create each image volume once, seeded with the image's own content at
    that path (already owned within the map by umoci), or empty and owned by
    the mapped root. An existing volume is the session's state and is kept."""
    VOLUMES.mkdir(parents=True, exist_ok=True, mode=0o711)
    for path in volumes:
        target = volume_dir(path)
        if target.exists():
            continue
        seed = rootfs / path.lstrip("/")
        staging = target.with_name(target.name + ".new")
        shutil.rmtree(staging, ignore_errors=True)
        if seed.is_dir() and not seed.is_symlink():
            command("cp", "-a", "--", str(seed), str(staging))
        else:
            staging.mkdir(mode=0o755)
            os.chown(staging, id_map["hostID"], id_map["hostID"])
        staging.rename(target)


def shm_bytes(resources):
    """POSIX shared memory for the workload: a quarter of its memory, at
    least 64 MiB and at most 1 GiB. Chromium renders into it and draws blank
    windows below about 256 MiB. Its pages are charged to the workload's
    cgroup like any other memory, so the size widens no limit."""
    return max(64 * 1024**2, min(1024**3, resources["memory_bytes"] // 4))


def checked_resources(resources):
    """The host's sizing, refused unless every limit is a positive integer."""
    if (
        not isinstance(resources, dict)
        or set(resources) != {"memory_bytes", "cpu_millis", "pids"}
        or not all(type(value) is int and value > 0 for value in resources.values())
    ):
        raise ValueError(f"refusing workload resources {resources!r}")
    return resources


def _syscall_check(result):
    if result < 0:
        number = ctypes.get_errno()
        raise OSError(number, os.strerror(number))
    return result


def idmap_workspace(id_map, target):
    """Mount the VM workspace at `target` through the workload's id map.

    The VirtioFS server reports every workspace entry as VM uid 0; through this
    mount that is container root, so the workload can chown, `tar -x` and
    `cp -a` there. runc 1.1 makes no idmapped mounts; its bind of this one
    keeps the idmap. The map comes from a user namespace held open by a child
    for as long as the mount is being made.
    """
    libc = ctypes.CDLL(None, use_errno=True)
    ready_read, ready_write = os.pipe()
    hold_read, hold_write = os.pipe()
    child = os.fork()
    if child == 0:
        os.close(ready_read)
        os.close(hold_write)
        os.write(ready_write, b"1" if libc.unshare(CLONE_NEWUSER) == 0 else b"0")
        os.read(hold_read, 1)
        os._exit(0)
    os.close(ready_write)
    os.close(hold_read)
    try:
        if os.read(ready_read, 1) != b"1":
            raise OSError("cannot create the workload's user namespace")
        mapping = f"{id_map['containerID']} {id_map['hostID']} {id_map['size']}\n"
        for name in ("uid_map", "gid_map"):
            Path(f"/proc/{child}/{name}").write_text(mapping)
        userns = os.open(f"/proc/{child}/ns/user", os.O_RDONLY | os.O_CLOEXEC)
        tree = None
        try:
            tree = _syscall_check(
                libc.syscall(OPEN_TREE, AT_FDCWD, VM_WORKSPACE.encode(), OPEN_TREE_CLONE | os.O_CLOEXEC)
            )
            attr = MountAttr(MOUNT_ATTR_IDMAP, 0, 0, userns)
            _syscall_check(
                libc.syscall(MOUNT_SETATTR, tree, b"", AT_EMPTY_PATH, ctypes.byref(attr), ctypes.sizeof(attr))
            )
            _syscall_check(libc.syscall(MOVE_MOUNT, tree, b"", AT_FDCWD, str(target).encode(), MOVE_MOUNT_F_EMPTY_PATH))
        finally:
            if tree is not None:
                os.close(tree)
            os.close(userns)
    finally:
        os.close(ready_read)
        os.close(hold_write)
        os.waitpid(child, 0)


def command(*args, check=True, **kwargs):
    return subprocess.run(args, check=check, timeout=30, **kwargs)


def resolv_conf():
    """Same resolver contract as the VM, pointed at the container's gateway."""
    return f"nameserver {GATEWAY}\noptions timeout:6 attempts:2\n"


def derive_redirects(output_rules):
    """(protocol, destination port, proxy port, destination) for every VM
    interception rule; the port or the destination may be None when the rule
    matched on the other alone.

    The VM's `nat OUTPUT` chain is the single statement of which ports are
    intercepted and where; mirroring it keeps the container on exactly the
    VM's policy when that list changes.
    """
    redirects = []
    for line in output_rules.splitlines():
        match = REDIRECT_RULE.match(line.strip())
        if match:
            destination, protocol, port, proxy = match.groups()
            redirects.append(
                (protocol, int(port) if port else None, int(proxy), destination)
            )
    return redirects


def derive_returns(output_rules):
    """Destinations the VM exempts from interception: its private networks,
    whose traffic leaves through their cables rather than a proxy."""
    return [
        match.group(1)
        for match in map(RETURN_RULE.match, (line.strip() for line in output_rules.splitlines()))
        if match
    ]


def open_cables(run, sysctl_root):
    """Let the container use the VM's cables: every protocol to a member
    leaves through the cable that routes it, as that cable's address, and
    what a cable delivers to one of the VM's own addresses is the container's. MASQUERADE takes the address
    when a packet leaves, so a cable plugged after the container started
    carries it as well. Forwarding is for the container alone: a VM on two
    networks never carries one network's packets onto the other.
    """
    run(
        IPTABLES,
        "-t",
        "nat",
        "-A",
        "POSTROUTING",
        "-o",
        CABLES,
        "-s",
        CONTAINER_ADDRESS,
        "-j",
        "MASQUERADE",
    )
    run(
        IPTABLES,
        "-t",
        "nat",
        "-A",
        NAT_CHAIN,
        "-i",
        CABLES,
        "-m",
        "addrtype",
        "--dst-type",
        "LOCAL",
        "-j",
        "DNAT",
        "--to-destination",
        CONTAINER_ADDRESS,
    )
    # Inserted after the container's FORWARD drop, so they sit above it.
    run(IPTABLES, "-I", "FORWARD", "-i", CABLES, "-o", HOST_LINK, "-j", "ACCEPT")
    run(IPTABLES, "-I", "FORWARD", "-i", HOST_LINK, "-o", CABLES, "-j", "ACCEPT")
    # Inserted last, so it sits first: no accept can ever join two networks.
    run(IPTABLES, "-I", "FORWARD", "-i", CABLES, "-o", CABLES, "-j", "DROP")
    (sysctl_root / "net/ipv4/ip_forward").write_text("1\n")


def network_ready(pid, run=command, sysctl_root=Path("/proc/sys")):
    """Prestart hook: give the container a gateway that leads only to the VM's
    interception proxies and, when the VM has one, its private link.

    Runs in the VM's network namespace with the container's pid. Traffic from
    the container's end of the veth is DNAT'ed to the loopback proxies the VM
    already uses; everything else that reaches the VM from that interface is
    dropped, so the container cannot talk to the VM's other listeners or its
    dummy address. Members of the VM's networks are reached over the VM's
    cables, every protocol, never through a proxy (`open_cables`).
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
    run(
        *namespace,
        "ip",
        "addr",
        "add",
        f"{CONTAINER_ADDRESS}/30",
        "dev",
        CONTAINER_LINK,
    )
    run(*namespace, "ip", "link", "set", CONTAINER_LINK, "up")
    run(*namespace, "ip", "route", "add", "default", "via", GATEWAY)

    rules = run(
        IPTABLES, "-t", "nat", "-S", "OUTPUT", capture_output=True, text=True
    ).stdout
    redirects = derive_redirects(rules)
    if not redirects:
        raise ValueError("VM has no interception rules for the container to mirror")
    for table, chain in (("nat", NAT_CHAIN), ("filter", INPUT_CHAIN)):
        table_args = ["-t", table] if table == "nat" else []
        run(IPTABLES, *table_args, "-N", chain, check=False)
        run(IPTABLES, *table_args, "-F", chain)
    # Member traffic returns before any proxy DNAT can claim it, as in the VM.
    for destination in derive_returns(rules):
        run(IPTABLES, "-t", "nat", "-A", NAT_CHAIN, "-i", HOST_LINK, "-d", destination, "-j", "RETURN")
    for protocol, port, proxy, destination in redirects:
        match_args = ["-d", destination] if destination else []
        if port is not None:
            match_args += ["--dport", str(port)]
        run(
            IPTABLES,
            "-t",
            "nat",
            "-A",
            NAT_CHAIN,
            "-i",
            HOST_LINK,
            "-p",
            protocol,
            *match_args,
            "-j",
            "DNAT",
            "--to-destination",
            f"127.0.0.1:{proxy}",
        )
        run(
            IPTABLES,
            "-A",
            INPUT_CHAIN,
            "-i",
            HOST_LINK,
            "-d",
            "127.0.0.1",
            "-p",
            protocol,
            "--dport",
            str(proxy),
            "-j",
            "ACCEPT",
        )
    run(IPTABLES, "-A", INPUT_CHAIN, "-i", HOST_LINK, "-j", "DROP")
    for table_args, parent, target in (
        (["-t", "nat"], "PREROUTING", ["-j", NAT_CHAIN]),
        ([], "INPUT", ["-j", INPUT_CHAIN]),
        ([], "FORWARD", ["-i", HOST_LINK, "-j", "DROP"]),
    ):
        if (
            run(IPTABLES, *table_args, "-C", parent, *target, check=False).returncode
            != 0
        ):
            run(IPTABLES, *table_args, "-I", parent, *target)
    open_cables(run, sysctl_root)


def staged_parts(stage, entry):
    """One staged file's bytes, part by part, verified against its digest
    once the last part is read."""
    digest = hashlib.sha256()
    for number in range(entry["parts"]):
        data = (stage / f"{entry['key']}-{number}").read_bytes()
        digest.update(data)
        yield data
    if digest.hexdigest() != entry["sha256"]:
        raise ValueError("OCI upload digest mismatch")


def assemble(stage, layout):
    """Reassemble bounded uploads and verify each file before umoci sees it."""
    transfer = json.loads((stage / "transfer.json").read_text())
    for entry in transfer:
        relative = PurePosixPath(entry["path"])
        if relative.is_absolute() or ".." in relative.parts:
            raise ValueError("unsafe OCI transfer path")
        destination = layout / relative
        destination.parent.mkdir(parents=True, exist_ok=True)
        with destination.open("xb") as output:
            for data in staged_parts(stage, entry):
                output.write(data)


def staged_manifest_digest(stage):
    """The manifest the staged layout's index.json names: the identity of
    what will be unpacked. Reads and verifies index.json alone."""
    transfer = json.loads((stage / "transfer.json").read_text())
    (entry,) = [entry for entry in transfer if entry["path"] == "index.json"]
    index = json.loads(b"".join(staged_parts(stage, entry)))
    digest = index["manifests"][0]["digest"]
    if not (isinstance(digest, str) and DIGEST.match(digest)):
        raise ValueError(f"staged index names no valid manifest digest: {digest!r}")
    return digest


def unpacked_root(stage, id_map):
    """The image's unpacked root for this launch: (bundle, image config,
    umoci's runtime config). Unpacked once per manifest digest and kept on
    the VM's overlay; a relaunch of a named session reuses it. Only the
    current digest's root is kept."""
    digest = staged_manifest_digest(stage)
    ROOTS.mkdir(parents=True, exist_ok=True, mode=0o711)
    root = ROOTS / digest.removeprefix("sha256:")
    for other in ROOTS.iterdir():
        if other != root:
            shutil.rmtree(other, ignore_errors=True)
    if not (root / "ready").is_file():
        shutil.rmtree(root, ignore_errors=True)
        root.mkdir(mode=0o711)
        layout = RUNTIME / "image"
        layout.mkdir()
        assemble(stage, layout)
        manifest = json.loads((layout / "blobs/sha256" / digest.split(":")[1]).read_text())
        image_config = (layout / "blobs/sha256" / manifest["config"]["digest"].split(":")[1]).read_text()
        mapping = f"{id_map['containerID']}:{id_map['hostID']}:{id_map['size']}"
        # Owned by the mapped ids, so the root filesystem is container root's.
        command(
            "umoci", "unpack", "--uid-map", mapping, "--gid-map", mapping,
            "--image", f"{layout}:image", str(root / "bundle"),
        )
        (root / "bundle").chmod(0o711)
        shutil.copyfile(root / "bundle" / "config.json", root / "runtime.json")
        (root / "image.json").write_text(image_config)
        shutil.rmtree(layout)
        (root / "ready").write_text(digest + "\n")
    return (
        root / "bundle",
        json.loads((root / "image.json").read_text()),
        json.loads((root / "runtime.json").read_text()),
    )


def run(stage):
    # A second workload cannot overwrite live state. Traversable, not listable:
    # runc sets the container up as the mapped root, which must reach the
    # bundle and the resolver file through here.
    RUNTIME.mkdir(mode=0o711)
    RUNTIME.chmod(0o711)
    state = RUNTIME / "state"
    process = None
    try:
        options = json.loads((stage / "options.json").read_text())
        id_map = checked_id_map(options.get("id_map"))
        bundle, image, unpacked = unpacked_root(stage, id_map)
        if options.get("workspace") is not None:
            WORKSPACE_VIEW.mkdir(mode=0o700, exist_ok=True)
            idmap_workspace(id_map, WORKSPACE_VIEW)
        config_path = bundle / "config.json"
        config = configure(unpacked, image, options)
        # configure() has refused any unsafe volume path by now.
        prepare_volumes(
            (image.get("config") or {}).get("Volumes") or {},
            bundle / "rootfs",
            checked_id_map(options["id_map"]),
        )
        config_path.write_text(json.dumps(config))
        (RUNTIME / "resolv.conf").write_text(resolv_conf())
        (stage / "ready").write_text("1\n")
        pid_file = RUNTIME / "workload.pid"
        process = subprocess.Popen(
            [
                *RUNC,
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
            command(*RUNC, "delete", "--force", CONTAINER)
        if process is not None and process.poll() is None:
            process.wait(timeout=5)
        shutil.rmtree(RUNTIME)


def exec_request(encoded):
    """(argv, tty) from the host's hex-encoded JSON request (capsem-core
    `workload_exec_command`): `command` runs under the image's /bin/sh, `argv`
    runs as given."""
    request = json.loads(bytes.fromhex(encoded))
    if not isinstance(request, dict) or set(request) - {"argv", "command", "tty"}:
        raise ValueError(f"refusing exec request {request!r}")
    if ("argv" in request) == ("command" in request):
        raise ValueError("an exec request names exactly one of argv and command")
    argv = request["argv"] if "argv" in request else ["/bin/sh", "-c", request["command"]]
    tty = request.get("tty", False)
    if (
        not isinstance(argv, list)
        or not argv
        or not all(isinstance(arg, str) and "\0" not in arg for arg in argv)
        or not argv[0]
        or type(tty) is not bool
    ):
        raise ValueError(f"refusing exec request {request!r}")
    return argv, tty


def exec_process(config, argv, tty):
    """The workload's own process with another command: the image's user, cwd
    and env, and the capabilities, rlimits and no-new-privileges `configure`
    gave it. runc exec joins its namespaces, cgroup and seccomp filter."""
    process = json.loads(json.dumps(config["process"]))
    process.update(args=argv, terminal=tty)
    return process


def exec_argv(process_path):
    """runc exec of the process spec at `process_path` into the workload."""
    return [*RUNC, "exec", "--process", process_path, CONTAINER]


def workload_bundle(run=command):
    """The bundle the workload this VM launched runs from, or None when it is
    not running. runc's own state names it: the bundle lives under the digest's
    unpacked root, and only runc knows which one this workload started from."""
    if not (RUNTIME / "state" / CONTAINER).exists():
        return None
    state = run(*RUNC, "state", CONTAINER, check=False, capture_output=True, text=True)
    if state.returncode != 0:
        return None
    state = json.loads(state.stdout)
    return Path(state["bundle"]) if state.get("status") == "running" else None


def exec_workload(encoded):
    """Replace this launcher with `runc exec` of the request in the workload.

    The process spec travels in a memfd, so nothing is written to disk and
    nothing is left to clean up; runc reads it through /proc/self/fd.
    """
    argv, tty = exec_request(encoded)
    bundle = workload_bundle()
    if bundle is None:
        raise SystemExit("capsem: no container workload is running in this session")
    config = json.loads((bundle / "config.json").read_text())
    spec = os.memfd_create("capsem-exec", 0)
    os.write(spec, json.dumps(exec_process(config, argv, tty)).encode())
    os.set_inheritable(spec, True)
    runc = exec_argv(f"/proc/self/fd/{spec}")
    os.execvp(runc[0], runc)


if __name__ == "__main__":
    if sys.argv[1:] == ["--network-ready"]:
        pid = int(json.load(sys.stdin)["pid"])
        if pid <= 1:
            raise ValueError("invalid container network namespace pid")
        network_ready(pid)
    elif len(sys.argv) == 3 and sys.argv[1] == "--exec":
        exec_workload(sys.argv[2])
    else:
        sys.exit(run(Path(sys.argv[1])))
