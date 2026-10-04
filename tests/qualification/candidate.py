"""The image under qualification, and what its own OCI config promises.

Main development qualifies the runtime against the reference image (the
official `dev`, pinned in `config/gate.toml`). `capsem-gate image-qualify`
names a candidate instead: an application image's digest-addressed layout,
consumed as built, never repacked. Either way the harness reads the image's
own config -- user, working directory, entrypoint -- and holds the workload
to it; nothing here supplies a startup configuration of its own.
"""

from __future__ import annotations

import json
import os
import tomllib
from dataclasses import dataclass
from pathlib import Path

from capsem_builder.gate import config as gate_config

from tests.fixtures.oci.pinned_image import PinnedImage, reference_image

ROOT = Path(__file__).resolve().parents[2]
_SETTINGS = gate_config.load(ROOT).functional.qualification
#: Set together by `capsem-gate image-qualify`; unset, the reference image.
NAME_ENV = _SETTINGS.image_variable
LAYOUT_ENV = _SETTINGS.layout_variable


@dataclass(frozen=True)
class Candidate:
    name: str
    layout: Path
    digest: str
    config: dict
    capabilities: frozenset[str]
    expect: dict

    @property
    def user(self) -> str:
        return self.config.get("User") or "0"

    @property
    def working_dir(self) -> str:
        return self.config.get("WorkingDir") or "/"

    @property
    def command(self) -> list[str]:
        return list(self.config.get("Entrypoint") or []) + list(self.config.get("Cmd") or [])


def _blob(layout: Path, digest: str) -> bytes:
    return (layout / "blobs" / "sha256" / digest.removeprefix("sha256:")).read_bytes()


def read(name: str, layout: Path) -> Candidate:
    """The candidate at `layout`: its one manifest, its config, and the
    capabilities its qualification manifest declares."""
    (entry,) = json.loads((layout / "index.json").read_text())["manifests"]
    manifest = json.loads(_blob(layout, entry["digest"]))
    image = json.loads(_blob(layout, manifest["config"]["digest"]))
    return Candidate(
        name=name,
        layout=layout,
        digest=entry["digest"],
        config=image.get("config") or {},
        capabilities=declared_capabilities(name),
        expect=declared(name).get("expect", {}),
    )


def declared(name: str) -> dict:
    """`images/<name>/qualify.toml`: what qualifying the image adds to the
    mandatory checks. It may only add -- capabilities, and the expectations
    their groups assert -- never remove a group or supply startup config."""
    path = ROOT / "images" / name / "qualify.toml"
    if not path.is_file():
        return {}
    manifest = tomllib.loads(path.read_text())
    unknown = set(manifest) - {"capabilities", "expect"}
    if unknown:
        raise ValueError(f"{path}: unsupported keys {sorted(unknown)}")
    return manifest


def declared_capabilities(name: str) -> frozenset[str]:
    return frozenset(declared(name).get("capabilities", ()))


def resolve(image: PinnedImage = reference_image) -> Candidate:
    """The candidate this run qualifies. Both variables or neither: a half-set
    pair would qualify the reference image and report it as the candidate."""
    name, layout = os.environ.get(NAME_ENV), os.environ.get(LAYOUT_ENV)
    if (name is None) != (layout is None):
        raise RuntimeError(f"set both {NAME_ENV} and {LAYOUT_ENV}, or neither")
    if name is None or layout is None:
        return read(image.settings().name, image.ready())
    return read(name, Path(layout))
