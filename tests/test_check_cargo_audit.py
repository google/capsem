from __future__ import annotations

import importlib.util
import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "scripts" / "check-cargo-audit.py"
SPEC = importlib.util.spec_from_file_location("check_cargo_audit_function_guard", SCRIPT)
assert SPEC is not None and SPEC.loader is not None
AUDIT = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = AUDIT
SPEC.loader.exec_module(AUDIT)


def _package(root: Path, name: str, version: str, source: str) -> dict[str, str]:
    package_root = root / name
    (package_root / "src").mkdir(parents=True)
    (package_root / "Cargo.toml").write_text(
        f'[package]\nname = "{name}"\nversion = "{version}"\n',
        encoding="utf-8",
    )
    (package_root / "src/lib.rs").write_text(source, encoding="utf-8")
    return {
        "name": name,
        "version": version,
        "manifest_path": str(package_root / "Cargo.toml"),
    }


def test_glib_function_advisory_accepts_resolved_graph_without_callers(
    tmp_path: Path,
) -> None:
    metadata = {
        "packages": [
            _package(tmp_path, "glib", "0.18.5", "pub struct VariantStrIter;"),
            _package(tmp_path, "capsem-tray", "1.6.0", "pub fn run() {}"),
        ]
    }

    assert AUDIT.validate_function_scoped_advisories(metadata) == [
        "RUSTSEC-2024-0429 glib 0.18.5: affected functions unreachable"
    ]


@pytest.mark.parametrize(
    "source",
    (
        "fn bad(value: glib::Variant) { value.array_iter_str().unwrap(); }",
        "fn bad(iter: glib::VariantStrIter<'_>) { drop(iter); }",
    ),
)
def test_glib_function_advisory_rejects_any_resolved_source_caller(
    tmp_path: Path,
    source: str,
) -> None:
    metadata = {
        "packages": [
            _package(tmp_path, "glib", "0.18.5", "pub struct VariantStrIter;"),
            _package(tmp_path, "consumer", "1.0.0", source),
        ]
    }

    with pytest.raises(ValueError, match=r"RUSTSEC-2024-0429.*consumer"):
        AUDIT.validate_function_scoped_advisories(metadata)


def test_glib_function_advisory_disappears_on_patched_line(tmp_path: Path) -> None:
    metadata = {
        "packages": [
            _package(tmp_path, "glib", "0.20.0", "pub struct VariantStrIter;"),
            _package(
                tmp_path,
                "consumer",
                "1.0.0",
                "fn uses_patched(value: glib::Variant) { value.array_iter_str(); }",
            ),
        ]
    }

    assert AUDIT.validate_function_scoped_advisories(metadata) == []
