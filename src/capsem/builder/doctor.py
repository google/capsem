"""Composable build prerequisite checks for capsem-builder.

Each check returns a CheckResult with pass/fail status, detail, and
optional fix instructions. Checks are pure functions that can be called
individually (from the build pipeline for fail-fast) or composed via
run_all_checks() for the doctor CLI command.
"""

from __future__ import annotations

import shutil
import subprocess
import sys
from dataclasses import dataclass, field
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
        if r.name == "container-runtime":
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
