"""Exact original OCI base bytes and the publication workflow's input-key owner."""

from __future__ import annotations

import json
import re
import shlex
from pathlib import Path
from typing import Annotated

import yaml
from capsem_builder.crypto import sha256
from capsem_builder.gate.configschema import Strict
from pydantic import StringConstraints

from .pinned_image import verify
from .producer_evidence import read_file, safe_tree

Digest = Annotated[str, StringConstraints(pattern=r"^sha256:[0-9a-f]{64}$")]
Key = Annotated[str, StringConstraints(pattern=r"^[0-9a-f]{64}$")]


def workflow_key(root: Path, name: str, base: str | None = None) -> str:
    """Read, never execute, the actual workflow's one canonical key invocation."""
    from images.ci.inputkey import derived_key, input_key

    if name not in {"base", "dev", "codex-cli", "claude-code", "agy", "antigravity-desktop"}:
        raise ValueError("native recipe is not allowlisted")
    workflow = yaml.safe_load((root / ".github/workflows/images.yaml").read_text())
    steps = workflow["jobs"]["base-arch" if name == "base" else "image-arch"]["steps"]
    (run,) = [step["run"] for step in steps if step.get("id") == "published"]
    command = run.split('key="$(')[1].split(')"')[0].replace("\\\n", " ")
    words = shlex.split(command)
    prefix = [
        "uv",
        "run",
        "--project",
        "build_system",
        "--frozen",
        "python",
        "images/ci/inputkey.py",
    ]
    if words[: len(prefix)] != prefix:
        raise ValueError("native key workflow owner differs")
    arguments = words[len(prefix) :]
    if name != "base":
        if (
            not base
            or not re.fullmatch(r"sha256:[0-9a-f]{64}", base)
            or arguments != ["--derived", "$IMAGE", "--with", "$BASE_DIGEST"]
        ):
            raise ValueError("native key requires the canonical derived owner and original base subject")
        return derived_key(name, base, root=root)
    if base is not None or arguments != ["images/base"]:
        raise ValueError("native base key workflow owner differs")
    return input_key([root / "images/base"], root=root)


def metadata(layout: Path, subject: str, platform: str, maximum: int):
    safe_tree(layout)
    if not re.fullmatch(r"sha256:[0-9a-f]{64}", subject):
        raise ValueError("native subject is not an immutable SHA256 descriptor")
    (entry,) = json.loads(read_file(layout / "index.json", maximum))["manifests"]
    blob = layout / "blobs/sha256"
    original = read_file(blob / subject[7:], maximum)
    if (entry["digest"], entry["size"]) != (subject, len(original)):
        raise ValueError("native index differs from original subject descriptor")
    manifest = json.loads(original)
    for descriptor in (manifest["config"], *manifest["layers"]):
        if (
            not re.fullmatch(r"sha256:[0-9a-f]{64}", descriptor["digest"])
            or type(descriptor["size"]) is not int
            or descriptor["size"] <= 0
        ):
            raise ValueError("native blob descriptor is not immutable and bounded")
    config = read_file(blob / manifest["config"]["digest"][7:], maximum)
    verify(layout, subject, platform)
    return original, config, manifest, json.loads(config)


class BaseBinding(Strict):
    """Original base metadata, small enough to transfer in the bounded receipt."""

    subject: Digest
    input_key: Key
    manifest_json: str
    config_json: str

    @classmethod
    def capture(cls, layout: Path, subject: str, platform: str, key: str, *, maximum: int):
        manifest, config, _, _ = metadata(layout, subject, platform, maximum)
        result = cls(
            subject=subject,
            input_key=key,
            manifest_json=manifest.decode(),
            config_json=config.decode(),
        )
        result.decoded(platform)
        return result

    def decoded(self, platform: str):
        if "sha256:" + sha256(self.manifest_json.encode()).hexdigest() != self.subject:
            raise ValueError("base manifest differs from its original subject")
        manifest, config = json.loads(self.manifest_json), json.loads(self.config_json)
        descriptor = manifest["config"]
        if (
            "sha256:" + sha256(self.config_json.encode()).hexdigest(),
            len(self.config_json.encode()),
        ) != (descriptor["digest"], descriptor["size"]):
            raise ValueError("base config differs from original descriptor")
        layers, root = manifest["layers"], config.get("rootfs", {})
        diffs = root.get("diff_ids", [])
        if (
            manifest.get("mediaType") != "application/vnd.oci.image.manifest.v1+json"
            or f"{config['os']}/{config['architecture']}" != platform
            or root.get("type") != "layers"
            or not layers
            or len(layers) != len(diffs)
            or any(not re.fullmatch(r"sha256:[0-9a-f]{64}", digest) for digest in diffs)
        ):
            raise ValueError("base platform or ordered layers differ")
        return manifest, config

    def require_child(self, layout: Path, subject: str, platform: str, *, maximum: int):
        base, config = self.decoded(platform)
        _, _, child, selected = metadata(layout, subject, platform, maximum)

        def prefix(layers):
            return [(layer["digest"], layer["size"]) for layer in layers]

        root = selected.get("rootfs", {})
        if (
            prefix(child["layers"][: len(base["layers"])]) != prefix(base["layers"])
            or root.get("type") != "layers"
            or root.get("diff_ids", [])[: len(base["layers"])] != config["rootfs"]["diff_ids"]
            or len(root.get("diff_ids", [])) != len(child["layers"])
        ):
            raise ValueError("candidate does not contain the exact original base prefix")


class BuildBinding(Strict):
    input_key: Key
    base: BaseBinding

    def require(
        self, root: Path, name: str, layout: Path, subject: str, platform: str, maximum: int
    ):
        if self.base.input_key != workflow_key(root, "base") or self.input_key != workflow_key(
            root, name, self.base.subject
        ):
            raise ValueError("native candidate producer inputs changed")
        self.base.require_child(layout, subject, platform, maximum=maximum)
