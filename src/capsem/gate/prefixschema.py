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
    cargo_target: str
    cargo_profiles: tuple[str, ...]
    cargo_target_max_gb: float
    lease_template: str
    name_length: int
    keep: int
    carried: tuple[str, ...]
    lent: tuple[str, ...]
    exports: tuple[str, ...]
    #: Trees a run writes but nothing publishes. Provenance for build
    #: scaffolding that is worth carrying between runs and must never be
    #: copied back into the checkout.
    produced: tuple[str, ...]

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

    @field_validator("build_cache", "cargo_target")
    @classmethod
    def _cache_is_positioned_against_the_prefix_root(cls, template: str) -> str:
        """One `{parent}`, so the cache cannot be relocated independently.

        They have to move together: a test that points the prefix root at a
        temporary directory and leaves the cache on the real filesystem gets a
        cross-device rename, which is a failure about neither of them.
        """
        if template.count("{parent}") != 1:
            raise ValueError("must position itself against {parent} exactly once")
        return template

    @model_validator(mode="after")
    def _cache_is_not_swept_as_a_prefix(self) -> PrefixConfig:
        """Retained output must not live where prefixes are reclaimed from.

        `prefix.sweep` reclaims every directory under `parent` except the
        newest `keep`, and it identifies a prefix by being there rather than by
        its name. A cache underneath would be deleted on the second run.
        """
        parent = PurePosixPath(self.parent)
        for name, template in (
            ("build_cache", self.build_cache),
            ("cargo_target", self.cargo_target),
        ):
            retained = PurePosixPath(template.format(parent=self.parent))
            if retained == parent or parent in retained.parents:
                raise ValueError(
                    f"{name} {template!r} is inside the prefix root "
                    f"{self.parent!r}, where a sweep would reclaim it as a prefix"
                )
        return self

    @model_validator(mode="after")
    def _cargo_profiles_are_plain_names(self) -> PrefixConfig:
        """Each names one directory cargo writes a profile's output into.

        They become symlinks inside the prefix, so a nested or escaping name
        would point the build somewhere the prefix does not own.
        """
        if not self.cargo_profiles:
            raise ValueError("cargo_profiles must name the profile directories cargo writes")
        for profile in self.cargo_profiles:
            if PurePosixPath(profile).name != profile or profile in {"", ".", ".."}:
                raise ValueError(f"cargo profile {profile!r} must be one plain directory name")
        return self

    @field_validator("cargo_target_max_gb")
    @classmethod
    def _cap_is_a_real_size(cls, cap: float) -> float:
        """A cap that can be switched off is not a cap.

        `[disk] required_free_gb` is the floor and stays one; this is the bound
        on the directory itself, and zero or negative would make every run
        discard what the run before it built.
        """
        if cap <= 0:
            raise ValueError("cargo_target_max_gb must be a positive size in GB")
        return cap
