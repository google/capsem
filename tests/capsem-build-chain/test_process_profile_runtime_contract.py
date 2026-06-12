from pathlib import Path


PROJECT_ROOT = Path(__file__).resolve().parents[2]
PROCESS_SRC = PROJECT_ROOT / "crates/capsem-process/src"


def test_capsem_process_runtime_does_not_load_settings_or_corp_files() -> None:
    forbidden = {
        "load_settings_and_corp_files": "runtime config must come from the selected profile, not settings.toml/corp reloads",
        "settings_config_path": "process logs must report profile runtime inputs, not settings.toml",
        "corp_config_paths": "corp files are merged by service/profile routes, not process runtime",
        "build_server_list_with_builtin": "process MCP runtime must use profile-only server construction",
        "build_server_list(": "process MCP refresh must use profile-only server construction",
    }

    offenders: list[str] = []
    for path in PROCESS_SRC.rglob("*.rs"):
        text = path.read_text()
        for needle, reason in forbidden.items():
            if needle in text:
                offenders.append(f"{path.relative_to(PROJECT_ROOT)} contains {needle!r}: {reason}")

    assert not offenders, "\n".join(offenders)
