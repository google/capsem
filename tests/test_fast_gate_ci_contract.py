"""Contracts for deterministic failures shared by smoke, test, and CI."""

from pathlib import Path
import re


ROOT = Path(__file__).resolve().parents[1]
JUSTFILE = (ROOT / "justfile").read_text(encoding="utf-8")


def _recipe(name: str) -> str:
    lines = JUSTFILE.splitlines()
    start = next(
        index
        for index, line in enumerate(lines)
        if line.startswith(f"{name}:") or line.startswith(f"{name} ")
    )
    end = next(
        (
            index
            for index in range(start + 1, len(lines))
            if lines[index]
            and not lines[index].startswith((" ", "\t", "#"))
        ),
        len(lines),
    )
    return "\n".join(lines[start:end])


def test_python_tests_use_the_tracked_lowercase_justfile_name() -> None:
    offenders: list[str] = []
    wrong_case_path = re.compile(r'\b(?:PROJECT_ROOT|ROOT)\s*/\s*["\']Justfile["\']')
    for path in (ROOT / "tests").rglob("*.py"):
        if wrong_case_path.search(path.read_text(encoding="utf-8")):
            offenders.append(path.relative_to(ROOT).as_posix())

    assert offenders == [], (
        "case-insensitive local filesystems hide these Linux CI failures: "
        + ", ".join(offenders)
    )


def test_smoke_reuses_the_complete_shared_fast_gate() -> None:
    smoke = _recipe("smoke")
    fast = _recipe("_test-fast")
    runner = _recipe("_test-candidate-run")

    assert smoke.index("just _test-fast") < smoke.index("just _check-assets")
    assert "scripts/check-source-syntax.py" in fast
    assert "just _test-release-contracts" in fast
    for required in (
        "scripts/check-cargo-audit.py",
        "scripts/audit-pnpm-bulk.py",
        "scripts/audit-python-lock.sh",
        "cargo clippy --workspace --all-targets -- -D warnings",
        "bash scripts/check-web-surface.sh frontend",
        "bash scripts/check-web-surface.sh release-site",
    ):
        assert required in runner


def test_fast_release_contracts_do_not_depend_on_ignored_build_outputs() -> None:
    release_contracts = _recipe("_test-candidate-run")
    materialized_test = (
        ROOT / "tests/capsem-build-chain/test_materialized_profile_payload.py"
    )
    source_contract = (
        ROOT / "tests/capsem-build-chain/test_profile_payload_contract.py"
    ).read_text(encoding="utf-8")

    assert materialized_test.is_file()
    assert (
        "--ignore=tests/capsem-build-chain/test_materialized_profile_payload.py"
        in release_contracts
    )
    assert (
        release_contracts.count(
            "tests/capsem-build-chain/test_materialized_profile_payload.py"
        )
        == 2
    )
    assert "MATERIALIZED_PROFILES_DIR" not in source_contract
