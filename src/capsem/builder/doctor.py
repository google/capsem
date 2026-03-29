"""Composable build prerequisite checks for capsem-builder.

Each check returns a CheckResult with pass/fail status, detail, and
optional fix instructions. Checks are pure functions that can be called
individually (from the build pipeline for fail-fast) or composed via
run_all_checks() for the doctor CLI command.
"""

from __future__ import annotations

import datetime
import shutil
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path


@dataclass
class CheckResult:
    """Result of a single prerequisite check."""

    name: str
    passed: bool
    detail: str
    fix: str | None = None

    def __str__(self) -> str:
        tag = "[PASS]" if self.passed else "[FAIL]"
        line = f"  {tag} {self.detail}"
        if not self.passed and self.fix:
            line += f"\n         fix: {self.fix}"
        return line


# ---------------------------------------------------------------------------
# Individual checks
# ---------------------------------------------------------------------------


def check_container_runtime() -> CheckResult:
    """Check for podman or docker on PATH (podman preferred)."""
    for name in ("podman", "docker"):
        path = shutil.which(name)
        if path:
            try:
                result = subprocess.run(
                    [name, "--version"],
                    capture_output=True, text=True, timeout=10,
                )
                version = result.stdout.strip()
                return CheckResult(
                    name="container-runtime",
                    passed=True,
                    detail=version,
                )
            except Exception:
                return CheckResult(
                    name="container-runtime",
                    passed=True,
                    detail=f"{name} (version unknown)",
                )
    is_mac = sys.platform == "darwin"
    fix = "brew install podman" if is_mac else "apt install podman  # or: apt install docker.io"
    return CheckResult(
        name="container-runtime",
        passed=False,
        detail="neither podman nor docker found on PATH",
        fix=fix,
    )


# Minimum memory (MB) for the podman VM to build images reliably.
# The rootfs build runs apt, npm, and curl installers concurrently inside the
# container -- 2 GB is not enough (OOM-killed exit 137 on Claude installer).
PODMAN_MIN_MEMORY_MB = 4096
PODMAN_RECOMMENDED_MEMORY_MB = 8192


def check_container_resources() -> CheckResult | None:
    """Check container runtime VM has enough memory and CPUs.

    Checks podman machine (macOS/Windows) or Docker Desktop resource limits.
    Returns None on native Linux or if resources can't be determined.
    """
    import json as _json

    # Podman machine (macOS/Windows)
    if shutil.which("podman") and sys.platform in ("darwin", "win32"):
        try:
            result = subprocess.run(
                ["podman", "machine", "inspect"],
                capture_output=True, text=True, timeout=10,
            )
            if result.returncode == 0:
                data = _json.loads(result.stdout)
                resources = data[0].get("Resources", {})
                memory_mb = resources.get("Memory", 0)
                cpus = resources.get("CPUs", 0)
                return _check_resources("podman VM", memory_mb, cpus,
                    fix=f"podman machine stop && podman machine set --memory {PODMAN_RECOMMENDED_MEMORY_MB} && podman machine start")
        except Exception:
            pass

    # Docker Desktop (macOS/Windows) -- uses docker info to read resource limits
    if shutil.which("docker") and sys.platform in ("darwin", "win32"):
        try:
            result = subprocess.run(
                ["docker", "info", "--format", "{{json .}}"],
                capture_output=True, text=True, timeout=10,
            )
            if result.returncode == 0:
                data = _json.loads(result.stdout)
                memory_bytes = data.get("MemTotal", 0)
                cpus = data.get("NCPU", 0)
                memory_mb = memory_bytes // (1024 * 1024)
                return _check_resources("Docker Desktop", memory_mb, cpus,
                    fix="Docker Desktop -> Settings -> Resources -> increase Memory to 8GB")
        except Exception:
            pass

    return None


def _check_resources(
    runtime_label: str, memory_mb: int, cpus: int, fix: str,
) -> CheckResult:
    """Evaluate container runtime resources against thresholds."""
    if memory_mb < PODMAN_MIN_MEMORY_MB:
        return CheckResult(
            name="container-resources",
            passed=False,
            detail=f"{runtime_label}: {memory_mb}MB RAM, {cpus} CPUs (minimum {PODMAN_MIN_MEMORY_MB}MB)",
            fix=fix,
        )
    detail = f"{runtime_label}: {memory_mb}MB RAM, {cpus} CPUs"
    if memory_mb < PODMAN_RECOMMENDED_MEMORY_MB:
        detail += f" (recommended {PODMAN_RECOMMENDED_MEMORY_MB}MB)"
    return CheckResult(name="container-resources", passed=True, detail=detail)


# Maximum acceptable clock skew (seconds) between host and container VM.
MAX_CLOCK_SKEW_SECONDS = 30


def check_container_clock() -> CheckResult | None:
    """Check if container VM clock is in sync with the host.

    On macOS, Podman and Docker Desktop run containers in a Linux VM whose
    clock can drift after sleep/wake. Skew beyond MAX_CLOCK_SKEW_SECONDS
    causes apt-get to reject release files as "not valid yet".
    Returns None on native Linux (no VM layer).
    """
    if sys.platform != "darwin":
        return None

    host_now = datetime.datetime.now(datetime.timezone.utc)

    # Try podman first, then docker
    for runtime, cmd in [
        ("podman", ["podman", "machine", "ssh", "--", "date", "-u", "+%s"]),
        ("docker", ["docker", "run", "--rm", "alpine", "date", "-u", "+%s"]),
    ]:
        if not shutil.which(runtime):
            continue
        try:
            result = subprocess.run(
                cmd, capture_output=True, text=True, timeout=15,
            )
            if result.returncode != 0:
                continue
            vm_epoch = int(result.stdout.strip())
            host_epoch = int(host_now.timestamp())
            skew = abs(host_epoch - vm_epoch)
            if skew > MAX_CLOCK_SKEW_SECONDS:
                direction = "behind" if vm_epoch < host_epoch else "ahead"
                fix = (
                    f"podman machine stop && podman machine start"
                    if runtime == "podman"
                    else "restart Docker Desktop"
                )
                return CheckResult(
                    name="container-clock",
                    passed=False,
                    detail=f"{runtime} VM clock is {skew}s {direction} host",
                    fix=fix,
                )
            return CheckResult(
                name="container-clock",
                passed=True,
                detail=f"{runtime} VM clock skew: {skew}s (ok)",
            )
        except Exception:
            continue

    return None


def check_rust_toolchain() -> CheckResult:
    """Check for rustup and cargo on PATH."""
    missing = []
    for name in ("rustup", "cargo"):
        if not shutil.which(name):
            missing.append(name)
    if missing:
        return CheckResult(
            name="rust-toolchain",
            passed=False,
            detail=f"{', '.join(missing)} not found",
            fix="curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh",
        )
    try:
        result = subprocess.run(
            ["rustup", "--version"],
            capture_output=True, text=True, timeout=10,
        )
        version = result.stdout.strip().split("\n")[0]
    except Exception:
        version = "rustup (version unknown)"
    return CheckResult(
        name="rust-toolchain",
        passed=True,
        detail=version,
    )


def check_cross_target(target: str) -> CheckResult:
    """Check if a rustup cross-compilation target is installed."""
    try:
        result = subprocess.run(
            ["rustup", "target", "list", "--installed"],
            capture_output=True, text=True, timeout=10,
        )
        installed = result.stdout.split()
        if target in installed:
            return CheckResult(
                name=f"target-{target.split('-')[0]}",
                passed=True,
                detail=f"target {target}",
            )
        return CheckResult(
            name=f"target-{target.split('-')[0]}",
            passed=False,
            detail=f"target {target} not installed",
            fix=f"rustup target add {target}",
        )
    except Exception:
        return CheckResult(
            name=f"target-{target.split('-')[0]}",
            passed=False,
            detail=f"cannot check target {target} (rustup not available)",
            fix=f"rustup target add {target}",
        )


def check_b3sum() -> CheckResult:
    """Check for b3sum on PATH."""
    if not shutil.which("b3sum"):
        return CheckResult(
            name="b3sum",
            passed=False,
            detail="b3sum not found",
            fix="cargo install b3sum",
        )
    try:
        result = subprocess.run(
            ["b3sum", "--version"],
            capture_output=True, text=True, timeout=10,
        )
        version = result.stdout.strip()
    except Exception:
        version = "b3sum (version unknown)"
    return CheckResult(name="b3sum", passed=True, detail=version)


def check_guest_config(guest_dir: Path) -> CheckResult:
    """Check that guest config directory has a valid build.toml."""
    config_dir = guest_dir / "config"
    build_toml = config_dir / "build.toml"

    if not config_dir.is_dir():
        return CheckResult(
            name="guest-config",
            passed=False,
            detail=f"config directory not found: {config_dir}",
            fix=f"capsem-builder init {guest_dir}",
        )

    if not build_toml.is_file():
        return CheckResult(
            name="guest-config",
            passed=False,
            detail=f"build.toml not found in {config_dir}",
            fix=f"capsem-builder init {guest_dir}",
        )

    try:
        import tomllib
    except ModuleNotFoundError:
        import tomli as tomllib  # type: ignore[no-redef]

    try:
        with open(build_toml, "rb") as f:
            data = tomllib.load(f)
    except Exception as e:
        return CheckResult(
            name="guest-config",
            passed=False,
            detail=f"invalid build.toml: {e}",
        )

    build = data.get("build", {})
    arches = build.get("architectures", {})
    count = len(arches)
    return CheckResult(
        name="guest-config",
        passed=True,
        detail=f"{guest_dir}/config/build.toml ({count} architecture{'s' if count != 1 else ''})",
    )


def check_source_files(repo_root: Path) -> CheckResult:
    """Check that required source files exist for build context assembly."""
    required = {
        "guest/artifacts/capsem-init": repo_root / "guest" / "artifacts" / "capsem-init",
        "guest/artifacts/capsem-bashrc": repo_root / "guest" / "artifacts" / "capsem-bashrc",
        "guest/artifacts/banner.txt": repo_root / "guest" / "artifacts" / "banner.txt",
        "guest/artifacts/tips.txt": repo_root / "guest" / "artifacts" / "tips.txt",
        "guest/artifacts/capsem-doctor": repo_root / "guest" / "artifacts" / "capsem-doctor",
        "guest/artifacts/capsem-bench": repo_root / "guest" / "artifacts" / "capsem-bench",
        "guest/artifacts/capsem_bench/": repo_root / "guest" / "artifacts" / "capsem_bench",
        "guest/artifacts/diagnostics/": repo_root / "guest" / "artifacts" / "diagnostics",
        "config/capsem-ca.crt": repo_root / "config" / "capsem-ca.crt",
    }

    missing = []
    for label, path in required.items():
        if label.endswith("/"):
            if not path.is_dir():
                missing.append(label.rstrip("/"))
        else:
            if not path.is_file():
                missing.append(label.split("/")[-1])

    if missing:
        return CheckResult(
            name="source-files",
            passed=False,
            detail=f"missing: {', '.join(missing)}",
            fix="files missing from guest/artifacts/ or config/ -- check your checkout",
        )

    total = len(required)
    return CheckResult(
        name="source-files",
        passed=True,
        detail=f"all {total} required source files present",
    )


# ---------------------------------------------------------------------------
# Compose all checks
# ---------------------------------------------------------------------------


def run_all_checks(guest_dir: Path, repo_root: Path) -> list[CheckResult]:
    """Run all prerequisite checks and return results."""
    results: list[CheckResult] = []
    results.append(check_container_runtime())
    resources_check = check_container_resources()
    if resources_check is not None:
        results.append(resources_check)
    clock_check = check_container_clock()
    if clock_check is not None:
        results.append(clock_check)
    results.append(check_rust_toolchain())
    results.append(check_cross_target("aarch64-unknown-linux-musl"))
    results.append(check_cross_target("x86_64-unknown-linux-musl"))
    results.append(check_b3sum())
    results.append(check_guest_config(guest_dir))
    results.append(check_source_files(repo_root))
    return results


# ---------------------------------------------------------------------------
# Output formatting
# ---------------------------------------------------------------------------


def format_results(results: list[CheckResult]) -> str:
    """Format check results as human-readable output."""
    lines: list[str] = []
    lines.append("capsem-builder doctor")
    lines.append("=" * 21)

    # Group by category based on check name
    categories: dict[str, list[CheckResult]] = {}
    for r in results:
        if r.name in ("container-runtime", "container-resources", "container-clock"):
            cat = "Container Runtime"
        elif r.name in ("rust-toolchain",) or r.name.startswith("target-"):
            cat = "Rust Toolchain"
        elif r.name == "b3sum":
            cat = "Build Tools"
        elif r.name == "guest-config":
            cat = "Guest Config"
        elif r.name == "source-files":
            cat = "Source Files"
        else:
            cat = "Other"
        categories.setdefault(cat, []).append(r)

    for cat_name, checks in categories.items():
        lines.append(f"\n== {cat_name} ==")
        for check in checks:
            lines.append(str(check))

    passed = sum(1 for r in results if r.passed)
    failed = len(results) - passed
    lines.append(f"\n{passed} passed, {failed} failed")
    return "\n".join(lines)
