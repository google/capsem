"""Tiny Python rehearsal client for the Capsem native Rust API surface."""

from __future__ import annotations

import json
from dataclasses import dataclass
from typing import Any, Callable, Optional
from urllib import error, request


Transport = Callable[[str, str, Optional[dict[str, Any]]], Any]


class CapsemNativeError(RuntimeError):
    pass


@dataclass(frozen=True)
class CapsemNativeClient:
    base_url: str = "http://127.0.0.1:8788"
    transport: Transport | None = None

    def __post_init__(self) -> None:
        object.__setattr__(self, "data", DataApi(self))
        object.__setattr__(self, "ui", UiApi(self))
        object.__setattr__(self, "native", NativeApi(self))

    def _request(self, method: str, path: str, body: Optional[dict[str, Any]] = None) -> Any:
        if self.transport is not None:
            return self.transport(method, path, body)

        payload = None if body is None else json.dumps(body).encode("utf-8")
        req = request.Request(
            f"{self.base_url.rstrip('/')}{path}",
            data=payload,
            method=method,
            headers={"content-type": "application/json"},
        )
        try:
            with request.urlopen(req, timeout=15) as response:
                return json.loads(response.read().decode("utf-8"))
        except error.HTTPError as exc:
            detail = exc.read().decode("utf-8")
            raise CapsemNativeError(detail or str(exc)) from exc


@dataclass(frozen=True)
class NativeApi:
    client: CapsemNativeClient

    def deck_proof(self) -> dict[str, Any]:
        return self.client._request("GET", "/native/deck-proof")

    def artifacts(self) -> list[dict[str, Any]]:
        return self.client._request("GET", "/native/artifacts")

    def artifact(self, artifact_id: str) -> dict[str, Any]:
        return self.client._request("GET", f"/native/artifacts/{artifact_id}")


@dataclass(frozen=True)
class DataApi:
    client: CapsemNativeClient

    def __post_init__(self) -> None:
        object.__setattr__(self, "sqlite", SqliteApi(self.client))


@dataclass(frozen=True)
class SqliteApi:
    client: CapsemNativeClient

    def query(self, sql: str) -> dict[str, Any]:
        return self.client._request("POST", "/native/data/sqlite/query", {"sql": sql})


@dataclass(frozen=True)
class UiApi:
    client: CapsemNativeClient

    def render_artifact(self, artifact_id: str) -> dict[str, Any]:
        return self.client._request(
            "POST",
            "/native/ui/render-artifact",
            {"artifactId": artifact_id},
        )
