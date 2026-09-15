"""Real Redis service proof inside the guest; no host or external networking."""

import contextlib
import hashlib
import json
import shutil
import socket
from pathlib import Path

from bundle import Bundle, command, wait_for

ARGV = [
    "/usr/local/bin/redis-server",
    "--bind",
    "127.0.0.1",
    "--port",
    "6379",
    "--dir",
    "/scratch",
    "--save",
    "",
    "--appendonly",
    "no",
    "--maxmemory",
    "8mb",
    "--maxmemory-policy",
    "noeviction",
]


def cli(bundle, *args):
    result = bundle.runc("exec", bundle.id, "/usr/local/bin/redis-cli", "--raw", *args)
    assert result.returncode == 0, (result.stdout, result.stderr)
    return result.stdout.strip()


@contextlib.contextmanager
def server(bundle):
    with bundle.running(ARGV) as process:

        def running():
            if process.poll() is not None:
                raise AssertionError(bundle.output())
            state = bundle.runc("state", bundle.id)
            return (
                state.returncode == 0
                and json.loads(state.stdout)["status"] == "running"
            )

        wait_for(running, "Redis container running")
        pid = json.loads(bundle.runc("state", bundle.id).stdout)["pid"]
        # Only bring up the container's own loopback; no veth, route or proxy.
        result = command(
            "nsenter", "-t", str(pid), "-n", "ip", "link", "set", "lo", "up"
        )
        assert result.returncode == 0, result.stderr
        wait_for(
            lambda: "Ready to accept connections tcp" in bundle.output()[0],
            "Redis ready",
        )
        assert cli(bundle, "PING") == "PONG"
        network = str(Path(f"/proc/{pid}/ns/net").readlink())
        assert network != str(Path("/proc/self/ns/net").readlink())
        root_check = bundle.runc(
            "exec",
            bundle.id,
            "/bin/sh",
            "-ec",
            "test -f /etc/alpine-release; test -d /scratch; "
            "test ! -e /newroot; test ! -e /capsem-pty-agent; "
            'test "$(id -u)" = 65534; readlink /proc/self/ns/net',
        )
        assert root_check.returncode == 0, root_check.stderr
        assert root_check.stdout.strip() == network
        yield process


def main():
    stage = Path("/root/oci")
    marker = Path("/var/tmp/redis-session-proof.rdb")
    assert not marker.exists(), "fresh VM can see the previous Redis RDB"
    pin = json.loads((stage / "redis-image.json").read_text())
    archive = stage / "redis-rootfs.tar.gz"
    with archive.open("wb") as output:
        for part in sorted(stage.glob("redis-rootfs.part*")):
            output.write(part.read_bytes())
    assert hashlib.sha256(archive.read_bytes()).hexdigest() == pin["archive_sha256"]
    evidence = {"image": pin["image"], "archive_sha256": pin["archive_sha256"]}
    bundle = Bundle(rootfs_archive=archive)
    try:
        executable = bundle.rootfs / "usr/local/bin/redis-server"
        evidence["redis_server_sha256"] = hashlib.sha256(
            executable.read_bytes()
        ).hexdigest()
        with server(bundle) as process:
            assert cli(bundle, "DBSIZE") == "0", "fresh VM inherited Redis data"
            assert f"redis_version:{pin['version']}" in cli(bundle, "INFO", "server")
            assert cli(bundle, "SET", "capsem-proof", "redis-in-a-real-vm") == "OK"
            assert cli(bundle, "GET", "capsem-proof") == "redis-in-a-real-vm"
            assert cli(bundle, "SAVE") == "OK"
            assert (bundle.scratch / "dump.rdb").is_file()
            shutil.copyfile(bundle.scratch / "dump.rdb", marker)
            evidence["rdb_sha256"] = hashlib.sha256(marker.read_bytes()).hexdigest()
            # Exiting PID 1 kills remaining namespace processes, including a
            # concurrent redis-cli SHUTDOWN; signal the server from outside.
            stopped = bundle.runc("kill", bundle.id, "TERM")
            assert stopped.returncode == 0, stopped.stderr
            assert process.wait(timeout=15) == 0, bundle.output()
        bundle.assert_clean()
        with server(bundle) as process:
            assert cli(bundle, "GET", "capsem-proof") == "redis-in-a-real-vm"
            evidence["persistence"] = "saved RDB reloaded after container restart"
            # Application limit must reject writes while the server stays responsive.
            # Redis checks maxmemory before a command; a single command can
            # overshoot. Bound the stimulus to eight 2 MiB writes.
            for index in range(8):
                reply = cli(bundle, "SETRANGE", f"pressure-{index}", "2097151", "x")
                if reply.startswith("OOM"):
                    break
                assert reply == "2097152", reply
            else:
                raise AssertionError("Redis maxmemory never rejected writes")
            assert cli(bundle, "PING") == "PONG"
            assert cli(bundle, "GET", "capsem-proof") == "redis-in-a-real-vm"
            assert int((bundle.cgroup / "memory.max").read_text()) == 67108864
            assert int((bundle.cgroup / "memory.current").read_text()) <= 67108864
            evidence["memory_limit"] = reply
            with socket.socket() as outside:
                outside.settimeout(1)
                assert outside.connect_ex(("127.0.0.1", 6379)) != 0, (
                    "Redis exposed to guest"
                )
            killed = bundle.runc("kill", "--all", bundle.id, "KILL")
            assert killed.returncode == 0, killed.stderr
            assert process.wait(timeout=15) == 137, bundle.output()
        bundle.assert_clean()
        evidence["cleanup"] = "forced exit 137; no container, cgroup or mounts"
        with server(bundle) as process:
            assert cli(bundle, "GET", "capsem-proof") == "redis-in-a-real-vm"
            assert cli(bundle, "CONFIG", "SET", "maxmemory", "0") == "OK"
            oversized = bundle.runc(
                "exec",
                bundle.id,
                "/usr/local/bin/redis-cli",
                "--raw",
                "SETRANGE",
                "cgroup-pressure",
                "134217727",
                "x",
            )
            assert oversized.returncode != 0, oversized.stdout
            assert process.wait(timeout=15) == 137, bundle.output()
        kernel = command("dmesg")
        assert kernel.returncode == 0, kernel.stderr
        oom = [
            line
            for line in kernel.stdout.splitlines()
            if "oom-kill:" in line
            and f"oom_memcg=/{bundle.id}," in line
            and "task=redis-server" in line
        ]
        assert len(oom) == 1, kernel.stdout
        evidence["cgroup_oom"] = oom[0]
        evidence["scratch_path"] = str(bundle.scratch)
        (stage / "redis-evidence.json").write_text(json.dumps(evidence, indent=2))
        print(json.dumps(evidence, indent=2))
    finally:
        bundle.close()
    assert not bundle.directory.exists()


if __name__ == "__main__":
    main()
