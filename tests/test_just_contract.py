from __future__ import annotations

import re
from pathlib import Path


PROJECT_ROOT = Path(__file__).resolve().parents[1]


def test_justfile_does_not_expose_legacy_guest_dir_knob() -> None:
    justfile = (PROJECT_ROOT / "justfile").read_text()

    assert "--guest-dir" not in justfile
    assert "capsem-builder build guest" not in justfile
    assert "capsem-builder agent config/docker/image" in justfile
    assert "capsem-builder agent --arch" not in justfile


def test_justfile_routes_assets_through_profile_admin_rail() -> None:
    justfile = (PROJECT_ROOT / "justfile").read_text()
    materialize_config = (PROJECT_ROOT / "scripts" / "materialize-config.sh").read_text()

    assert 'echo "ERROR: profile id required. Use: just build-assets <profile-id> [arm64|x86_64]"' in justfile
    assert '--profile "config/profiles/${PROFILE_ARG}/profile.toml"' in justfile
    assert "--config-root config" in justfile
    assert "cargo run -p capsem-admin -- image build" in justfile
    assert "cargo run -p capsem-admin -- manifest generate" in justfile
    assert "bash \"$ROOT/scripts/materialize-config.sh\"" in justfile
    assert "cargo run -p capsem-admin -- profile materialize" in materialize_config
    assert 'profile_paths=("$ROOT"/config/profiles/*/profile.toml)' in materialize_config
    assert "--config-root \"$CONFIG_ROOT\"" in materialize_config


def test_justfile_and_scripts_do_not_reintroduce_retired_escape_paths() -> None:
    roots = [
        PROJECT_ROOT / "justfile",
        PROJECT_ROOT / "bootstrap.sh",
        PROJECT_ROOT / ".github" / "workflows" / "ci.yaml",
        PROJECT_ROOT / ".github" / "workflows" / "release.yaml",
    ]
    retired = [
        "capsem-debug-upstream",
        "mock_server_runtime",
        "capsem-bench mitm-local",
        "guest/config",
        "--guest-dir",
    ]

    for path in roots:
        text = path.read_text()
        for needle in retired:
            assert needle not in text, f"{needle!r} still appears in {path}"


def test_active_docs_and_skills_do_not_teach_retired_just_run() -> None:
    """`just run` is gone; docs must teach `just exec` or `just shell`.

    This guard intentionally scans only active instruction surfaces, not
    changelog or sprint archaeology. `just run-service` and `just run-ui` remain
    valid recipe names and are not matched by the retired-command regex.
    """
    retired = re.compile(r"\bjust run(?:\s|['\"]|$)")
    roots = [
        PROJECT_ROOT / "docs" / "src" / "content" / "docs",
        PROJECT_ROOT / "skills",
    ]
    failures: list[str] = []
    for root in roots:
        for path in sorted(root.rglob("*")):
            if not path.is_file() or path.suffix.lower() not in {".md", ".mdx"}:
                continue
            for line_no, line in enumerate(path.read_text().splitlines(), start=1):
                if retired.search(line):
                    failures.append(
                        f"{path.relative_to(PROJECT_ROOT)}:{line_no}: {line.strip()}"
                    )

    assert not failures, (
        "active docs/skills still teach retired `just run`; use `just exec` for "
        "one-shot commands and `just shell` for interactive VMs:\n"
        + "\n".join(failures)
    )


def test_justfile_exposes_docs_release_gate() -> None:
    justfile = (PROJECT_ROOT / "justfile").read_text()

    assert "\ndocs: _pnpm-install\n" in justfile
    docs_block = justfile.split("\ndocs: _pnpm-install\n", maxsplit=1)[1].split(
        "\n\n", maxsplit=1
    )[0]
    assert "pnpm --dir docs run build" in docs_block
    assert "pnpm --dir site run build" in docs_block
