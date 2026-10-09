"""One bounded HTTP transport; no service discovery or mutation retries."""

from __future__ import annotations

import io
import math
from collections.abc import Mapping, Sequence
from contextlib import suppress
from enum import StrEnum
from typing import Self
from urllib.parse import quote, urlencode, urlsplit

import aiohttp
from pydantic import BaseModel, TypeAdapter
from yarl import URL

from .models.error_code import ErrorCode
from .models.error_response import ErrorResponse
from .models.model_base import JsonValue

QueryValue = str | int | bool | Sequence[str | int | bool] | None


class Method(StrEnum):
    GET = "GET"
    POST = "POST"
    PUT = "PUT"
    DELETE = "DELETE"


class MediaType(StrEnum):
    JSON = "application/json"
    BINARY = "application/octet-stream"
    GZIP = "application/gzip"


class CapsemError(Exception):
    """Base exception for Capsem SDK errors."""


class HttpError(CapsemError):
    """The gateway returned an unsuccessful HTTP status."""

    status: int
    body: str
    response: ErrorResponse | None
    code: ErrorCode | None

    def __init__(self, status: int, body: str) -> None:
        self.status = status
        self.body = body
        self.response = None
        with suppress(ValueError):
            self.response = ErrorResponse.model_validate_json(body)
        self.code = self.response.code if self.response is not None else None
        super().__init__(f"HTTP {status}: {body}")

    def __reduce__(self) -> tuple[type[Self], tuple[object, ...], dict[str, object]]:
        return (type(self), (self.status, self.body), self.__dict__.copy())

    def __setstate__(self, state: dict[str, object] | None, /) -> None:
        if state is not None:
            self.__dict__.update(state)
        self.args = (f"HTTP {self.status}: {self.body}",)


def _query_value(value: str | int | bool) -> str:
    return str(value).lower() if isinstance(value, bool) else str(value)


class Transport:
    def __init__(self, url: str, token: str, *, timeout: float = 30) -> None:
        parsed = urlsplit(url)
        if (parsed.scheme not in ("http", "https") or not parsed.hostname
                or parsed.username is not None or parsed.password is not None
                or parsed.query or parsed.fragment):
            raise ValueError("gateway URL must be HTTP(S), without credentials, query or fragment")
        if not token or "\r" in token or "\n" in token:
            raise ValueError("gateway bearer token is required and must not contain line breaks")
        if not math.isfinite(timeout) or timeout <= 0:
            raise ValueError("timeout must be positive and finite")
        self._url = url.rstrip("/")
        self._token = token
        self._timeout = timeout
        self._session: aiohttp.ClientSession | None = None
        self._closed = False

    @property
    def timeout(self) -> float:
        """The default per-request deadline, in seconds."""
        return self._timeout

    def __getstate__(self) -> dict[str, object]:
        return {**self.__dict__, "_session": None}

    async def __aenter__(self) -> Self:
        return self

    async def __aexit__(self, *_args: object) -> None:
        await self.close()

    async def close(self) -> None:
        self._closed = True
        if self._session is not None:
            await self._session.close()

    async def request(
        self, method: Method, path: str, *,
        path_parameters: Mapping[str, str] | None = None,
        query: Mapping[str, QueryValue] | None = None,
        body: BaseModel | JsonValue | bytes | None = None,
        json_body: bool = False,
        accept: MediaType = MediaType.JSON,
        timeout: float | None = None,
    ) -> bytes:
        """Send one request. `timeout` replaces the default deadline for it."""
        if self._closed:
            raise RuntimeError("SDK client is closed")
        if timeout is not None and (not math.isfinite(timeout) or timeout <= 0):
            raise ValueError("timeout must be positive and finite")
        if not path.startswith("/") or "?" in path or "#" in path:
            raise ValueError("operation path must be absolute without query or fragment")
        for name, value in (path_parameters or {}).items():
            # Encoded dots prevent a literal '.' or '..' identifier becoming traversal.
            encoded = quote(value, safe="").replace(".", "%2E")
            path = path.replace("{" + name + "}", encoded)
        if "{" in path or "}" in path:
            raise ValueError("operation has unresolved path parameters")
        pairs = []
        for key, value in (query or {}).items():
            if value is None:
                continue
            if isinstance(value, Sequence) and not isinstance(value, str):
                pairs.append((key, ",".join(_query_value(item) for item in value)))
            else:
                pairs.append((key, _query_value(value)))
        target = self._url + path + ("?" + urlencode(pairs) if pairs else "")
        headers = {"Authorization": f"Bearer {self._token}", "Accept": accept.value}
        data: str | bytes | io.BytesIO | None = None
        if json_body or isinstance(body, BaseModel):
            data = (body.model_dump_json(by_alias=True, exclude_unset=True)
                    if isinstance(body, BaseModel) else TypeAdapter(JsonValue).dump_json(body))
            headers["Content-Type"] = MediaType.JSON.value
        elif isinstance(body, bytes):
            # A sized stream: aiohttp warns on raw bytes over 1 MiB (file parts are larger).
            data = io.BytesIO(body)
            headers["Content-Type"] = MediaType.BINARY.value
        elif body is not None:
            raise TypeError("non-model JSON bodies require json_body=True")
        if self._session is None:
            self._session = aiohttp.ClientSession(timeout=aiohttp.ClientTimeout(total=self._timeout))
        async with self._session.request(
            method.value, URL(target, encoded=True), headers=headers, data=data, allow_redirects=False,
            timeout=aiohttp.ClientTimeout(total=self._timeout if timeout is None else timeout),
        ) as response:
            payload = await response.read()
            if not 200 <= response.status < 300:
                raise HttpError(response.status, payload.decode("utf-8", errors="replace"))
            return payload
