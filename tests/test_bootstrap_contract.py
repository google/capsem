from __future__ import annotations

from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[1]


def _gate_labels(name: str = "candidate") -> tuple[str, ...]:
    """Every step of a command's plan, in graph order. See `helpers.gate`."""
    import sys as _sys

    _sys.path.insert(0, str(PROJECT_ROOT / "tests"))
    from helpers.gate import gate_labels

    return gate_labels(name)


def _gate_issues(name: str | None = None) -> str:
    """Everything the gate would issue, with real argv. See `helpers.gate`."""
    import sys as _sys

    _sys.path.insert(0, str(PROJECT_ROOT / "tests"))
    from helpers.gate import gate_issues

    return gate_issues(name)


def _read(path: str) -> str:
    return (PROJECT_ROOT / path).read_text()


def test_bootstrap_always_checks_project_skills_and_site_shape() -> None:
    bootstrap = _read("bootstrap.sh")

    assert "check_bootstrap_shape" in bootstrap
    assert "check_bootstrap_shape\n\n# Ask the developer" in bootstrap
    for link in [
        ".agents/skills",
        ".claude/skills",
        ".codex/skills",
        ".cursor/skills",
        ".gemini/skills",
    ]:
        assert link in bootstrap
        assert "../skills" in bootstrap
    for required_file in [
        "skills/dev-sprint/SKILL.md",
        "skills/dev-testing/SKILL.md",
        "skills/dev-capsem/SKILL.md",
        "skills/ironbank/SKILL.md",
        "skills/frontend-design/SKILL.md",
        "site/package.json",
        "site/astro.config.mjs",
        "site/src/components/FAQ.svelte",
        "site/src/lib/data.ts",
    ]:
        assert required_file in bootstrap
    assert "find skills -mindepth 2 -name SKILL.md" in bootstrap


def test_bootstrap_runs_full_doctor_fix_without_a_parallel_check_mode() -> None:
    bootstrap = _read("bootstrap.sh")

    assert '"$SCRIPT_DIR/scripts/doctor-common.sh" --fix' in bootstrap
    assert "doctor-common.sh --check" not in bootstrap
    assert "dry-run" not in bootstrap.lower()


def test_bootstrap_uses_colima_exit_status_not_running_text() -> None:
    bootstrap = _read("bootstrap.sh")

    assert "colima status >/dev/null 2>&1" in bootstrap
    assert 'colima status 2>&1 | grep -qi "running"' not in bootstrap


def test_bootstrap_repairs_stale_live_rosetta_registration_before_docker_probe() -> None:
    bootstrap = _read("bootstrap.sh")

    registration = "colima ssh -- test -f /proc/sys/fs/binfmt_misc/rosetta"
    assert registration in bootstrap
    assert "colima restart" in bootstrap
    assert bootstrap.index(registration) < bootstrap.index("docker info >/dev/null")


def test_bootstrap_waits_for_container_dns_after_colima_restart() -> None:
    bootstrap = _read("bootstrap.sh")

    assert "docker run --rm --pull=missing alpine:3.20 getent hosts ghcr.io" in bootstrap
    assert "Docker DNS did not become ready" in bootstrap
    assert "for attempt in $(seq 1 30)" in bootstrap


def test_just_test_invokes_bootstrap_and_release_quality_gates() -> None:
    justfile = _read("justfile")
    web_gate = _read("scripts/check-web-surface.sh")

    # Quoted: a checkout under a path with a space split into two arguments
    # otherwise, so the recipe was not portable to `~/My Projects/capsem`.
    assert '_bootstrap:\n    sh {{quote(justfile_directory() / "bootstrap.sh")}} -y' in justfile
    # The gate is one plan now, so these are phases rather than recipe calls.
    labels = _gate_labels()
    assert any(label.startswith("fast.") for label in labels)
    assert "prepare.bootstrap" in labels
    assert "prepare.storage-budget" in labels
    assert "python.ruff" in labels
    assert "python.ty.strict" in labels
    for command in [
        "uv run capsem-builder validate-skills skills",
        "cargo clippy --workspace --all-targets -- -D warnings",
        "bash scripts/check-web-surface.sh frontend",
        "bash scripts/check-web-surface.sh docs",
        "bash scripts/check-web-surface.sh site",
        "bash scripts/check-web-surface.sh release-site",
    ]:
        assert command in _gate_issues()
    for command in [
        "pnpm --dir frontend run check",
        "pnpm --dir frontend run test",
        "pnpm --dir frontend run build",
    ]:
        assert command in web_gate


def test_both_release_lanes_reuse_fail_closed_static_module() -> None:
    binary_workflow = _read(".github/workflows/release.yaml")
    profile_workflow = _read(".github/workflows/release-assets.yaml")
    fast_gate = _read(".github/workflows/fast-gate.yaml")

    assert "uses: ./.github/workflows/fast-gate.yaml" in binary_workflow
    assert "uses: ./.github/workflows/fast-gate.yaml" in profile_workflow
    assert "run: just _test-fast" in fast_gate
    assert "run: just _test-static" in fast_gate
    assert "run: just test" not in binary_workflow
    assert "run: just test" not in profile_workflow
    assert "cargo clippy --workspace --all-targets -- -D warnings" in _gate_issues()


def test_frontend_release_gate_is_owned_by_the_canonical_test() -> None:
    justfile = _read("justfile")
    web_gate = _read("scripts/check-web-surface.sh")

    assert "\ntest-frontend:" not in justfile
    block = justfile.split("\n_test-candidate:", 1)[1].split("\n_build-host-image:", 1)[0]
    assert "bash scripts/check-web-surface.sh frontend" in block
    assert "pnpm --dir frontend run check" in web_gate
    assert "pnpm --dir frontend run test" in web_gate
    assert "pnpm --dir frontend run build" in web_gate
