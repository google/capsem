"""Connection ownership shared by the public gateway clients."""

from __future__ import annotations

from typing import Self

from ._transport import Transport


class Client:
    def __init__(self, url: str, token: str, *, timeout: float = 30) -> None:
        self.__transport = Transport(url, token, timeout=timeout)
        self._owns_connection = True
        self._closed = False

    @classmethod
    def _from_transport(cls, transport: Transport) -> Self:
        instance = cls.__new__(cls)
        instance.__transport = transport
        instance._owns_connection = False
        instance._closed = False
        return instance

    @property
    def _transport(self) -> Transport:
        if self._closed:
            raise RuntimeError("SDK client is closed")
        return self.__transport

    async def __aenter__(self) -> Self:
        if self._closed:
            raise RuntimeError("SDK client is closed")
        return self

    async def __aexit__(self, *_args: object) -> None:
        await self.close()

    async def close(self) -> None:
        if not self._closed:
            self._closed = True
            if self._owns_connection:
                await self.__transport.close()
