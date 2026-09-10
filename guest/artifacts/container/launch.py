"""Run one verified OCI layout in a disposable guest. Invoked by the host CLI."""

import hashlib
import json
import shutil
import subprocess
import sys
import time
from pathlib import Path, PurePosixPath

RUNTIME = Path("/var/tmp/capsem-container")
CONTAINER = "workload"


def configure(unpacked, image, options):
    process = unpacked["process"]
    metadata = image.get("config") or {}
    if options["args"]:
        process["args"] = (metadata.get("Entrypoint") or []) + options["args"]
    if not process.get("args") or not process["args"][0]:
        raise ValueError("image has no command")
    environment = dict(entry.split("=", 1) for entry in process.get("env", []))
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
    return {
        "ociVersion": "1.0.2",
        "root": {"path": "rootfs", "readonly": True},
        "hostname": "container",
        "process": process,
        "mounts": mounts,
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


def command(*args, **kwargs):
    return subprocess.run(args, check=True, timeout=30, **kwargs)


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
                part.unlink()
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
        deadline = time.monotonic() + 15
        while not pid_file.exists() and process.poll() is None:
            if time.monotonic() >= deadline:
                raise TimeoutError("container did not start")
            time.sleep(0.01)
        if process.poll() is None:
            pid = int(pid_file.read_text())
            command("nsenter", "-t", str(pid), "-n", "ip", "link", "set", "lo", "up")
        return process.wait()
    finally:
        if (state / CONTAINER).exists():
            command(*runc, "delete", "--force", CONTAINER)
        if process is not None and process.poll() is None:
            process.wait(timeout=5)
        shutil.rmtree(RUNTIME)


if __name__ == "__main__":
    sys.exit(run(Path(sys.argv[1])))
