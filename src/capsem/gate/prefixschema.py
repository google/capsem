"""What `config/gate.toml` says about a run's private copy of the checkout.

Split out of `harnessschema`, which had grown past the module ceiling this
project holds itself to. The seam is a real one rather than a line count: every
other section there describes what the gate *does* -- what may run beside what,
how a run is recorded, how much disk it may take. This describes where the
subject under test physically lives, which is a question about isolation and
reuse, and it now has three rules of its own to enforce.
"""

from __future__ import annotations

from pathlib import PurePosixPath

from pydantic import field_validator, model_validator

from .configschema import Strict


class PrefixConfig(Strict):
    """Where a run's private copy of the checkout lives, and what it carries."""

    parent: str
    build_cache: str
    lease_template: str
    name_length: int
    keep: int
    carried: tuple[str, ...]
    lent: tuple[str, ...]
    exports: tuple[str, ...]

    @field_validator("lease_template")
    @classmethod
    def _lease_is_one_identity_filename(cls, template: str) -> str:
        if template.count("{identity}") != 1 or PurePosixPath(template).name != template:
            raise ValueError("lease_template must be one filename containing {identity} once")
        return template

    @model_validator(mode="after")
    def _paths_stay_inside(self) -> PrefixConfig:
        """`carried` and `exports` name places inside a checkout.

        An absolute entry would copy something the run does not own; a `..`
        entry would write outside the prefix on export, which is the one
        direction a private copy must never reach.
        """
        for group in (self.carried, self.lent, self.exports):
            for path in group:
                parts = PurePosixPath(path)
                if parts.is_absolute() or ".." in parts.parts:
                    raise ValueError(f"{path!r} must be relative and must not escape upwards")
        overlap = set(self.carried) & set(self.lent)
        if overlap:
            raise ValueError(
                f"{sorted(overlap)} is both carried from the checkout and lent "
                "between runs; the copy would overwrite the lent tree"
            )
        return self

    @model_validator(mode="after")
    def _cache_is_not_swept_as_a_prefix(self) -> PrefixConfig:
        """The lent output must not live where prefixes are reclaimed from.

        `prefix.sweep` reclaims every directory under `parent` except the
        newest `keep`, and it identifies a prefix by being there rather than by
        its name. A cache underneath would be deleted on the second run.
        """
        cache = PurePosixPath(self.build_cache)
        parent = PurePosixPath(self.parent)
        if cache == parent or parent in cache.parents:
            raise ValueError(
                f"build_cache {self.build_cache!r} is inside the prefix root "
                f"{self.parent!r}, where a sweep would reclaim it as a prefix"
            )
        return self
