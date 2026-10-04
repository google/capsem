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
    context: str
    base_context: str | None = None
    script: str
    repository: str
    cache_stage: SafeToken
    digests: dict[Platform, ManifestDigest]

    @model_validator(mode="after")
    def pins_at_least_one_platform(self) -> PinnedImageConfig:
        if not self.digests:
            raise ValueError(f"{self.name}: digests must pin at least one platform")
        return self
