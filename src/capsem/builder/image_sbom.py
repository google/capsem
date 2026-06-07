"""SPDX SBOM generation from profile-derived image inventories."""

from __future__ import annotations

from datetime import UTC, datetime
from typing import Literal
from urllib.parse import quote
import re

from pydantic import BaseModel, ConfigDict, Field

from capsem.builder.image_plan import ImagePlan
from capsem.builder.image_verify import ImageInventory, ImageVerificationArch


class StrictModel(BaseModel):
    model_config = ConfigDict(extra="forbid", frozen=True, populate_by_name=True)


class SpdxCreationInfo(StrictModel):
    created: str
    creators: list[str]
    license_list_version: str = Field(default="3.25", alias="licenseListVersion")


class SpdxExternalRef(StrictModel):
    reference_category: Literal["PACKAGE-MANAGER"] = Field(alias="referenceCategory")
    reference_type: Literal["purl"] = Field(alias="referenceType")
    reference_locator: str = Field(alias="referenceLocator")


class SpdxPackage(StrictModel):
    spdx_id: str = Field(alias="SPDXID")
    name: str
    version_info: str = Field(alias="versionInfo")
    download_location: Literal["NOASSERTION"] = Field(
        default="NOASSERTION",
        alias="downloadLocation",
    )
    files_analyzed: Literal[False] = Field(default=False, alias="filesAnalyzed")
    supplier: Literal["NOASSERTION"] = "NOASSERTION"
    license_concluded: Literal["NOASSERTION"] = Field(
        default="NOASSERTION",
        alias="licenseConcluded",
    )
    license_declared: Literal["NOASSERTION"] = Field(
        default="NOASSERTION",
        alias="licenseDeclared",
    )
    copyright_text: Literal["NOASSERTION"] = Field(
        default="NOASSERTION",
        alias="copyrightText",
    )
    external_refs: list[SpdxExternalRef] = Field(
        default_factory=list,
        alias="externalRefs",
    )
    comment: str


class SpdxRelationship(StrictModel):
    spdx_element_id: str = Field(alias="spdxElementId")
    relationship_type: Literal["DESCRIBES"] = Field(alias="relationshipType")
    related_spdx_element: str = Field(alias="relatedSpdxElement")


class SpdxDocument(StrictModel):
    spdx_version: Literal["SPDX-2.3"] = Field(default="SPDX-2.3", alias="spdxVersion")
    data_license: Literal["CC0-1.0"] = Field(default="CC0-1.0", alias="dataLicense")
    spdx_id: Literal["SPDXRef-DOCUMENT"] = Field(
        default="SPDXRef-DOCUMENT",
        alias="SPDXID",
    )
    name: str
    document_namespace: str = Field(alias="documentNamespace")
    creation_info: SpdxCreationInfo = Field(alias="creationInfo")
    packages: list[SpdxPackage]
    relationships: list[SpdxRelationship] = Field(default_factory=list)
    document_describes: list[str] = Field(default_factory=list, alias="documentDescribes")


def _created_now() -> str:
    return datetime.now(UTC).replace(microsecond=0).isoformat().replace("+00:00", "Z")


def _spdx_token(value: str) -> str:
    token = re.sub(r"[^A-Za-z0-9.-]+", "-", value).strip("-")
    return token or "unknown"


def _purl(kind: Literal["apt", "python", "node"], name: str, version: str, arch: str) -> str:
    name_safe = "/" if kind == "node" else ""
    encoded_name = quote(name, safe=name_safe)
    encoded_version = quote(version, safe="")
    if kind == "apt":
        return f"pkg:deb/debian/{encoded_name}@{encoded_version}?arch={quote(arch)}"
    if kind == "python":
        return f"pkg:pypi/{encoded_name}@{encoded_version}"
    return f"pkg:npm/{encoded_name}@{encoded_version}"


def _package(
    *,
    kind: Literal["apt", "python", "node", "tool"],
    arch: ImageVerificationArch,
    name: str,
    version: str,
) -> SpdxPackage:
    external_refs: list[SpdxExternalRef] = []
    if kind in {"apt", "python", "node"}:
        external_refs.append(
            SpdxExternalRef(
                referenceCategory="PACKAGE-MANAGER",
                referenceType="purl",
                referenceLocator=_purl(kind, name, version, arch),
            )
        )
    return SpdxPackage(
        SPDXID=f"SPDXRef-{_spdx_token(arch)}-{kind}-{_spdx_token(name)}",
        name=name,
        versionInfo=version,
        externalRefs=external_refs,
        comment=f"capsem image inventory source={kind} arch={arch}",
    )


def generate_image_spdx_document(
    plan: ImagePlan,
    arch: ImageVerificationArch,
    inventory: ImageInventory,
    *,
    created: str | None = None,
) -> SpdxDocument:
    packages: list[SpdxPackage] = []
    for name, version in sorted(inventory.apt.items()):
        packages.append(_package(kind="apt", arch=arch, name=name, version=version))
    for name, version in sorted(inventory.python_modules.items()):
        packages.append(_package(kind="python", arch=arch, name=name, version=version))
    for name, version in sorted(inventory.node_packages.items()):
        packages.append(_package(kind="node", arch=arch, name=name, version=version))
    for name, version in sorted(inventory.tools.items()):
        packages.append(_package(kind="tool", arch=arch, name=name, version=version))

    describes = [package.spdx_id for package in packages]
    contract_id = plan.package_contract_hash.replace(":", "-")
    return SpdxDocument(
        name=(
            f"Capsem {plan.profile_id}@{plan.profile_revision} "
            f"{arch} guest image SBOM"
        ),
        documentNamespace=(
            "https://capsem.dev/spdx/"
            f"{_spdx_token(plan.profile_id)}/"
            f"{_spdx_token(plan.profile_revision)}/"
            f"{_spdx_token(arch)}/"
            f"{_spdx_token(contract_id)}"
        ),
        creationInfo=SpdxCreationInfo(
            created=created or _created_now(),
            creators=["Tool: capsem-admin"],
        ),
        packages=packages,
        relationships=[
            SpdxRelationship(
                spdxElementId="SPDXRef-DOCUMENT",
                relationshipType="DESCRIBES",
                relatedSpdxElement=spdx_id,
            )
            for spdx_id in describes
        ],
        documentDescribes=describes,
    )


def dump_spdx_document_json(document: SpdxDocument) -> str:
    return document.model_dump_json(by_alias=True, exclude_none=True, indent=2)
