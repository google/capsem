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
        object.__setattr__(self, "generate", GenerateApi(self))
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

    def reset_workspace(self) -> dict[str, Any]:
        return self.client._request("POST", "/native/workspace/reset", {})

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
class GenerateApi:
    client: CapsemNativeClient

    def image(
        self,
        *,
        artifact_id: str,
        title: str,
        prompt: str,
        provider: str = "gemini",
    ) -> dict[str, Any]:
        return self.client._request(
            "POST",
            "/native/generate/image",
            {
                "id": artifact_id,
                "title": title,
                "prompt": prompt,
                "provider": provider,
            },
        )


@dataclass(frozen=True)
class UiApi:
    client: CapsemNativeClient

    def sheet(
        self,
        *,
        artifact_id: str,
        title: str,
        columns: list[str],
        rows: list[dict[str, Any]],
        source: Optional[dict[str, Any]] = None,
    ) -> dict[str, Any]:
        return self.client._request(
            "POST",
            "/native/data/sheet",
            {
                "id": artifact_id,
                "title": title,
                "columns": columns,
                "rows": rows,
                "source": source,
            },
        )

    def table(
        self,
        *,
        artifact_id: str,
        title: str,
        source_artifact: str,
        columns: list[str],
        rows: list[dict[str, Any]],
        searchable: bool = True,
        filterable: bool = True,
        page_size: int = 10,
    ) -> dict[str, Any]:
        return self.client._request(
            "POST",
            "/native/ui/table",
            {
                "id": artifact_id,
                "title": title,
                "sourceArtifact": source_artifact,
                "columns": columns,
                "rows": rows,
                "searchable": searchable,
                "filterable": filterable,
                "pageSize": page_size,
            },
        )

    def chart(
        self,
        *,
        artifact_id: str,
        title: str,
        chart: str,
        source_artifact: str,
        data: list[dict[str, Any]],
        x: str,
        series: list[dict[str, Any]],
        x_label: str,
        y_label: str,
        y_unit: str,
        stack: bool = False,
        direction: str = "vertical",
        legend: Optional[str] = None,
        second_axis: Optional[dict[str, Any]] = None,
        export: Optional[list[str]] = None,
    ) -> dict[str, Any]:
        return self.client._request(
            "POST",
            "/native/ui/chart",
            {
                "id": artifact_id,
                "title": title,
                "chart": chart,
                "sourceArtifact": source_artifact,
                "data": data,
                "x": x,
                "series": series,
                "xLabel": x_label,
                "yLabel": y_label,
                "yUnit": y_unit,
                "stack": stack,
                "direction": direction,
                "legend": legend,
                "secondAxis": second_axis,
                "export": export or ["png", "svg"],
            },
        )

    def diagram(
        self,
        *,
        artifact_id: str,
        title: str,
        source: str,
        kind: str = "mermaid",
        export: Optional[list[str]] = None,
    ) -> dict[str, Any]:
        return self.client._request(
            "POST",
            "/native/ui/diagram",
            {
                "id": artifact_id,
                "title": title,
                "kind": kind,
                "source": source,
                "export": export or ["svg", "png"],
            },
        )

    def slide(
        self,
        *,
        artifact_id: str,
        title: str,
        blocks: list[dict[str, Any]],
    ) -> dict[str, Any]:
        return self.client._request(
            "POST",
            "/native/ui/slide",
            {"id": artifact_id, "title": title, "blocks": blocks},
        )

    def slide_deck(
        self,
        *,
        artifact_id: str,
        title: str,
        slides: list[dict[str, Any]],
        export: Optional[list[str]] = None,
    ) -> dict[str, Any]:
        return self.client._request(
            "POST",
            "/native/ui/slide-deck",
            {
                "id": artifact_id,
                "title": title,
                "slides": slides,
                "export": export or ["html", "pdf"],
            },
        )

    def render_artifact(self, artifact_id: str) -> dict[str, Any]:
        return self.client._request(
            "POST",
            "/native/ui/render-artifact",
            {"artifactId": artifact_id},
        )
