from __future__ import annotations

from pathlib import Path


PROJECT_ROOT = Path(__file__).resolve().parents[1]

ADMIN_CONTRACT_MODULES = [
    "src/capsem/admin/cli.py",
    "src/capsem/builder/profiles.py",
    "src/capsem/builder/service_settings.py",
    "src/capsem/builder/image_plan.py",
    "src/capsem/builder/image_workspace.py",
    "src/capsem/builder/image_verify.py",
    "src/capsem/builder/image_sbom.py",
    "src/capsem/builder/manifest_check.py",
    "src/capsem/builder/manifest_generate.py",
    "src/capsem/builder/manifest_crypto.py",
    "src/capsem/builder/security_packs.py",
]


def test_admin_contract_modules_do_not_use_raw_json_boundaries() -> None:
    for relative in ADMIN_CONTRACT_MODULES:
        text = (PROJECT_ROOT / relative).read_text(encoding="utf-8")

        assert "json.loads" not in text, relative
        assert "json.dumps" not in text, relative


def test_doctor_surface_points_admins_to_capsem_admin_not_builder_init() -> None:
    doctor = (PROJECT_ROOT / "src" / "capsem" / "builder" / "doctor.py").read_text(
        encoding="utf-8"
    )
    admin_docs = (
        PROJECT_ROOT / "docs" / "src" / "content" / "docs" / "usage" / "admin-cli.md"
    ).read_text(encoding="utf-8")

    assert "capsem-admin doctor" in doctor
    assert "capsem-admin profile init-builtins" in doctor
    assert "capsem-builder doctor" not in doctor
    assert "capsem-builder init" not in doctor
    assert "capsem-admin doctor --profile" in admin_docs
    assert "guest/config" in admin_docs
