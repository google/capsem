"""Release contract: user.toml is not a supported runtime/config rail."""

from __future__ import annotations

from pathlib import Path


PROJECT_ROOT = Path(__file__).resolve().parents[2]

LIVE_PATHS = [
    PROJECT_ROOT / "crates",
    PROJECT_ROOT / "scripts",
    PROJECT_ROOT / "tests",
    PROJECT_ROOT / "justfile",
    PROJECT_ROOT / "config",
    PROJECT_ROOT / "site",
    PROJECT_ROOT / "benchmarks",
]

FORBIDDEN = [
    "user.toml",
    "CAPSEM_USER_CONFIG",
    "user_config_path",
    "load_settings_files",
    "save_mcp_user_config",
    "load_mcp_user_config",
    "build_server_list(",
]

ALLOWLIST = {
    Path("tests/capsem-build-chain/test_no_legacy_user_config.py"),
    Path("tests/capsem-build-chain/test_process_profile_runtime_contract.py"),
}


def iter_files() -> list[Path]:
    files: list[Path] = []
    for root in LIVE_PATHS:
        if root.is_file():
            files.append(root)
            continue
        for path in root.rglob("*"):
            if not path.is_file():
                continue
            rel = path.relative_to(PROJECT_ROOT)
            if rel in ALLOWLIST:
                continue
            if "__pycache__" in rel.parts:
                continue
            if path.suffix in {".pyc", ".png", ".jpg", ".jpeg", ".gif", ".ico"}:
                continue
            files.append(path)
    return files


def test_no_live_code_mentions_legacy_user_config_rail() -> None:
    failures: list[str] = []
    for path in iter_files():
        try:
            text = path.read_text(errors="ignore")
        except UnicodeDecodeError:
            continue
        for needle in FORBIDDEN:
            if needle in text:
                failures.append(f"{path.relative_to(PROJECT_ROOT)} contains {needle!r}")

    assert not failures, "legacy user config rail survived:\n" + "\n".join(sorted(failures))


def test_mitm_local_benchmark_does_not_write_settings_policy() -> None:
    benchmark = PROJECT_ROOT / "tests/capsem-serial/test_mitm_local_benchmark.py"
    text = benchmark.read_text()

    assert "settings.toml" not in text
    assert "security.web.http_upstream_ports" not in text
