"""What `config/gate.toml` says about the container fixtures the functional suites use.

Split from `buildschema`, which was at the module ceiling: Kingslanding's
pinned images and the capsem-debug test image are fixtures of the suites, not
products the gate builds and ships.
"""

from __future__ import annotations

from typing import Annotated

from pydantic import StringConstraints, model_validator

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


class DebugImageConfig(Strict):
    """capsem-debug: the test-tooling image, served to sessions by digest.

    `digests` pins one OCI image *manifest* per platform -- the bytes the
    harness serves and a registry returns for `repository@digest` -- never an
    index, so a platform cannot silently resolve to something else.
    """

    name: SafeToken
    context: str
    script: str
    repository: str
    cache_stage: SafeToken
    digests: dict[Platform, ManifestDigest]

    @model_validator(mode="after")
    def pins_at_least_one_platform(self) -> DebugImageConfig:
        if not self.digests:
            raise ValueError("debug_image.digests must pin at least one platform")
        return self
