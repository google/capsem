"""Typed profile-derived image build plans for capsem-admin."""

from __future__ import annotations

from typing import Literal

import blake3
from pydantic import BaseModel, ConfigDict, Field, TypeAdapter

from capsem.builder.profiles import (
    ArchAssets,
    PackageContract,
    ProfilePayloadV2,
    ToolContract,
    VmNetworkMode,
)

SUPPORTED_IMAGE_ARCHES: tuple[Literal["arm64", "x86_64"], ...] = ("arm64", "x86_64")
ImageArch = Literal["all", "arm64", "x86_64"]


class StrictModel(BaseModel):
    model_config = ConfigDict(extra="forbid", frozen=True, populate_by_name=True)


class ImagePlanVm(StrictModel):
    memory_mib: int
    cpus: int
    disk_mib: int
    network: VmNetworkMode
    track_rootfs_dependencies: bool


class ImagePlanArch(StrictModel):
    arch: Literal["arm64", "x86_64"]
    declared_assets: ArchAssets


class ImagePlan(StrictModel):
    schema_: Literal["capsem.image-plan.v1"] = Field(
        default="capsem.image-plan.v1",
        alias="schema",
    )
    profile_id: str
    profile_revision: str
    profile_name: str
    guest_abi: str
    package_contract_hash: str
    vm: ImagePlanVm
    arches: list[ImagePlanArch]
    packages: PackageContract
    tools: dict[str, ToolContract]


def _package_contract_hash(profile: ProfilePayloadV2) -> str:
    payload = profile.packages.model_dump_json(
        by_alias=True,
        exclude_none=True,
    ).encode()
    return f"blake3:{blake3.blake3(payload).hexdigest()}"


def _selected_arches(arch: ImageArch) -> tuple[Literal["arm64", "x86_64"], ...]:
    if arch == "all":
        return SUPPORTED_IMAGE_ARCHES
    return (arch,)


def derive_image_plan(profile: ProfilePayloadV2, arch: ImageArch = "all") -> ImagePlan:
    selected = _selected_arches(arch)
    plan_arches: list[ImagePlanArch] = []
    for arch_name in selected:
        declared_assets = profile.vm.assets.get(arch_name)
        if declared_assets is None:
            raise ValueError(
                f"profile '{profile.id}' revision '{profile.revision}' "
                f"missing VM assets for arch '{arch_name}'"
            )
        plan_arches.append(
            ImagePlanArch(arch=arch_name, declared_assets=declared_assets)
        )

    return ImagePlan(
        profile_id=profile.id,
        profile_revision=profile.revision,
        profile_name=profile.name,
        guest_abi=profile.compatibility.guest_abi,
        package_contract_hash=_package_contract_hash(profile),
        vm=ImagePlanVm(
            memory_mib=profile.vm.memory_mib,
            cpus=profile.vm.cpus,
            disk_mib=profile.vm.disk_mib,
            network=profile.vm.network,
            track_rootfs_dependencies=profile.vm.track_rootfs_dependencies,
        ),
        arches=plan_arches,
        packages=profile.packages,
        tools=profile.tools,
    )


def dump_image_plan_json(plan: ImagePlan) -> str:
    return plan.model_dump_json(by_alias=True, exclude_none=True, indent=2)


def dump_image_plan_schema_json() -> str:
    schema = TypeAdapter(ImagePlan).json_schema(
        by_alias=True,
        ref_template="#/$defs/{model}",
    )
    return TypeAdapter(dict).dump_json(schema, indent=2).decode()
