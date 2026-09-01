"""Contracts for profile bytes produced by the artifact materialization stage."""

from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[2]
PROFILES_DIR = PROJECT_ROOT / "config" / "profiles"
MATERIALIZED_PROFILES_DIR = PROJECT_ROOT / "cache" / "target" / "config" / "profiles"


def test_materialized_profile_root_payload_matches_source_profile_root() -> None:
    failures: list[str] = []
    for profile_dir in sorted(PROFILES_DIR.iterdir()):
        if not profile_dir.is_dir():
            continue
        profile_id = profile_dir.name
        materialized_dir = MATERIALIZED_PROFILES_DIR / profile_id
        if not materialized_dir.is_dir():
            failures.append(f"{profile_id}: missing materialized profile directory")
            continue

        source_root = profile_dir / "root"
        materialized_root = materialized_dir / "root"
        source_paths = {
            path.relative_to(source_root).as_posix()
            for path in source_root.rglob("*")
            if path.is_file()
        }
        materialized_paths = {
            path.relative_to(materialized_root).as_posix()
            for path in materialized_root.rglob("*")
            if path.is_file()
        }
        if source_paths != materialized_paths:
            missing = sorted(source_paths - materialized_paths)
            extra = sorted(materialized_paths - source_paths)
            failures.append(
                f"{profile_id}: materialized root payload drift "
                f"missing={missing} extra={extra}"
            )
            continue
        for rel in sorted(source_paths):
            source_bytes = (source_root / rel).read_bytes()
            materialized_bytes = (materialized_root / rel).read_bytes()
            if materialized_bytes != source_bytes:
                failures.append(f"{profile_id}: materialized root payload differs for {rel}")

    assert not failures, "materialized profile root drift:\n" + "\n".join(failures)
