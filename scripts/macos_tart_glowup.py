#!/usr/bin/env python3
"""Install and exercise an exact Capsem package in a disposable Tart Mac."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shlex
import shutil
import subprocess
import sys
import time
from typing import Callable, Sequence

try:
    from release_glowup import (
        ArtifactIdentity,
        GlowupContractError,
        assert_manifest_artifact,
        build_report,
        load_manifest_bytes,
        validate_installed_evidence,
    )
except ModuleNotFoundError:
    from scripts.release_glowup import (
        ArtifactIdentity,
        GlowupContractError,
        assert_manifest_artifact,
        build_report,
        load_manifest_bytes,
        validate_installed_evidence,
    )


PROJECT_ROOT = Path(__file__).resolve().parent.parent
STORAGE_POLICY = PROJECT_ROOT / "config" / "storage-policy.toml"
STORAGE_CONTROLLER = PROJECT_ROOT / "scripts" / "docker-storage-policy.py"


def storage_policy_string(section: str, key: str) -> str:
    text = STORAGE_POLICY.read_text()
    section_match = re.search(
        rf"(?ms)^\[{re.escape(section)}\]\s*(.*?)(?=^\[|\Z)",
        text,
    )
    if section_match is None:
        raise RuntimeError(f"storage policy is missing [{section}]")
    value_match = re.search(
        rf'(?m)^{re.escape(key)}\s*=\s*"([^"]+)"\s*$',
        section_match.group(1),
    )
    if value_match is None:
        raise RuntimeError(f"storage policy [{section}] is missing {key}")
    return value_match.group(1)


OWNED_VM_PREFIX = storage_policy_string("tart", "owned_vm_prefix")
DEFAULT_IMAGE = storage_policy_string("tart", "base_image")
Run = Callable[..., subprocess.CompletedProcess[str]]


def tart_clone_command(image: str, vm_name: str) -> list[str]:
    return ["tart", "clone", image, vm_name]


def tart_run_command(
    vm_name: str,
    share: Path,
    asset_share: Path | None = None,
    profile_share: Path | None = None,
) -> list[str]:
    directories = [f"--dir=capsem-release:{share}"]
    if asset_share is not None:
        directories.append(f"--dir=capsem-assets:{asset_share}")
    if profile_share is not None:
        directories.append(f"--dir=capsem-profiles:{profile_share}")
    return [
        "tart",
        "run",
        "--no-graphics",
        *directories,
        vm_name,
    ]


def tart_ip_command(vm_name: str, wait_seconds: int = 300) -> list[str]:
    return ["tart", "ip", vm_name, "--wait", str(wait_seconds)]


def storage_control_command(command: str, label: str) -> list[str]:
    return [
        "uv",
        "run",
        "python",
        str(STORAGE_CONTROLLER),
        command,
        "--label",
        label,
    ]


def run_storage_control(
    command: str,
    label: str,
    *,
    strict: bool = True,
) -> subprocess.CompletedProcess[str]:
    control = storage_control_command(command, label)
    print("+", shlex.join(control), flush=True)
    result = subprocess.run(control, check=False, text=True)
    if strict and result.returncode != 0:
        raise RuntimeError(
            f"Tart storage controller failed at {label} (status {result.returncode})"
        )
    return result


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
    destination.parent.mkdir(parents=True, exist_ok=True)
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


def validate_report(
    report_path: Path,
    *,
    artifact: ArtifactIdentity,
) -> dict[str, object]:
    if not report_path.is_file():
        raise RuntimeError(f"Tart guest did not write its report: {report_path}")
    report = json.loads(report_path.read_text())
    if report.get("schema") != "capsem.release_glowup.guest.v1":
        raise RuntimeError("Tart guest wrote an unsupported glow-up evidence schema")
    if report.get("artifact_sha256") != artifact.sha256:
        raise RuntimeError("Tart guest package SHA does not match the host candidate")
    installed = report.get("installed")
    if not isinstance(installed, dict):
        raise RuntimeError("Tart guest report has no normalized installed evidence")
    validate_installed_evidence(installed)
    return report


def local_tart_capabilities() -> dict[str, bool]:
    """Describe only the evidence produced by the unsigned local Tart rail."""

    return {
        "native_install": True,
        "package_receipt": True,
        "launchd": True,
        "physical_vz_boot": False,
        "signed": False,
        "gatekeeper": False,
    }


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


def capture_guest_diagnostics(ip: str, work_dir: Path) -> None:
    """Persist bounded installer evidence before the failed VM is destroyed."""

    remote_script = r"""
set +e
echo '=== /var/log/install.log (tail) ==='
sudo tail -n 300 /var/log/install.log
echo '=== ~/.capsem/logs/install.log ==='
cat "$HOME/.capsem/logs/install.log"
echo '=== ~/.capsem/logs/install-failure.txt ==='
cat "$HOME/.capsem/logs/install-failure.txt"
echo '=== ~/.capsem/logs listing ==='
ls -la "$HOME/.capsem/logs"
echo '=== package script log events ==='
sudo log show --last 15m --style compact \
  --predicate 'process == "installer" OR process == "package_script_service"' \
  | tail -n 400
"""
    command = ssh_command(ip, [shlex.join(["bash", "-lc", remote_script])])
    try:
        result = subprocess.run(
            command,
            check=False,
            capture_output=True,
            text=True,
            timeout=120,
        )
        contents = (
            f"diagnostic_status={result.returncode}\n{result.stdout}\n{result.stderr}"
        )
    except (OSError, subprocess.SubprocessError) as error:
        contents = f"diagnostic_capture_failed={error}\n"
    (work_dir / "guest-diagnostics.log").write_text(contents, encoding="utf-8")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--package", required=True, type=Path)
    parser.add_argument("--version", required=True)
    parser.add_argument("--manifest-url", required=True)
    parser.add_argument("--manifest-file", required=True, type=Path)
    parser.add_argument("--sbom", required=True, type=Path)
    parser.add_argument("--asset-share", required=True, type=Path)
    parser.add_argument("--profile-share", required=True, type=Path)
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
    run_storage_control("tart-clean", "macos-glowup-preflight")
    package = args.package.resolve()
    if not package.is_file() or package.stat().st_size == 0:
        raise RuntimeError(f"package is missing or empty: {package}")
    artifact = ArtifactIdentity.from_path(
        package,
        version=args.version,
        platform="macos",
        architecture="arm64",
    )
    manifest_file = args.manifest_file.resolve()
    asset_share = args.asset_share.resolve()
    if not asset_share.is_dir():
        raise RuntimeError(f"candidate asset share is missing: {asset_share}")
    profile_share = args.profile_share.resolve()
    if not profile_share.is_dir():
        raise RuntimeError(f"candidate profile share is missing: {profile_share}")
    manifest = load_manifest_bytes(manifest_file.read_bytes())
    assert_manifest_artifact(manifest, artifact)

    work_dir = args.work_dir.resolve()
    share = work_dir / "share"
    if share.exists():
        shutil.rmtree(share)
    share.mkdir(parents=True, exist_ok=True)
    (work_dir / "report.json").unlink(missing_ok=True)
    (work_dir / "guest-diagnostics.log").unlink(missing_ok=True)
    report_path = share / "report.json"
    report_path.unlink(missing_ok=True)
    candidate_dir = share / "candidate"
    if candidate_dir.exists():
        shutil.rmtree(candidate_dir)
    release_dir = (
        candidate_dir
        / "releases"
        / "download"
        / args.channel
        / f"v{args.version}"
    )
    guest_package = release_dir / package.name
    stage_file(package, guest_package)
    stage_file(args.sbom.resolve(), release_dir / "capsem-sbom.spdx.json")
    stage_file(
        manifest_file,
        candidate_dir / "assets" / args.channel / "manifest.json",
    )
    stage_file(PROJECT_ROOT / "scripts" / "macos_tart_guest.sh", share / "guest.sh")
    stage_file(
        PROJECT_ROOT / "scripts" / "verify-installed-release.py",
        share / "verify-installed-release.py",
    )
    stage_file(
        PROJECT_ROOT / "scripts" / "macos-install-user-request.sh",
        share / "macos-install-user-request.sh",
    )
    stage_file(
        PROJECT_ROOT / "scripts" / "release_glowup.py",
        share / "release_glowup.py",
    )

    vm_name = f"{OWNED_VM_PREFIX}{os.getpid()}-{int(time.time())}"
    require_owned_vm(vm_name)
    runner: subprocess.Popen[str] | None = None
    log_stream = None
    ip: str | None = None
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
        command = tart_run_command(vm_name, share, asset_share, profile_share)
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
                f"/Volumes/My Shared Files/capsem-release/{guest_package.relative_to(share)}",
            ]
        )
        run_checked(ssh_command(ip, [remote]), timeout=1800)
        guest_report = validate_report(report_path, artifact=artifact)
        report = build_report(
            adapter="macos-tart-launchd",
            artifact=artifact,
            installed=guest_report["installed"],
            capabilities=local_tart_capabilities(),
        )
        report["adapter_evidence"] = {
            "tart_image": args.image,
            "tart_vm": vm_name,
            "guest": guest_report.get("guest", {}),
        }
        rendered_report = json.dumps(report, indent=2, sort_keys=True) + "\n"
        report_path.write_text(rendered_report)
        final_report_path = work_dir / "report.json"
        final_report_path.write_text(rendered_report)
        print(f"Tart macOS installed-package proof passed: {final_report_path}")
        return 0
    finally:
        primary_error = sys.exc_info()[0] is not None
        if primary_error and ip is not None:
            capture_guest_diagnostics(ip, work_dir)
        cleanup_vm(vm_name)
        terminate_runner(runner, log_stream)
        final_control = run_storage_control(
            "tart-clean",
            "macos-glowup-final",
            strict=False,
        )
        if final_control.returncode != 0 and not primary_error:
            raise RuntimeError("Tart storage controller found leaked or running glow-up VMs")


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (GlowupContractError, OSError, RuntimeError, subprocess.SubprocessError) as error:
        print(f"Tart macOS installed-package proof failed: {error}", file=sys.stderr)
        raise SystemExit(1)
