"""One bounded HTTP transport; no service discovery or mutation retries."""

from __future__ import annotations

import math
from collections.abc import Mapping, Sequence
from enum import StrEnum
from urllib.parse import quote, urlencode, urlsplit

import aiohttp
from pydantic import BaseModel
from yarl import URL

QueryValue = str | int | bool | Sequence[str | int | bool] | None


class Method(StrEnum):
    GET = "GET"
    POST = "POST"
    DELETE = "DELETE"


class MediaType(StrEnum):
    JSON = "application/json"
    BINARY = "application/octet-stream"


class HttpError(Exception):
    """The gateway returned an unsuccessful HTTP status."""

    def __init__(self, status: int, body: str) -> None:
        self.status = status
        self.body = body
        super().__init__(f"HTTP {status}: {body}")


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

    async def __aenter__(self) -> Transport:
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
        body: BaseModel | bytes | None = None,
        accept: MediaType = MediaType.JSON,
    ) -> bytes:
        if self._closed:
            raise RuntimeError("SDK client is closed")
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
        data: str | bytes | None = body if isinstance(body, bytes) else None
        if isinstance(body, BaseModel):
            data = body.model_dump_json(by_alias=True, exclude_unset=True)
            headers["Content-Type"] = MediaType.JSON.value
        elif body is not None:
            headers["Content-Type"] = MediaType.BINARY.value
        if self._session is None:
            self._session = aiohttp.ClientSession(timeout=aiohttp.ClientTimeout(total=self._timeout))
        async with self._session.request(
            method.value, URL(target, encoded=True), headers=headers, data=data, allow_redirects=False,
        ) as response:
            payload = await response.read()
            if not 200 <= response.status < 300:
                raise HttpError(response.status, payload.decode("utf-8", errors="replace"))
            return payload
