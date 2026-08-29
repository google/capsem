from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[3]


def test_python_lock_export_uses_one_strict_lock_mode() -> None:
    source = (
        PROJECT_ROOT / "build_system" / "scripts" / "audit" / "audit-python-lock.sh"
    ).read_text()
    export = source.split("uv export \\\n", maxsplit=1)[1].split(
        "uv run --project build_system", maxsplit=1
    )[0]

    assert "--locked" in export
    assert "--frozen" not in export, (
        "current uv rejects --frozen with --locked; --locked already refuses "
        "a stale or changing project lock"
    )
