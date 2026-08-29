from __future__ import annotations

import re
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[3]


def _gate_issues(name: str | None = None) -> str:
    """Everything the gate would issue, with real argv. See `helpers.gate`."""
    import sys as _sys

    _sys.path.insert(0, str(PROJECT_ROOT / "tests"))
    from helpers.gate import gate_issues

    return gate_issues(name)


def test_justfile_does_not_expose_legacy_guest_dir_knob() -> None:
    justfile = (PROJECT_ROOT / "justfile").read_text()

    assert "--guest-dir" not in justfile
    assert "capsem-builder build guest" not in justfile
    from capsem_builder.gate import config as gate_config

    assert " ".join(gate_config.load(PROJECT_ROOT).initrd.build).endswith(
        "capsem-builder agent config/docker/image"
    )
    assert "capsem-builder agent --arch" not in justfile


def test_justfile_routes_assets_through_profile_admin_rail() -> None:
    justfile = (PROJECT_ROOT / "justfile").read_text()
    materialize_config = (PROJECT_ROOT / "scripts" / "materialize-config.sh").read_text()

    # An image build without a profile is unrepresentable now: the argv is
    # built from one, so there is nothing to guard against with an `echo`.
    from capsem_builder.gate import config as gate_config
    from capsem_builder.gate.imagebuild import build_argv

    config = gate_config.load(PROJECT_ROOT)
    argv = " ".join(build_argv(config, profile="code", arch="arm64", template="all"))
    assert "--profile config/profiles/code/profile.toml" in argv
    assert "--config-root config" in argv
    assert "capsem-admin -- image build" in argv
    assert "capsem-admin -- manifest generate" in " ".join(config.initrd.manifest)
    assert "scripts/materialize-config.sh" in justfile
    assert "cargo run -p capsem-admin -- profile materialize" in materialize_config
    assert 'profile_paths=("$CONFIG_ROOT"/profiles/*/profile.toml)' in materialize_config
    assert '--config-root "$CONFIG_ROOT"' in materialize_config


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
                    failures.append(f"{path.relative_to(PROJECT_ROOT)}:{line_no}: {line.strip()}")

    assert not failures, (
        "active docs/skills still teach retired `just run`; use `just exec` for "
        "one-shot commands and `just shell` for interactive VMs:\n" + "\n".join(failures)
    )


def test_justfile_exposes_one_docs_build() -> None:
    justfile = (PROJECT_ROOT / "justfile").read_text()

    assert "\nbuild-docs: _pnpm-install\n" in justfile
    docs_block = justfile.split("\nbuild-docs: _pnpm-install\n", maxsplit=1)[1].split(
        "\n\n", maxsplit=1
    )[0]
    assert "bash scripts/check-web-surface.sh docs" in docs_block
    assert "bash scripts/check-web-surface.sh site" in docs_block


def test_every_recipe_the_gate_tells_an_operator_to_run_exists() -> None:
    """A remediation naming a recipe nobody wrote is a dead end at the worst moment.

    `hostimage.py`'s own docstring records the last time this happened:
    `install-image` and `cross-compile` both dispatched `just _build-host-image`,
    a recipe that has never existed, so both were broken at runtime and no test
    noticed -- each stopped at the recipe boundary instead of crossing it.

    It happened again. Sealing the Linux parity lane added a refusal that reads

        no Linux parity base image for capsem-linux-rust-base:<digest>. Its
        dependencies changed; run `just warm` to build it with network before
        the gate runs without.

    and `just warm` did not exist. The release stopped there, correctly, and
    handed the operator a command that fails. Bumping `web/app/pnpm-lock.yaml`
    for a security advisory is what re-keyed the image and found it.

    So: every ``just <recipe>`` the gate names in prose must be a recipe the
    justfile actually defines.
    """
    import ast

    justfile = (PROJECT_ROOT / "justfile").read_text(encoding="utf-8")
    defined = set(re.findall(r"^([a-z_][\w-]*)\s*[\w\"=]*.*:", justfile, re.MULTILINE))

    gate = PROJECT_ROOT / "build_system" / "builder" / "gate"
    named: dict[str, str] = {}
    for module in sorted(gate.glob("*.py")):
        tree = ast.parse(module.read_text(encoding="utf-8"))
        # Docstrings are excluded deliberately. `hostimage.py` describes the
        # earlier `_build-host-image` incident in prose, and a note about a
        # recipe that never existed is the point of that note -- what must not
        # exist is a *message handed to an operator* naming a dead command.
        docstrings = {
            ast.get_docstring(node, clean=False)
            for node in ast.walk(tree)
            if isinstance(node, ast.Module | ast.ClassDef | ast.FunctionDef)
        }
        for node in ast.walk(tree):
            if not isinstance(node, ast.Constant) or not isinstance(node.value, str):
                continue
            if node.value in docstrings:
                continue
            # Backticked, which is how these messages name a command. Without
            # it, ordinary prose ("just wrote the manifest") reads as a recipe.
            for recipe in re.findall(r"`just ([a-z_][\w-]*)", node.value):
                named.setdefault(recipe, module.name)

    missing = {name: where for name, where in named.items() if name not in defined}
    assert not missing, (
        f"the gate tells an operator to run recipes that do not exist (recipe -> module): {missing}"
    )
