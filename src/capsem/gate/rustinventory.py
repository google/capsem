"""Typed Rust test-target inventory shared by gate guards and runners.

Cargo owns target declarations. Nextest owns the executable test inventory.
Normalizing both sources to :class:`RustTarget` lets the gate compare identities
instead of grepping command text or trusting a test count.
"""

from __future__ import annotations

from collections.abc import Iterable

from pydantic import BaseModel, ConfigDict, Field

_NATIVE_KINDS = frozenset({"bin", "lib", "proc-macro", "test"})


class InventoryMismatch(ValueError):
    """Cargo and Nextest disagree about the native targets being tested."""


class RustTarget(BaseModel):
    """One testable Cargo target, independent of tool-specific identifiers."""

    model_config = ConfigDict(frozen=True)

    package: str
    name: str
    kind: str

    def render(self) -> str:
        """Return a stable human identity for diagnostics and run evidence."""
        return f"{self.package}:{self.kind}/{self.name}"


class RustTestInventory(BaseModel):
    """Native and doctest targets observed from one inventory source."""

    model_config = ConfigDict(frozen=True)

    native: frozenset[RustTarget] = frozenset()
    doctest: frozenset[RustTarget] = frozenset()

    @classmethod
    def from_cargo_metadata(cls, payload: object) -> RustTestInventory:
        """Normalize Cargo metadata's declared test and doctest targets."""
        metadata = _CargoMetadata.model_validate(payload)
        native: set[RustTarget] = set()
        doctest: set[RustTarget] = set()

        for package in metadata.packages:
            for target in package.targets:
                kind = _one_kind(target.kind, package=package.name, target=target.name)
                identity = RustTarget(package=package.name, name=target.name, kind=kind)
                if target.test and kind in _NATIVE_KINDS:
                    native.add(identity)
                if target.doctest and kind in _NATIVE_KINDS:
                    doctest.add(identity)

        return cls(native=frozenset(native), doctest=frozenset(doctest))

    @classmethod
    def from_nextest_list(cls, payload: object) -> RustTestInventory:
        """Normalize Nextest's listed suites; Nextest never owns doctests."""
        listing = _NextestList.model_validate(payload)
        native = {
            RustTarget(package=suite.package_name, name=suite.binary_name, kind=suite.kind)
            for suite in listing.rust_suites.values()
            if suite.status == "listed" and suite.kind in _NATIVE_KINDS
        }
        return cls(native=frozenset(native))

    def require_same_native_targets(self, nextest: RustTestInventory) -> None:
        """Fail with exact identities when Nextest does not match Cargo."""
        missing = self.native - nextest.native
        unexpected = nextest.native - self.native
        if not missing and not unexpected:
            return

        details: list[str] = []
        if missing:
            details.append(f"missing from Nextest: {_render(missing)}")
        if unexpected:
            details.append(f"not declared testable by Cargo: {_render(unexpected)}")
        raise InventoryMismatch("; ".join(details))


class _CargoTarget(BaseModel):
    model_config = ConfigDict(extra="ignore")

    name: str
    kind: tuple[str, ...]
    test: bool
    doctest: bool


class _CargoPackage(BaseModel):
    model_config = ConfigDict(extra="ignore")

    name: str
    targets: tuple[_CargoTarget, ...]


class _CargoMetadata(BaseModel):
    model_config = ConfigDict(extra="ignore")

    packages: tuple[_CargoPackage, ...]


class _NextestSuite(BaseModel):
    model_config = ConfigDict(extra="ignore", populate_by_name=True)

    package_name: str = Field(alias="package-name")
    binary_name: str = Field(alias="binary-name")
    kind: str
    status: str


class _NextestList(BaseModel):
    model_config = ConfigDict(extra="ignore", populate_by_name=True)

    rust_suites: dict[str, _NextestSuite] = Field(alias="rust-suites")


def _one_kind(kinds: tuple[str, ...], *, package: str, target: str) -> str:
    if len(kinds) != 1:
        rendered = ", ".join(kinds) if kinds else "<none>"
        raise ValueError(f"Cargo target {package}:{target} has ambiguous kinds: {rendered}")
    return kinds[0]


def _render(targets: Iterable[RustTarget]) -> str:
    return ", ".join(sorted(target.render() for target in targets))
