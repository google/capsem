from __future__ import annotations

import pytest

from capsem.builder.image_plan import (
    SUPPORTED_IMAGE_ARCHES,
    derive_image_plan,
    dump_image_plan_json,
)
from capsem.builder.profiles import create_profile_draft, validate_profile_json

PROFILE_FIXTURE = "schemas/fixtures/profile-v2-valid.json"


def test_derive_image_plan_defaults_to_all_supported_arches() -> None:
    profile = create_profile_draft(
        "corp-dev",
        revision="2026.0520.10",
        name="Corp Dev",
    )

    plan = derive_image_plan(profile)
    dumped = dump_image_plan_json(plan)
    reparsed = plan.__class__.model_validate_json(dumped)

    assert plan == reparsed
    assert plan.schema_ == "capsem.image-plan.v1"
    assert plan.profile_id == "corp-dev"
    assert plan.profile_revision == "2026.0520.10"
    assert [arch.arch for arch in plan.arches] == list(SUPPORTED_IMAGE_ARCHES)
    assert plan.packages.system.distro == "debian"
    assert plan.packages.runtimes["python"] == "3.12"
    assert plan.tools["capsem_doctor"].required is True
    assert plan.package_contract_hash.startswith("blake3:")


def test_derive_image_plan_can_narrow_to_single_arch_from_profile_fixture() -> None:
    with open(PROFILE_FIXTURE, encoding="utf-8") as handle:
        profile = validate_profile_json(handle.read())

    plan = derive_image_plan(profile, arch="arm64")

    assert [arch.arch for arch in plan.arches] == ["arm64"]
    assert plan.arches[0].declared_assets.kernel.hash.startswith("blake3:")
    assert plan.packages.node_packages["playwright"] == "1.44.0"


def test_derive_image_plan_rejects_all_arches_when_profile_is_incomplete() -> None:
    with open(PROFILE_FIXTURE, encoding="utf-8") as handle:
        profile = validate_profile_json(handle.read())

    with pytest.raises(ValueError, match="missing VM assets for arch"):
        derive_image_plan(profile)
