"""Profile-derived image build workspace materialization."""

from __future__ import annotations

from pathlib import Path
from typing import Literal

import blake3
import tomli_w
from pydantic import BaseModel, ConfigDict, Field

from capsem.builder.image_plan import ImageArch, ImagePlan, derive_image_plan
from capsem.builder.profiles import ProfilePayloadV2, dump_profile_toml


class StrictModel(BaseModel):
    model_config = ConfigDict(extra="forbid", frozen=True, populate_by_name=True)


class ImageWorkspaceFile(StrictModel):
    path: str
    size: int
    hash: str


class ImageWorkspaceReport(StrictModel):
    schema_: Literal["capsem.image-workspace.v1"] = Field(
        default="capsem.image-workspace.v1",
        alias="schema",
    )
    profile_id: str
    profile_revision: str
    out_dir: str
    package_contract_hash: str
    arches: list[Literal["arm64", "x86_64"]]
    files: list[ImageWorkspaceFile]


class ImageBuildReport(StrictModel):
    schema_: Literal["capsem.image-build.v1"] = Field(
        default="capsem.image-build.v1",
        alias="schema",
    )
    ok: bool
    dry_run: bool
    profile_id: str
    profile_revision: str
    output_dir: str
    workspace: ImageWorkspaceReport
    template: Literal["kernel", "rootfs"]


def _write_text(root: Path, relative: str, payload: str, files: list[ImageWorkspaceFile]) -> None:
    path = root / relative
    path.parent.mkdir(parents=True, exist_ok=True)
    if not payload.endswith("\n"):
        payload += "\n"
    path.write_text(payload, encoding="utf-8")
    data = path.read_bytes()
    files.append(
        ImageWorkspaceFile(
            path=relative,
            size=len(data),
            hash=f"blake3:{blake3.blake3(data).hexdigest()}",
        )
    )


def _toml(data: dict) -> str:
    return tomli_w.dumps(data)


def _build_config(plan: ImagePlan) -> dict:
    version_commands = {
        "node": "node --version 2>&1 | tr -d v",
        "npm": "npm --version 2>&1",
        "uv": "uv --version 2>&1 | awk '{print $2}'",
        "pip": "pip3 --version 2>&1 | awk '{print $2}'",
    }
    for tool_id, tool in sorted(plan.tools.items()):
        if not tool.required or tool.source != "guest":
            continue
        version_commands.setdefault(tool_id, _tool_version_command(tool_id))
    arch_configs = {
        "arm64": {
            "base_image": f"debian:{plan.packages.system.release}-slim",
            "docker_platform": "linux/arm64",
            "rust_target": "aarch64-unknown-linux-musl",
            "kernel_branch": "6.6",
            "kernel_image": "arch/arm64/boot/Image",
            "defconfig": "kernel/defconfig.arm64",
            "node_major": int(plan.packages.runtimes.get("node", "24").split(".")[0]),
        },
        "x86_64": {
            "base_image": f"debian:{plan.packages.system.release}-slim",
            "docker_platform": "linux/amd64",
            "rust_target": "x86_64-unknown-linux-musl",
            "kernel_branch": "6.6",
            "kernel_image": "arch/x86_64/boot/bzImage",
            "defconfig": "kernel/defconfig.x86_64",
            "node_major": int(plan.packages.runtimes.get("node", "24").split(".")[0]),
        },
    }
    selected = {arch.arch: arch_configs[arch.arch] for arch in plan.arches}
    return {
        "build": {
            "compression": "zstd",
            "compression_level": 15,
            "squashfs_block_size": "128K",
            "version_commands": version_commands,
            "architectures": selected,
        }
    }


def _tool_version_command(tool_id: str) -> str:
    if tool_id == "capsem_doctor":
        return "capsem-doctor --version 2>&1 | head -1"
    return f"{tool_id} --version 2>/dev/null | head -1"


def _manifest_config(plan: ImagePlan) -> dict:
    return {
        "image": {
            "name": plan.profile_id,
            "version": plan.profile_revision,
            "description": f"Profile-derived image workspace for {plan.profile_name}",
            "changelog": [
                {
                    "version": plan.profile_revision,
                    "date": "2026-05-20",
                    "changes": [
                        "Generated from signed Profile V2 package/tool contract.",
                    ],
                }
            ],
        }
    }


def _vm_resources(plan: ImagePlan) -> dict:
    return {
        "resources": {
            "cpu_count": plan.vm.cpus,
            "ram_gb": max(1, plan.vm.memory_mib // 1024),
            "scratch_disk_size_gb": max(1, plan.vm.disk_mib // 1024),
            "log_bodies": False,
            "max_body_capture": 4096,
            "retention_days": 30,
            "max_sessions": 100,
            "min_content_sessions": 25,
            "max_disk_gb": 100,
            "terminated_retention_days": 365,
        }
    }


def _package_sets(plan: ImagePlan) -> dict[str, dict]:
    package_sets: dict[str, dict] = {}
    apt_packages = [
        _format_package_spec(name, version, "=")
        for name, version in sorted(plan.packages.system.apt.items())
    ]
    if apt_packages:
        package_sets["config/packages/apt.toml"] = {
            "apt": {
                "name": "Profile System Packages",
                "manager": "apt",
                "install_cmd": "apt-get install -y --no-install-recommends",
                "packages": apt_packages,
                "network": {
                    "name": "Debian",
                    "domains": ["deb.debian.org", "security.debian.org"],
                    "allow_get": True,
                },
            }
        }

    python_packages = [
        _format_package_spec(name, version, "==")
        for name, version in sorted(plan.packages.python_modules.items())
    ]
    if python_packages:
        package_sets["config/packages/python.toml"] = {
            "python": {
                "name": "Profile Python Packages",
                "manager": "uv",
                "install_cmd": "uv pip install --system --break-system-packages",
                "packages": python_packages,
                "network": {
                    "name": "PyPI",
                    "domains": ["pypi.org", "files.pythonhosted.org"],
                    "allow_get": True,
                },
            }
        }

    node_packages = [
        _format_package_spec(name, version, "@")
        for name, version in sorted(plan.packages.node_packages.items())
    ]
    if node_packages:
        package_sets["config/packages/node.toml"] = {
            "node": {
                "name": "Profile Node Packages",
                "manager": "npm",
                "install_cmd": "npm install -g",
                "packages": node_packages,
                "network": {
                    "name": "npm",
                    "domains": ["registry.npmjs.org"],
                    "allow_get": True,
                },
            }
        }
    curl_installs = [
        f"{name}={url}"
        for name, url in sorted(plan.packages.curl_installs.items())
    ]
    if curl_installs:
        package_sets["config/packages/curl.toml"] = {
            "curl": {
                "name": "Profile Curl Installs",
                "manager": "curl",
                "install_cmd": "curl -fsSL",
                "packages": curl_installs,
                "version_commands": {
                    name: _tool_version_command(name)
                    for name in sorted(plan.packages.curl_installs)
                },
                "network": {
                    "name": "Curl installers",
                    "domains": ["antigravity.google", "edgedl.me.gvt1.com"],
                    "allow_get": True,
                },
            }
        }
    return package_sets


def _format_package_spec(name: str, version: str, separator: str) -> str:
    if version in {"*", "latest", "build-time"}:
        return name
    return f"{name}{separator}{version}"


def materialize_profile_image_workspace(
    profile: ProfilePayloadV2,
    out_dir: Path,
    *,
    arch: ImageArch = "all",
    force: bool = False,
) -> ImageWorkspaceReport:
    if out_dir.exists() and any(out_dir.iterdir()) and not force:
        raise FileExistsError(f"{out_dir} is not empty; pass --force to overwrite")

    out_dir.mkdir(parents=True, exist_ok=True)
    plan = derive_image_plan(profile, arch=arch)
    files: list[ImageWorkspaceFile] = []

    _write_text(out_dir, "profile.toml", dump_profile_toml(profile), files)
    _write_text(out_dir, "image-plan.json", plan.model_dump_json(by_alias=True, indent=2), files)
    _write_text(out_dir, "config/build.toml", _toml(_build_config(plan)), files)
    _write_text(out_dir, "config/manifest.toml", _toml(_manifest_config(plan)), files)
    _write_text(out_dir, "config/vm/resources.toml", _toml(_vm_resources(plan)), files)
    for relative, data in _package_sets(plan).items():
        _write_text(out_dir, relative, _toml(data), files)

    return ImageWorkspaceReport(
        profile_id=plan.profile_id,
        profile_revision=plan.profile_revision,
        out_dir=str(out_dir),
        package_contract_hash=plan.package_contract_hash,
        arches=[item.arch for item in plan.arches],
        files=files,
    )


def dump_image_workspace_report_json(report: ImageWorkspaceReport) -> str:
    return report.model_dump_json(by_alias=True, exclude_none=True, indent=2)


def dump_image_build_report_json(report: ImageBuildReport) -> str:
    return report.model_dump_json(by_alias=True, exclude_none=True, indent=2)
