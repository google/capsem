#!/usr/bin/env python3
"""Install and exercise an exact Capsem package in a disposable Tart Mac."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import shlex
import shutil
import subprocess
import sys
import time
from typing import Callable, Sequence


PROJECT_ROOT = Path(__file__).resolve().parent.parent
OWNED_VM_PREFIX = "capsem-glowup-"
DEFAULT_IMAGE = "ghcr.io/cirruslabs/macos-sequoia-base:latest"
Run = Callable[..., subprocess.CompletedProcess[str]]


def tart_clone_command(image: str, vm_name: str) -> list[str]:
    return ["tart", "clone", image, vm_name]


def tart_run_command(vm_name: str, share: Path) -> list[str]:
    return [
        "tart",
        "run",
        "--no-graphics",
        f"--dir=capsem-release:{share}",
        vm_name,
    ]


def tart_ip_command(vm_name: str, wait_seconds: int = 300) -> list[str]:
    return ["tart", "ip", vm_name, "--wait", str(wait_seconds)]


def ssh_command(ip: str, remote_args: Sequence[str]) -> list[str]:
    return [
        "sshpass",
        "-p",
        "admin",
        "ssh",
        "-o",
        "StrictHostKeyChecking=no",
        "-o",
        "UserKnownHostsFile=/dev/null",
        "-o",
        "ConnectTimeout=10",
        "-o",
        "IdentitiesOnly=yes",
        "-o",
        "PreferredAuthentications=password",
        "-o",
        "PubkeyAuthentication=no",
        f"admin@{ip}",
        *remote_args,
    ]


def require_owned_vm(vm_name: str) -> None:
    if not vm_name.startswith(OWNED_VM_PREFIX):
        raise ValueError(f"refusing non-owned VM name: {vm_name}")


def cleanup_vm(vm_name: str, *, run: Run = subprocess.run) -> None:
    require_owned_vm(vm_name)
    for command in (
        ["tart", "stop", vm_name],
        ["tart", "delete", vm_name],
    ):
        run(command, check=False, capture_output=True, text=True)


def run_checked(
    command: Sequence[str],
    *,
    timeout: int | None = None,
    capture_output: bool = False,
) -> subprocess.CompletedProcess[str]:
    print("+", shlex.join(str(part) for part in command), flush=True)
    return subprocess.run(
        command,
        check=True,
        text=True,
        timeout=timeout,
        capture_output=capture_output,
    )


def stage_file(source: Path, destination: Path) -> None:
    destination.unlink(missing_ok=True)
    # Do not hard-link or copy macOS provenance/resource-fork metadata into the
    # VirtioFS share. Tart guests can intermittently receive EACCES when Python
    # opens a hard-linked host file carrying com.apple.provenance.
    shutil.copyfile(source, destination)
    destination.chmod(source.stat().st_mode & 0o777)


def sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for chunk in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def wait_for_ssh(ip: str, timeout: int = 180) -> None:
    deadline = time.monotonic() + timeout
    last_error = ""
    while time.monotonic() < deadline:
        result = subprocess.run(
            ssh_command(ip, ["true"]),
            check=False,
            capture_output=True,
            text=True,
        )
        if result.returncode == 0:
            return
        last_error = (result.stderr or result.stdout).strip()
        time.sleep(2)
    raise RuntimeError(f"Tart guest SSH was not ready after {timeout}s: {last_error}")


def wait_for_guest_ip(
    vm_name: str,
    runner: subprocess.Popen[str],
    timeout: int = 300,
) -> str:
    deadline = time.monotonic() + timeout
    last_error = ""
    print("+", shlex.join(tart_ip_command(vm_name, 5)), flush=True)
    while time.monotonic() < deadline:
        returncode = runner.poll()
        if returncode is not None:
            raise RuntimeError(
                f"Tart VM runner exited before boot (status {returncode}); "
                "inspect target/macos-tart-glowup/tart-run.log"
            )
        result = subprocess.run(
            tart_ip_command(vm_name, 5),
            check=False,
            capture_output=True,
            text=True,
            timeout=15,
        )
        ip = result.stdout.strip()
        if result.returncode == 0 and ip:
            return ip
        last_error = (result.stderr or result.stdout).strip()
    raise RuntimeError(f"Tart guest IP was not available after {timeout}s: {last_error}")


def validate_host() -> None:
    if platform.system() != "Darwin":
        raise RuntimeError("Tart macOS install proof must run on macOS")
    if platform.machine() != "arm64":
        raise RuntimeError("Tart requires an Apple Silicon macOS host")
    for tool in ("tart", "sshpass"):
        if shutil.which(tool) is None:
            raise RuntimeError(
                f"{tool} is required; run bootstrap.sh or install Tart prerequisites"
            )


def validate_report(report_path: Path) -> dict[str, object]:
    if not report_path.is_file():
        raise RuntimeError(f"Tart guest did not write its report: {report_path}")
    report = json.loads(report_path.read_text())
    for field in (
        "package_receipt",
        "app_bundle",
        "binary_cohort",
        "installed_status",
    ):
        if report.get(field) is not True:
            raise RuntimeError(f"Tart guest report did not prove {field}")
    return report


def terminate_runner(
    runner: subprocess.Popen[str] | None,
    log_stream: object | None,
) -> None:
    if runner is not None:
        try:
            runner.wait(timeout=15)
        except subprocess.TimeoutExpired:
            runner.terminate()
            try:
                runner.wait(timeout=10)
            except subprocess.TimeoutExpired:
                runner.kill()
                runner.wait(timeout=10)
    if log_stream is not None:
        log_stream.close()  # type: ignore[union-attr]


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--package", required=True, type=Path)
    parser.add_argument("--version", required=True)
    parser.add_argument("--manifest-url", required=True)
    parser.add_argument("--channel", choices=("stable", "nightly"), required=True)
    parser.add_argument(
        "--image",
        default=os.environ.get("CAPSEM_TART_IMAGE", DEFAULT_IMAGE),
    )
    parser.add_argument(
        "--work-dir",
        type=Path,
        default=PROJECT_ROOT / "target" / "macos-tart-glowup",
    )
    args = parser.parse_args()

    validate_host()
    package = args.package.resolve()
    if not package.is_file() or package.stat().st_size == 0:
        raise RuntimeError(f"package is missing or empty: {package}")

    work_dir = args.work_dir.resolve()
    share = work_dir / "share"
    share.mkdir(parents=True, exist_ok=True)
    report_path = share / "report.json"
    report_path.unlink(missing_ok=True)
    stage_file(package, share / "Capsem.pkg")
    stage_file(PROJECT_ROOT / "scripts" / "macos_tart_guest.sh", share / "guest.sh")
    stage_file(
        PROJECT_ROOT / "scripts" / "verify-installed-release.py",
        share / "verify-installed-release.py",
    )
    stage_file(
        PROJECT_ROOT / "scripts" / "macos-install-user-request.sh",
        share / "macos-install-user-request.sh",
    )

    vm_name = f"{OWNED_VM_PREFIX}{os.getpid()}-{int(time.time())}"
    require_owned_vm(vm_name)
    runner: subprocess.Popen[str] | None = None
    log_stream = None
    try:
        run_checked(tart_clone_command(args.image, vm_name), timeout=3600)
        run_checked(
            [
                "tart",
                "set",
                vm_name,
                "--cpu",
                "4",
                "--memory",
                "8192",
                "--disk-size",
                "80",
            ]
        )
        tart_log = work_dir / "tart-run.log"
        log_stream = tart_log.open("w")
        command = tart_run_command(vm_name, share)
        print("+", shlex.join(command), flush=True)
        runner = subprocess.Popen(
            command,
            stdout=log_stream,
            stderr=subprocess.STDOUT,
            text=True,
        )
        ip = wait_for_guest_ip(vm_name, runner)
        wait_for_ssh(ip)
        remote = shlex.join(
            [
                "bash",
                "/Volumes/My Shared Files/capsem-release/guest.sh",
                args.version,
                args.manifest_url,
                args.channel,
            ]
        )
        run_checked(ssh_command(ip, [remote]), timeout=1800)
        report = validate_report(report_path)
        report.update(
            {
                "tart_image": args.image,
                "tart_vm": vm_name,
                "package": str(package),
                "package_sha256": sha256(package),
                "tested_commit": subprocess.run(
                    ["git", "rev-parse", "HEAD"],
                    cwd=PROJECT_ROOT,
                    check=True,
                    capture_output=True,
                    text=True,
                ).stdout.strip(),
            }
        )
        rendered_report = json.dumps(report, indent=2, sort_keys=True) + "\n"
        report_path.write_text(rendered_report)
        final_report_path = work_dir / "report.json"
        final_report_path.write_text(rendered_report)
        print(f"Tart macOS installed-package proof passed: {final_report_path}")
        return 0
    finally:
        cleanup_vm(vm_name)
        terminate_runner(runner, log_stream)


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (OSError, RuntimeError, subprocess.SubprocessError) as error:
        print(f"Tart macOS installed-package proof failed: {error}", file=sys.stderr)
        raise SystemExit(1)
