"""capsem-process consumes only the service's published active policy.

The service merges built-in defaults, settings.toml and corp config into
`vm/active_policy.toml`; the per-VM process boots from that file and receives
the same exact published bytes for reload. It never reads the source files, or
a reload could apply policy the service never published.
"""

from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[2]
PROCESS_SRC = PROJECT_ROOT / "crates/capsem-process/src"


def test_capsem_process_runtime_does_not_load_settings_or_corp_files() -> None:
    forbidden = {
        "load_settings_and_corp_files": "runtime config must come from the active policy file, not settings.toml/corp reloads",
        "settings_config_path": "process logs must report the active policy file, not settings.toml",
        "corp_config_paths": "corp files are merged by the service into the active policy file, not by the process",
    }

    offenders: list[str] = []
    for path in PROCESS_SRC.rglob("*.rs"):
        text = path.read_text()
        for needle, reason in forbidden.items():
            if needle in text:
                offenders.append(
                    f"{path.relative_to(PROJECT_ROOT)} contains {needle!r}: {reason}"
                )

    assert not offenders, "\n".join(offenders)
