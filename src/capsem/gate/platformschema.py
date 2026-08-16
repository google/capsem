"""What the published product runs on, and where that claim is proved.

Split out of `productschema`, which describes the install layout, the package
rail and the asset manifest -- what the machinery makes. This is a different
question: which operating systems a user can run the result on. It is also the
only part of the config a user reads back, through the README badges and the
support tables, so it is worth being able to find.
"""

from __future__ import annotations

from .configschema import Strict


class MacosPlatform(Strict):
    """The macOS floor, and the name users see for it."""

    minimum_version: str
    minimum_release_name: str


class LinuxDistribution(Strict):
    """One distribution release for the glow-up proof to run the package on.

    The image is derived from `version` rather than restated beside it --
    `debian:13-slim` and `debian:trixie-slim` are the same digest, so naming
    the release twice only creates somewhere for the two to disagree.
    """

    name: str
    version: str
    repository: str
    tag_suffix: str = ""
    digest: str
    libc: str
    """The libc this release ships, e.g. `glibc 2.39` or `musl 1.2.5`.

    A fact about the distribution rather than about Capsem, recorded so the
    support claim can be derived without running anything -- badges and docs
    need an answer at read time.  The glow-up proof reads the real libc out of
    the image and refuses to pass if it disagrees, so this cannot quietly rot
    into a wrong claim the way a hand-written support list does.
    """

    def image(self) -> str:
        """The digest-pinned probe, e.g. `debian:13-slim@sha256:...`."""
        return f"{self.repository}:{self.version}{self.tag_suffix}@{self.digest}"

    def label(self) -> str:
        return f"{self.name} {self.version}"

    def flavour(self) -> str:
        return self.libc.split()[0]

    def libc_version(self) -> tuple[int, ...]:
        return tuple(int(part) for part in self.libc.split()[1].split("."))

    def sort_key(self) -> tuple[int, ...]:
        """Release order, numerically: `24.04` is below `26.04`, and `3.9`
        below `3.21` only when the parts are compared as numbers."""
        return tuple(int(part) for part in self.version.split("."))


class LinuxPlatform(Strict):
    """The Linux floor, expressed as the glibc the binaries are built against.

    `minimum_glibc` is not chosen; it is derived from the shipped binaries by
    `scripts/derive-deb-libc-floor.py` and recorded here so the support claim
    and the shipped bytes can be compared.
    """

    minimum_glibc: str
    distributions: tuple[LinuxDistribution, ...]
    """Every release the glow-up proof runs the package on.

    One list, not a supported set beside an unsupported one: whether a release
    is served follows from its libc and the floor, so splitting them would mean
    writing down an answer that is already derivable -- and a release could
    then sit in the wrong list and still pass.
    """

    def supported(self) -> tuple[LinuxDistribution, ...]:
        """The releases the published binaries actually run on."""
        floor = tuple(int(part) for part in self.minimum_glibc.split("."))
        return tuple(
            distribution
            for distribution in self.distributions
            if distribution.flavour() == "glibc" and distribution.libc_version() >= floor
        )

    def claims(self) -> tuple[str, ...]:
        """The support claim, one line per distribution, oldest served first.

        Derived rather than listed, so proving a newer release -- Ubuntu 26.04
        beside 24.04 -- never narrows what is promised.
        """
        oldest: dict[str, LinuxDistribution] = {}
        for distribution in self.supported():
            current = oldest.get(distribution.name)
            if current is None or distribution.sort_key() < current.sort_key():
                oldest[distribution.name] = distribution
        return tuple(
            f"{distribution.name} {distribution.version} or later"
            for distribution in oldest.values()
        )


class PlatformsConfig(Strict):
    """What the published product runs on.

    These floors were written out longhand in the two public `install.sh`
    copies, in the Tauri bundle, in the README and in the docs, with nothing
    comparing them -- and they had already drifted: the installers refused
    anything below macOS 14 while the app bundle advertised 13.0. A user-facing
    support claim is exactly the kind of value that must have one home.
    """

    macos: MacosPlatform
    linux: LinuxPlatform
