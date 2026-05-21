"""Local image asset verification for profile-derived image plans."""

from __future__ import annotations

from pathlib import Path
import tarfile
from typing import Literal, Mapping
import xml.etree.ElementTree as ET

import blake3
from pydantic import BaseModel, ConfigDict, Field

from capsem.builder.image_plan import ImagePlan
from capsem.builder.profiles import AssetDeclaration, VersionStr

ImageVerificationArch = Literal["arm64", "x86_64"]


class StrictModel(BaseModel):
    model_config = ConfigDict(extra="forbid", frozen=True, populate_by_name=True)


class ImageAssetVerification(StrictModel):
    arch: ImageVerificationArch
    kind: Literal["kernel", "initrd", "rootfs"]
    path: str
    exists: bool
    expected_size: int
    actual_size: int | None = None
    expected_hash: str
    actual_hash: str | None = None
    ok: bool
    failure: Literal["missing", "size_mismatch", "hash_mismatch"] | None = None


class ImageInventory(StrictModel):
    schema_: Literal["capsem.image-inventory.v1"] = Field(
        default="capsem.image-inventory.v1",
        alias="schema",
    )
    apt: dict[str, VersionStr] = Field(default_factory=dict)
    python_modules: dict[str, VersionStr] = Field(default_factory=dict)
    node_packages: dict[str, VersionStr] = Field(default_factory=dict)
    tools: dict[str, VersionStr] = Field(default_factory=dict)


ImageInventoryMap = Mapping[ImageVerificationArch, tuple[Path | None, ImageInventory]]


class ImageContractVerification(StrictModel):
    kind: Literal["apt", "python", "node", "tool"]
    name: str
    expected_version: str
    actual_version: str | None = None
    expected_source: Literal["guest", "host", "profile"] | None = None
    required: bool | None = None
    ok: bool
    failure: Literal["missing", "version_mismatch"] | None = None


class ImageInventoryVerification(StrictModel):
    arch: ImageVerificationArch
    path: str | None = None
    ok: bool
    failure: Literal["missing"] | None = None
    package_contract: list[ImageContractVerification] = Field(default_factory=list)
    tool_contract: list[ImageContractVerification] = Field(default_factory=list)


class ImageProbeVerification(StrictModel):
    kind: Literal["capsem_doctor_bundle"]
    path: str
    ok: bool
    tests: int
    failures: int
    errors: int
    skipped: int


class ImageVerificationReport(StrictModel):
    schema_: Literal["capsem.image-verification.v1"] = Field(
        default="capsem.image-verification.v1",
        alias="schema",
    )
    ok: bool
    profile_id: str
    profile_revision: str
    assets_dir: str
    assets: list[ImageAssetVerification]
    inventories: list[ImageInventoryVerification] = Field(default_factory=list)
    probes: list[ImageProbeVerification] = Field(default_factory=list)


def _blake3_hash(path: Path) -> str:
    hasher = blake3.blake3()
    with path.open("rb") as handle:
        while chunk := handle.read(1024 * 1024):
            hasher.update(chunk)
    return f"blake3:{hasher.hexdigest()}"


def _asset_filename(asset: AssetDeclaration) -> str:
    path = asset.url.path or ""
    filename = Path(path).name
    if not filename:
        raise ValueError(f"asset URL '{asset.url}' does not include a filename")
    return filename


def _verify_one(
    assets_dir: Path,
    arch: ImageVerificationArch,
    kind: Literal["kernel", "initrd", "rootfs"],
    asset: AssetDeclaration,
) -> ImageAssetVerification:
    path = assets_dir / arch / _asset_filename(asset)
    expected_hash = asset.hash
    expected_size = asset.size
    if not path.exists():
        return ImageAssetVerification(
            arch=arch,
            kind=kind,
            path=str(path),
            exists=False,
            expected_size=expected_size,
            expected_hash=expected_hash,
            ok=False,
            failure="missing",
        )

    actual_size = path.stat().st_size
    actual_hash = _blake3_hash(path)
    if actual_size != expected_size:
        failure: Literal["size_mismatch", "hash_mismatch"] = "size_mismatch"
    elif actual_hash != expected_hash:
        failure = "hash_mismatch"
    else:
        failure = None
    return ImageAssetVerification(
        arch=arch,
        kind=kind,
        path=str(path),
        exists=True,
        expected_size=expected_size,
        actual_size=actual_size,
        expected_hash=expected_hash,
        actual_hash=actual_hash,
        ok=failure is None,
        failure=failure,
    )


def _version_satisfies(expected: str, actual: str) -> bool:
    if actual in {"", "N/A"}:
        return False
    return expected == "*" or actual == expected


def _verify_contract_entry(
    *,
    kind: Literal["apt", "python", "node", "tool"],
    name: str,
    expected_version: str,
    actual_versions: dict[str, VersionStr],
    expected_source: Literal["guest", "host", "profile"] | None = None,
    required: bool | None = None,
) -> ImageContractVerification:
    actual_version = actual_versions.get(name)
    if actual_version is None:
        return ImageContractVerification(
            kind=kind,
            name=name,
            expected_version=expected_version,
            expected_source=expected_source,
            required=required,
            ok=False,
            failure="missing",
        )
    if not _version_satisfies(expected_version, actual_version):
        return ImageContractVerification(
            kind=kind,
            name=name,
            expected_version=expected_version,
            actual_version=actual_version,
            expected_source=expected_source,
            required=required,
            ok=False,
            failure="version_mismatch",
        )
    return ImageContractVerification(
        kind=kind,
        name=name,
        expected_version=expected_version,
        actual_version=actual_version,
        expected_source=expected_source,
        required=required,
        ok=True,
    )


def _verify_package_contract(
    plan: ImagePlan,
    inventory: ImageInventory,
) -> list[ImageContractVerification]:
    rows: list[ImageContractVerification] = []
    for name, expected_version in sorted(plan.packages.system.apt.items()):
        rows.append(
            _verify_contract_entry(
                kind="apt",
                name=name,
                expected_version=expected_version,
                actual_versions=inventory.apt,
            )
        )
    for name, expected_version in sorted(plan.packages.python_modules.items()):
        rows.append(
            _verify_contract_entry(
                kind="python",
                name=name,
                expected_version=expected_version,
                actual_versions=inventory.python_modules,
            )
        )
    for name, expected_version in sorted(plan.packages.node_packages.items()):
        rows.append(
            _verify_contract_entry(
                kind="node",
                name=name,
                expected_version=expected_version,
                actual_versions=inventory.node_packages,
            )
        )
    return rows


def _verify_tool_contract(
    plan: ImagePlan,
    inventory: ImageInventory,
) -> list[ImageContractVerification]:
    rows: list[ImageContractVerification] = []
    for name, tool in sorted(plan.tools.items()):
        if not tool.required:
            continue
        rows.append(
            _verify_contract_entry(
                kind="tool",
                name=name,
                expected_version=tool.version,
                actual_versions=inventory.tools,
                expected_source=tool.source.value,
                required=tool.required,
            )
        )
    return rows


def load_image_inventory_json(path: Path) -> ImageInventory:
    return ImageInventory.model_validate_json(path.read_text(encoding="utf-8"))


def dump_image_inventory_json(inventory: ImageInventory) -> str:
    return inventory.model_dump_json(by_alias=True, exclude_none=True, indent=2)


def load_doctor_bundle_probe(path: Path) -> ImageProbeVerification:
    with tarfile.open(path, "r:*") as bundle:
        member = next(
            (
                item
                for item in bundle.getmembers()
                if Path(item.name).name == "pytest-junit.xml" and item.isfile()
            ),
            None,
        )
        if member is None:
            raise ValueError(f"doctor bundle missing pytest-junit.xml: {path}")
        handle = bundle.extractfile(member)
        if handle is None:
            raise ValueError(f"doctor bundle cannot read pytest-junit.xml: {path}")
        root = ET.fromstring(handle.read())

    tests = int(root.attrib.get("tests", "0"))
    failures = int(root.attrib.get("failures", "0"))
    errors = int(root.attrib.get("errors", "0"))
    skipped = int(root.attrib.get("skipped", "0"))
    return ImageProbeVerification(
        kind="capsem_doctor_bundle",
        path=str(path),
        ok=tests > 0 and failures == 0 and errors == 0,
        tests=tests,
        failures=failures,
        errors=errors,
        skipped=skipped,
    )


def verify_image_assets(
    plan: ImagePlan,
    assets_dir: Path,
    *,
    inventories: ImageInventoryMap | None = None,
    probes: list[ImageProbeVerification] | None = None,
) -> ImageVerificationReport:
    assets: list[ImageAssetVerification] = []
    for arch in plan.arches:
        assets.extend(
            [
                _verify_one(
                    assets_dir,
                    arch.arch,
                    "kernel",
                    arch.declared_assets.kernel,
                ),
                _verify_one(
                    assets_dir,
                    arch.arch,
                    "initrd",
                    arch.declared_assets.initrd,
                ),
                _verify_one(
                    assets_dir,
                    arch.arch,
                    "rootfs",
                    arch.declared_assets.rootfs,
                ),
            ]
        )
    inventory_reports: list[ImageInventoryVerification] = []
    inventory_by_arch = inventories or {}
    for arch in plan.arches:
        inventory_tuple = inventory_by_arch.get(arch.arch)
        if inventory_tuple is None:
            inventory_reports.append(
                ImageInventoryVerification(
                    arch=arch.arch,
                    path=str(assets_dir / arch.arch / "image-inventory.json"),
                    ok=False,
                    failure="missing",
                )
            )
            continue
        inventory_path, inventory = inventory_tuple
        package_contract = _verify_package_contract(plan, inventory)
        tool_contract = _verify_tool_contract(plan, inventory)
        inventory_reports.append(
            ImageInventoryVerification(
                arch=arch.arch,
                path=str(inventory_path) if inventory_path is not None else None,
                ok=all(row.ok for row in package_contract)
                and all(row.ok for row in tool_contract),
                package_contract=package_contract,
                tool_contract=tool_contract,
            )
        )

    return ImageVerificationReport(
        ok=all(asset.ok for asset in assets)
        and all(inventory.ok for inventory in inventory_reports)
        and all(probe.ok for probe in (probes or [])),
        profile_id=plan.profile_id,
        profile_revision=plan.profile_revision,
        assets_dir=str(assets_dir),
        assets=assets,
        inventories=inventory_reports,
        probes=probes or [],
    )


def dump_image_verification_report_json(report: ImageVerificationReport) -> str:
    return report.model_dump_json(by_alias=True, exclude_none=True, indent=2)
