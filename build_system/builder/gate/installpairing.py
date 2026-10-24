"""Exact immutable inputs for the existing installed-container proof."""

from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True)
class NativePairing:
    channel: str
    before_manifest: Path
    after_manifest: Path
    before_package: Path
    before_inputs: Path
    after_inputs: Path

    def __post_init__(self) -> None:
        if self.channel not in {"stable", "nightly"}:
            raise ValueError("installed pairing requires an existing release channel")
        if any(
            not path.is_absolute() or ".." in path.parts or path.resolve() == Path("/")
            for path in self.mounts
        ):
            raise ValueError("installed pairing must name exact absolute artifact inputs")

    @property
    def mounts(self) -> tuple[Path, ...]:
        return (
            self.before_manifest,
            self.after_manifest,
            self.before_package,
            self.before_inputs,
            self.after_inputs,
        )

    def arguments(self) -> tuple[str, ...]:
        return (
            "--release-channel",
            self.channel,
            "--release-baseline-channel",
            self.channel,
            "--release-transition",
            "binary_only",
            "--before-manifest",
            str(self.before_manifest),
            "--after-manifest",
            str(self.after_manifest),
            "--before-package",
            str(self.before_package),
            "--before-release-inputs",
            str(self.before_inputs),
            "--after-release-inputs",
            str(self.after_inputs),
        )
