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
import re
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
    declaration = declared(name)
    (entry,) = json.loads((layout / "index.json").read_text())["manifests"]
    manifest = json.loads(_blob(layout, entry["digest"]))
    image = json.loads(_blob(layout, manifest["config"]["digest"]))
    return Candidate(
        name=name,
        layout=layout,
        digest=entry["digest"],
        config=image.get("config") or {},
        capabilities=frozenset(declaration.get("capabilities", ())),
        expect=declaration.get("expect", {}),
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
    if "agent" in manifest.get("capabilities", ()):
        _validate_agent_expectation(path, manifest.get("expect", {}))
    return manifest


def _validate_agent_expectation(path: Path, expect: dict) -> None:
    """Refuse unsupported agent contracts before collection can start a fixture.

    The current real-turn owner supports an explicit synthetic API-key
    environment, not arbitrary vendor enrollment. Never invent a key for an
    OAuth-only agent merely to make qualification collect.
    """
    if not isinstance(expect, dict):
        raise TypeError(f"{path}: agent [expect] requires a table")
    turn = expect.get("turn")
    if not isinstance(turn, list) or not turn or not all(
        isinstance(argument, str) and argument.strip() for argument in turn
    ):
        raise ValueError(f"{path}: agent [expect].turn requires a nonempty argv list")
    credential = expect.get("credential_env")
    fixture = expect.get("credential_fixture")
    if fixture is not None:
        if (
            fixture != "agy_oauth" or credential is not None
            or expect.get("provider") != "google" or turn[0] != "agy"
            or expect.get("path") != "/v1internal:streamGenerateContent"
        ):
            raise ValueError(f"{path}: agent credential_fixture must select only the existing Google agy_oauth fixture")
    elif not isinstance(credential, str) or not re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*", credential):
        raise ValueError(f"{path}: agent [expect].credential_env requires a supported synthetic credential environment name")
    provider = expect.get("provider")
    if not isinstance(provider, str) or not provider.strip():
        raise ValueError(f"{path}: agent [expect].provider requires a nonempty provider identity")
    endpoint = expect.get("path")
    if not isinstance(endpoint, str) or not endpoint.startswith("/") or any(
        character.isspace() for character in endpoint
    ):
        raise ValueError(f"{path}: agent [expect].path requires an absolute provider request path")


def declared_capabilities(name: str) -> frozenset[str]:
    return frozenset(declared(name).get("capabilities", ()))


def _selection(image: PinnedImage) -> tuple[str, Path | None]:
    name, layout = os.environ.get(NAME_ENV), os.environ.get(LAYOUT_ENV)
    if (name is None) != (layout is None):
        raise RuntimeError(f"set both {NAME_ENV} and {LAYOUT_ENV}, or neither")
    if name is None or layout is None:
        return image.settings().name, None
    return name, Path(layout)


def selected_capabilities(image: PinnedImage = reference_image) -> frozenset[str]:
    """Selection reads source declarations; collection needs no built artifacts."""
    name, _ = _selection(image)
    return declared_capabilities(name)


def resolve(image: PinnedImage = reference_image) -> Candidate:
    """Read actual image bytes, refusing a half-set candidate selector."""
    name, layout = _selection(image)
    return read(name, image.ready() if layout is None else layout)
