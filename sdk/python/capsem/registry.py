"""Friendly one-pull registry credentials for container creation."""

from __future__ import annotations

from dataclasses import dataclass

from . import models


@dataclass(frozen=True, slots=True, repr=False)
class Registry:
    username: str | None = None
    password: str | None = None
    ca_pem: str | None = None

    def __post_init__(self) -> None:
        for name in ("username", "password", "ca_pem"):
            value = getattr(self, name)
            if value is not None and not isinstance(value, str):
                raise TypeError(f"{name} must be a string")

    def __repr__(self) -> str:
        present = lambda value: "<redacted>" if value is not None else "<none>"
        return (
            "Registry("
            f"username={present(self.username)}, "
            f"password={present(self.password)}, "
            f"ca_pem={present(self.ca_pem)})"
        )

    def _wire(self) -> models.RegistryAccess:
        return models.RegistryAccess(
            username=self.username,
            password=self.password,
            ca_pem=self.ca_pem,
        )
