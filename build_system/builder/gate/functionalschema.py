"""What `config/gate.toml` says about the container fixtures the functional suites use.

Split from `buildschema`, which was at the module ceiling: Kingslanding's
pinned images and the capsem-debug test image are fixtures of the suites, not
products the gate builds and ships.
"""

from __future__ import annotations

from pathlib import PurePosixPath
from typing import Annotated

from pydantic import PositiveInt, StringConstraints, field_validator, model_validator

from .configschema import SafeToken, Strict

ManifestDigest = Annotated[str, StringConstraints(pattern=r"^sha256:[0-9a-f]{64}$")]
Platform = Annotated[str, StringConstraints(pattern=r"^linux/[a-z0-9]+$")]


class KingslandingConfig(Strict):
    fixture_script: str
    fixture_dir: str
    suite_path: str
    benchmark_paths: tuple[str, ...]


class GreyjoyConfig(Strict):
    suite_path: str


class QualificationConfig(Strict):
    """The image qualification harness (tests/qualification): the suite, and
    the two variables naming a candidate image and its layout. Unset, it
    qualifies the reference image."""

    suite_path: str
    image_variable: str
    layout_variable: str


class NativeImagesConfig(Strict):
    """CI evidence only; fixed native platform and bounded owned artifacts."""

    module: SafeToken
    receipt_dir: str
    artifact_dir: str
    report_name: str
    report_variable: str
    layout_output: str
    platform: Platform
    system: SafeToken
    machine: SafeToken
    receipt_max_bytes: PositiveInt
    journal_max_bytes: PositiveInt
    journal_max_runs: PositiveInt
    artifact_max_bytes: PositiveInt
    artifact_max_files: PositiveInt

    @field_validator("receipt_dir", "artifact_dir", "report_name")
    @classmethod
    def owned_relative_path(cls, value: str) -> str:
        path = PurePosixPath(value)
        if path.is_absolute() or ".." in path.parts or not path.parts:
            raise ValueError("native evidence paths must stay relative and contained")
        return value


class PinnedImageConfig(Strict):
    """An image the suites serve by digest: capsem-debug, the test-tooling
    image, and the reference image, the official image every runtime test boots.

    `digests` pins one OCI image *manifest* per platform -- the bytes the
    harness serves and a registry returns for `repository@digest` -- never an
    index, so a platform cannot silently resolve to something else.
    `base_context`, when set, is the base image the build makes first and
    passes as BASE, as the image workflow does. The pin changes only by a
    deliberate commit: rebuilding or publishing the image never moves it.
    """

    name: SafeToken
    #: Producer-local Docker aliases, separate from the catalog/product name.
    local_name: SafeToken
    context: str
    base_context: str | None = None
    script: str
    repository: str
    cache_stage: SafeToken
    evidence_max_bytes: PositiveInt
    digests: dict[Platform, ManifestDigest]
    #: The variable an installed proof reads the image's layout path from,
    #: for an image the release proofs boot (the reference image).
    layout_variable: str | None = None

    @model_validator(mode="after")
    def pins_at_least_one_platform(self) -> PinnedImageConfig:
        if not self.digests:
            raise ValueError(f"{self.name}: digests must pin at least one platform")
        return self
