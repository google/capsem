"""Thin gate adapter for the typed ``capsem-cache`` control plane."""

from __future__ import annotations

from ..cache.config import load_policy
from ..cache.controlmodels import ImageCachePolicy
from . import cachelayout
from .config import for_root
from .errors import GateError
from .proc import Runner
from .sourcecommit import SourceCommit


class CacheControl:
    """Record native cache mutations as ordinary gate process actions."""

    def __init__(self, runner: Runner) -> None:
        self._runner = runner
        self._repository = cachelayout.authority(for_root(runner.root))
        self._policy = load_policy(runner.root)

    def _run(self, *arguments: str, check: bool = True) -> int:
        return self._runner.run(
            (
                "capsem-cache",
                "--repository",
                str(self._repository),
                "--policy-repository",
                str(self._runner.root),
                *arguments,
            ),
            check=check,
        )

    def release(self, boundary: str, *, best_effort: bool = False) -> None:
        """Release exact working images after their final consumer."""
        control = self._policy.control
        if control is None or boundary not in control.docker.releases:
            expected = () if control is None else tuple(sorted(control.docker.releases))
            raise GateError(
                f"unknown cache release boundary {boundary!r}; expected "
                f"{', '.join(expected) or 'a configured boundary'}"
            )
        self._run(
            "release",
            boundary,
            "--apply",
            "--reason",
            f"gate completed cache boundary {boundary}",
            check=not best_effort,
        )

    def image_policy(self, resource: str) -> ImageCachePolicy:
        """Return one typed Docker image owner from the sole cache policy."""
        if self._policy.control is None:
            raise GateError("cache policy has no Docker image controls")
        try:
            image = self._policy.control.docker.images[resource]
        except KeyError:
            raise GateError(f"cache policy has no image resource {resource!r}") from None
        if image.maximum_count is None or image.maximum_age_hours is None:
            raise GateError(f"cache image resource {resource!r} has no receipt bounds")
        return image

    def reclaim(self, resource: str, *, keep: str, protect: tuple[str, ...] = ()) -> None:
        """Retire superseded tags around a caller-owned exact anchor."""
        protected = tuple(
            part for tag in sorted(set(protect) - {keep}) for part in ("--protect", tag)
        )
        self._run(
            "reclaim-image",
            resource,
            "--keep",
            keep,
            *protected,
            "--apply",
            "--reason",
            f"gate retained current {resource} generation",
        )

    def prune(self, *, best_effort: bool = False) -> None:
        """Apply routine filesystem and native retention policy."""
        self._run(
            "prune",
            "--apply",
            "--reason",
            "gate completed a reusable build rail",
            check=not best_effort,
        )

    def clean(self) -> None:
        """Explicit cold cleanup of repository and owned native caches."""
        self._run(
            "clean",
            "all",
            "--apply",
            "--reason",
            "operator requested a cold gate rebuild",
        )

    def capture_failure(
        self,
        *,
        label: str,
        run_id: str | None = None,
        source_commit: SourceCommit | None = None,
    ) -> None:
        """Preserve evidence without replacing the original gate failure."""
        identity = []
        if run_id is not None:
            identity.extend(("--run-id", run_id))
        if source_commit is not None:
            identity.extend(("--source-commit", str(source_commit)))
        self._run("capture-failure", "--label", label, *identity, check=False)

    def enforce(self, cache_id: str, label: str | None = None) -> None:
        """Enforce one cache owner's maximum before expensive work."""
        reason = f"gate cache enforcement for {label or cache_id}"
        self._run("enforce", cache_id, "--reason", reason)
