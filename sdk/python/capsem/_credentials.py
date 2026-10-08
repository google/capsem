"""Explicit host credential injection; returns only opaque broker references."""
from __future__ import annotations

from . import _operations as api
from . import models
from ._transport import Transport


class Credentials:
    def __init__(self, transport: Transport) -> None:
        self._transport = transport

    async def inject(
        self, provider: str, value: str, *, storage: str = "file",
        request_timeout: float | None = None,
    ) -> models.CredentialInjectResponse:
        """Register host material and use its reference in a VM environment.

        Memory storage lasts for this service lifetime. This operation never
        retries or starts consent, and does not refresh supplied OAuth tokens.
        """
        if not isinstance(value, str) or not value:
            raise ValueError("credential value must be a nonempty string")
        try:
            wire_provider = models.CredentialInjectProvider(provider)
            wire_storage = models.CredentialStorage(storage)
        except (ValueError, TypeError):
            raise ValueError("invalid credential provider or storage") from None
        return await api.inject_credential(
            self._transport,
            body=models.CredentialInjectRequest(
                provider=wire_provider, value=value, storage=wire_storage,
            ),
            request_timeout=request_timeout,
        )
