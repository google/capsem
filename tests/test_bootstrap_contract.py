from __future__ import annotations

from pathlib import Path


PROJECT_ROOT = Path(__file__).resolve().parents[1]


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


def test_just_test_invokes_bootstrap_and_release_quality_gates() -> None:
    justfile = _read("justfile")

    assert "_bootstrap:\n    sh {{justfile_directory()}}/bootstrap.sh -y" in justfile
    assert "test: _bootstrap _install-tools _clean-stale _pnpm-install" in justfile
    for command in [
        "uv run ruff check .",
        "uv run ty check src/capsem",
        "uv run capsem-builder validate-skills skills",
        "cargo clippy --workspace --all-targets -- -D warnings",
        "pnpm run check",
        "pnpm run test",
        "pnpm run build",
    ]:
        assert command in justfile
