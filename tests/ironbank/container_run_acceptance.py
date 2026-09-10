"""Real CLI, hermetic registry, rebuilt VM, default Redis startup and teardown.

Explicit ARM64 spike proof: requires the pinned Redis prefetch.
Run by path; this is not a portable release qualification gate.
"""

import contextlib
import json
import os
import signal
import subprocess
import time

import pytest
from helpers.constants import BIN_DIR, CODE_PROFILE_ID
from helpers.service import ServiceInstance, vm_session_dir

from tests.fixtures.oci.registry import registry

pytestmark = pytest.mark.integration


def wait_for(predicate, description, timeout=90):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if predicate():
            return
        time.sleep(0.05)
    raise AssertionError(f"timed out: {description}")


@pytest.fixture
def service(tmp_path):
    instance = ServiceInstance()
    try:
        instance.start()
        yield instance
    finally:
        instance.stop(cleanup=False)
        (tmp_path / "service.log").write_bytes(
            (instance.tmp_dir / "service.log").read_bytes()
        )
        for log in instance.home_dir.rglob("*.log*"):
            if log.is_file() and log.stat().st_size < 2 * 1024 * 1024:
                target = tmp_path / "host-logs" / log.relative_to(instance.home_dir)
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_bytes(log.read_bytes())
        instance.stop()


def command(service, reference, certificate=None, *args):
    result = [
        str(BIN_DIR / "capsem"),
        "--uds-path",
        str(service.uds_path),
        "run",
        "--profile",
        CODE_PROFILE_ID,
    ]
    if certificate is not None:
        result += ["--registry-ca", str(certificate)]
    return [*result, *args, reference]


def environment(service):
    return {
        **os.environ,
        "CAPSEM_HOME": str(service.home_dir),
        "CAPSEM_RUN_DIR": str(service.tmp_dir),
        "CAPSEM_PROFILES_DIR": str(service.profiles_dir),
    }


def test_cli_default_redis_stream_and_cancel(service, tmp_path):
    client = service.client()
    with registry(tmp_path) as (reference, certificate, requests):
        rejected = subprocess.run(
            command(service, reference),
            env=environment(service),
            capture_output=True,
            timeout=30,
            check=False,
        )
        assert rejected.returncode != 0
        assert b"Pulling " in rejected.stderr and not requests
        assert client.get("/vms/list")["sandboxes"] == []
        with (
            (tmp_path / "stdout").open("wb") as stdout,
            (tmp_path / "stderr").open("wb") as stderr,
            contextlib.ExitStack() as retained_logs,
        ):
            process_logs = {}
            process = subprocess.Popen(
                command(service, reference, certificate),
                env=environment(service),
                stdout=stdout,
                stderr=stderr,
            )
            try:

                def ready():
                    for log in service.tmp_dir.glob("persistent/*/process.log"):
                        if log not in process_logs:
                            process_logs[log] = retained_logs.enter_context(
                                log.open("rb")
                            )
                    assert process.poll() is None, (tmp_path / "stderr").read_text() + (
                        tmp_path / "stdout"
                    ).read_text()
                    return (
                        b"Ready to accept connections tcp"
                        in (tmp_path / "stdout").read_bytes()
                    )

                wait_for(ready, "attached Redis logs")
                rows = client.get("/vms/list")["sandboxes"]
                assert len(rows) == 1 and rows[0]["name"] == "redis"
                session = vm_session_dir(service.tmp_dir, client, rows[0]["id"])
                proof = client.post(
                    f"/vms/{rows[0]['id']}/exec",
                    {
                        "command": "set -eu; pid=$(cat /var/tmp/capsem-container/workload.pid); "
                        "nsenter -t $pid -n /bin/bash -c 'exec 3<>/dev/tcp/127.0.0.1/6379; printf \"PING\\r\\n\" >&3; head -c 7 <&3'",
                        "timeout_secs": 10,
                    },
                )
                assert proof["exit_code"] == 0 and proof["stdout"] == "+PONG\r\n", proof
                (tmp_path / "proof.json").write_text(
                    json.dumps(
                        {"reference": reference, "vm": rows[0], "ping": proof}, indent=2
                    )
                )
                process.send_signal(signal.SIGINT)
                assert process.wait(timeout=30) == 130
                assert client.get("/vms/list")["sandboxes"] == []
                assert not session.exists(), (
                    "cancelled container VM retained its workspace/runtime"
                )
                assert len([path for path in requests if "/blobs/" in path]) == 2
            finally:
                if process.poll() is None:
                    process.terminate()
                    with contextlib.suppress(subprocess.TimeoutExpired):
                        process.wait(timeout=15)
                    if process.poll() is None:
                        process.kill()
                        process.wait(timeout=5)
                for log in process_logs.values():
                    log.seek(0)
                    (tmp_path / "process.log").write_bytes(log.read())


@pytest.mark.parametrize(
    ("args", "options", "code", "output"),
    [
        (
            ["/bin/sh", "-c", "printf '\\000\\377hello'; exit 7"],
            [],
            7,
            b"\x00\xffhello",
        ),
        (["/bin/sh", "-c", "sleep 120 & wait"], ["--timeout", "5"], 124, b""),
        (["/missing-command"], [], 127, None),
    ],
)
def test_cli_exit_timeout_and_failed_launch(
    service, tmp_path, args, options, code, output
):
    client = service.client()
    with registry(tmp_path) as (reference, certificate, requests):
        for attempt in range(2):
            result = subprocess.run(
                [*command(service, reference, certificate, *options), *args],
                env=environment(service),
                capture_output=True,
                timeout=45,
                check=False,
            )
            (tmp_path / f"attempt-{attempt}.stderr").write_bytes(result.stderr)
            assert result.returncode == code, result.stderr + result.stdout
            if output is not None:
                assert result.stdout == output, result.stdout
            assert client.get("/vms/list")["sandboxes"] == []
            assert not list(service.tmp_dir.glob("persistent/*/guest"))
        assert len([path for path in requests if "/blobs/" in path]) == 2, requests


def test_cli_runtime_failure_removes_vm(service, tmp_path):
    metadata = {"Entrypoint": ["/missing-entrypoint"], "Cmd": []}
    with registry(tmp_path, image_config=metadata) as (reference, certificate, _):
        result = subprocess.run(
            command(service, reference, certificate),
            env=environment(service),
            capture_output=True,
            timeout=45,
            check=False,
        )
        assert result.returncode == 1, result.stderr + result.stdout
        assert b"no such file" in result.stdout + result.stderr
        assert service.client().get("/vms/list")["sandboxes"] == []
        assert not list(service.tmp_dir.glob("persistent/*/guest"))


def test_cli_stream_exceeds_captured_exec_limit(service, tmp_path):
    with registry(tmp_path) as (reference, certificate, _):
        result = subprocess.run(
            [
                *command(service, reference, certificate),
                "/bin/sh",
                "-c",
                "dd if=/dev/zero bs=1048576 count=12 2>/dev/null; exit 9",
            ],
            env=environment(service),
            capture_output=True,
            timeout=45,
            check=False,
        )
        assert result.returncode == 9, result.stderr
        assert len(result.stdout) == 12 * 1024**2
        assert not any(result.stdout)
        assert service.client().get("/vms/list")["sandboxes"] == []


def test_shell_run_still_uses_existing_command_path(service):
    result = subprocess.run(
        command(service, "printf shell-proof; exit 3"),
        env=environment(service),
        capture_output=True,
        timeout=45,
        check=False,
    )
    assert result.returncode == 3 and result.stdout == b"shell-proof", result.stderr
    assert service.client().get("/vms/list")["sandboxes"] == []
