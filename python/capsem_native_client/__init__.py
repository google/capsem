"""Tiny Python rehearsal client for the Capsem native Rust API surface."""

from __future__ import annotations

import json
from dataclasses import dataclass
from typing import Any, Callable, Optional
from urllib import error, request

from .generated_native_artifact import (
    NATIVE_ARTIFACT_COMPONENTS,
    NATIVE_ARTIFACT_KINDS,
    NATIVE_ARTIFACT_MEDIA,
    PLOTLY_CHART_KINDS,
    PLOTLY_DIRECTIONS,
    PLOTLY_STACK_MODES,
)

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
        object.__setattr__(self, "export", ExportApi(self))
        object.__setattr__(self, "workspace", WorkspaceApi(self))
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
        payload = self.client._request("GET", "/native/artifacts")
        if not isinstance(payload, list):
            raise CapsemNativeError("native artifacts response must be a list")
        return [parse_native_artifact(value) for value in payload]

    def artifact(self, artifact_id: str) -> dict[str, Any]:
        return parse_native_artifact(self.client._request("GET", f"/native/artifacts/{artifact_id}"))

    def telemetry(self) -> list[dict[str, Any]]:
        return self.client._request("GET", "/native/telemetry")

    def mcp_tools(self) -> dict[str, Any]:
        return self.client._request("GET", "/native/mcp/tools")


@dataclass(frozen=True)
class WorkspaceApi:
    client: CapsemNativeClient

    def reset(self) -> dict[str, Any]:
        return self.client._request("POST", "/native/workspace/reset", {})

    def info(self) -> dict[str, Any]:
        return self.client._request("GET", "/native/workspace/info")

    def snapshot(self) -> dict[str, Any]:
        return self.client._request("GET", "/native/workspace/snapshot")

    def projection(self) -> dict[str, Any]:
        return self.client._request("GET", "/native/workspace/projection")

    def checkpoint(self) -> dict[str, Any]:
        return self.client._request("POST", "/native/workspace/checkpoint", {})

    def select(self, target: Optional[str]) -> dict[str, Any]:
        return self.client._request("POST", "/native/workspace/select", {"target": target})

    def delete(self, target: str) -> dict[str, Any]:
        return self.client._request("POST", "/native/workspace/delete", {"target": target})

    def title(self, *, target: str, title: str) -> dict[str, Any]:
        return self.client._request(
            "POST",
            "/native/workspace/title",
            {"target": target, "title": title},
        )

    def mutate_title(self, *, target: str, title: str) -> dict[str, Any]:
        return self.client._request(
            "POST",
            "/native/workspace/mutate",
            {"type": "title", "target": target, "title": title},
        )

    def mutate_text(
        self,
        *,
        target: str,
        text: str,
        selector: Optional[str] = None,
        host_selector: Optional[str] = None,
        shadow_selector: Optional[str] = None,
        source_request_seq: Optional[int] = None,
    ) -> dict[str, Any]:
        return self.client._request(
            "POST",
            "/native/workspace/mutate",
            {
                "type": "text",
                "target": target,
                "text": text,
                "selector": selector,
                "hostSelector": host_selector,
                "shadowSelector": shadow_selector,
                "sourceRequestSeq": source_request_seq,
            },
        )

    def comment(
        self,
        *,
        target: str,
        instruction: str,
        annotation: Optional[dict[str, Any]] = None,
    ) -> dict[str, Any]:
        body: dict[str, Any] = {"target": target, "instruction": instruction}
        if annotation is not None:
            body["annotation"] = annotation
        return self.client._request("POST", "/native/workspace/change-request", body)

    def resolve(self, task_id: str) -> dict[str, Any]:
        return self.client._request("POST", "/native/workspace/resolve", {"taskId": task_id})

    def style(
        self,
        *,
        target: str,
        styles: dict[str, str],
        selector: Optional[str] = None,
        host_selector: Optional[str] = None,
        shadow_selector: Optional[str] = None,
        source_request_seq: Optional[int] = None,
    ) -> dict[str, Any]:
        return self.client._request(
            "POST",
            "/native/workspace/style",
            {
                "target": target,
                "selector": selector,
                "hostSelector": host_selector,
                "shadowSelector": shadow_selector,
                "styles": styles,
                "sourceRequestSeq": source_request_seq,
            },
        )

    def mutate_style(
        self,
        *,
        target: str,
        styles: dict[str, str],
        selector: Optional[str] = None,
        host_selector: Optional[str] = None,
        shadow_selector: Optional[str] = None,
        source_request_seq: Optional[int] = None,
    ) -> dict[str, Any]:
        return self.client._request(
            "POST",
            "/native/workspace/mutate",
            {
                "type": "style",
                "target": target,
                "selector": selector,
                "hostSelector": host_selector,
                "shadowSelector": shadow_selector,
                "styles": styles,
                "sourceRequestSeq": source_request_seq,
            },
        )


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

    def text(
        self,
        *,
        artifact_id: str,
        title: str,
        prompt: str,
        system: Optional[str] = None,
        provider: str = "gemini",
        model: Optional[str] = None,
    ) -> dict[str, Any]:
        return self.client._request(
            "POST",
            "/native/generate/text",
            {
                "id": artifact_id,
                "title": title,
                "prompt": prompt,
                "system": system,
                "provider": provider,
                "model": model,
            },
        )

    def image(
        self,
        *,
        artifact_id: str,
        title: str,
        prompt: str,
        provider: str = "gemini",
        model: Optional[str] = None,
    ) -> dict[str, Any]:
        return self.client._request(
            "POST",
            "/native/generate/image",
            {
                "id": artifact_id,
                "title": title,
                "prompt": prompt,
                "provider": provider,
                "model": model,
            },
        )

    def embedding(
        self,
        *,
        artifact_id: str,
        title: str,
        input: list[str],
        provider: str = "openai",
        model: Optional[str] = None,
    ) -> dict[str, Any]:
        return self.client._request(
            "POST",
            "/native/generate/embedding",
            {
                "id": artifact_id,
                "title": title,
                "input": input,
                "provider": provider,
                "model": model,
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
        x_unit: Optional[str] = None,
        stack: str = "none",
        direction: str = "vertical",
        legend: Optional[str] = None,
        second_axis: Optional[dict[str, Any]] = None,
        fit: Optional[dict[str, Any]] = None,
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
                "xUnit": x_unit,
                "yLabel": y_label,
                "yUnit": y_unit,
                "stack": stack,
                "direction": direction,
                "legend": legend,
                "secondAxis": second_axis,
                "fit": fit,
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

    def timeline(
        self,
        *,
        artifact_id: str,
        title: str,
        lanes: list[dict[str, Any]],
        events: list[dict[str, Any]],
        export: Optional[list[str]] = None,
    ) -> dict[str, Any]:
        return self.client._request(
            "POST",
            "/native/ui/timeline",
            {
                "id": artifact_id,
                "title": title,
                "lanes": lanes,
                "events": events,
                "export": export or ["html"],
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


@dataclass(frozen=True)
class ExportApi:
    client: CapsemNativeClient

    def spreadsheet(self, artifact_id: str, *, format: str = "xlsx") -> dict[str, Any]:
        return self.client._request(
            "POST",
            "/native/export/spreadsheet",
            {"artifactId": artifact_id, "format": format},
        )

    def slide_deck(self, artifact_id: str, *, format: str = "pptx") -> dict[str, Any]:
        return self.client._request(
            "POST",
            "/native/export/slide-deck",
            {"artifactId": artifact_id, "format": format},
        )


def parse_native_artifact(value: Any) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise CapsemNativeError("native artifact must be an object")
    artifact_id = _require_string(value, "id")
    kind = _require_enum(value, "kind", list(NATIVE_ARTIFACT_KINDS))
    _require_string(value, "title")
    handle = _require_string(value, "handle")
    if not handle.startswith("capsem://artifact/"):
        raise CapsemNativeError("native artifact handle must use capsem://artifact/")
    spec = value.get("spec")
    if not isinstance(spec, dict):
        raise CapsemNativeError("native artifact spec must be an object")
    _validate_native_artifact_spec(artifact_id, kind, spec)
    return value


def _validate_native_artifact_spec(artifact_id: str, kind: str, spec: dict[str, Any]) -> None:
    if kind == "generatedText":
        _require_component(spec, NATIVE_ARTIFACT_COMPONENTS[kind])
        _require_media(spec, NATIVE_ARTIFACT_MEDIA[kind])
        _require_string(spec, "provider")
        _require_string(spec, "prompt")
        _require_string(spec, "status")
        return
    if kind == "generatedImage":
        _require_component(spec, NATIVE_ARTIFACT_COMPONENTS[kind])
        _require_media(spec, NATIVE_ARTIFACT_MEDIA[kind])
        _require_string(spec, "provider")
        _require_string(spec, "prompt")
        _require_string(spec, "status")
        return
    if kind == "generatedEmbedding":
        _require_component(spec, NATIVE_ARTIFACT_COMPONENTS[kind])
        _require_media(spec, NATIVE_ARTIFACT_MEDIA[kind])
        _require_string(spec, "provider")
        _require_string_array(spec, "input")
        _require_string(spec, "status")
        return
    if kind == "sheet":
        _require_component(spec, NATIVE_ARTIFACT_COMPONENTS[kind])
        _require_string_array(spec, "columns")
        _require_array(spec, "rows")
        return
    if kind == "table":
        _require_component(spec, NATIVE_ARTIFACT_COMPONENTS[kind])
        _require_string(spec, "sourceArtifact")
        _require_string_array(spec, "columns")
        _require_array(spec, "rows")
        if not isinstance(spec.get("pageSize"), int) or spec["pageSize"] < 1:
            raise CapsemNativeError(f"{artifact_id} table pageSize must be a positive integer")
        return
    if kind == "chart":
        _require_component(spec, NATIVE_ARTIFACT_COMPONENTS[kind])
        chart = _require_enum(spec, "chart", list(PLOTLY_CHART_KINDS))
        _require_string(spec, "sourceArtifact")
        _require_array(spec, "data")
        _require_string(spec, "x")
        _require_array(spec, "series")
        _require_string(spec, "xLabel")
        _require_string(spec, "yLabel")
        _require_string(spec, "yUnit")
        stack = spec.get("stack")
        if stack is not None:
            _require_enum(spec, "stack", list(PLOTLY_STACK_MODES))
        direction = spec.get("direction")
        if direction is not None:
            _require_enum(spec, "direction", list(PLOTLY_DIRECTIONS))
        if stack not in (None, "none") and chart != "barChart":
            raise CapsemNativeError(f"{chart} does not support stack mode {stack}")
        if direction == "horizontal" and chart != "barChart":
            raise CapsemNativeError(f"{chart} does not support horizontal direction")
        if spec.get("secondAxis") is not None and chart not in ("barChart", "lineChart", "scatterPlot"):
            raise CapsemNativeError(f"{chart} does not support secondAxis")
        if spec.get("fit") is not None and chart not in ("lineChart", "scatterPlot"):
            raise CapsemNativeError(f"{chart} does not support fit metadata")
        return
    if kind == "diagram":
        _require_component(spec, NATIVE_ARTIFACT_COMPONENTS[kind])
        _require_enum(spec, "kind", ["mermaid"])
        _require_string(spec, "source")
        return
    if kind == "timeline":
        _require_component(spec, NATIVE_ARTIFACT_COMPONENTS[kind])
        _require_array(spec, "lanes")
        _require_array(spec, "events")
        return
    if kind == "slide":
        _require_component(spec, NATIVE_ARTIFACT_COMPONENTS[kind])
        _require_array(spec, "blocks")
        return
    if kind == "slideDeck":
        _require_component(spec, NATIVE_ARTIFACT_COMPONENTS[kind])
        _require_array(spec, "slides")
        return


def _require_component(value: dict[str, Any], component: str) -> None:
    if value.get("component") != component:
        raise CapsemNativeError(f"artifact spec component must be {component}")


def _require_media(value: dict[str, Any], media: str) -> None:
    if value.get("media") != media:
        raise CapsemNativeError(f"artifact spec media must be {media}")


def _require_string(value: dict[str, Any], field: str) -> str:
    item = value.get(field)
    if not isinstance(item, str) or not item.strip():
        raise CapsemNativeError(f"{field} must be a non-empty string")
    return item


def _require_array(value: dict[str, Any], field: str) -> list[Any]:
    item = value.get(field)
    if not isinstance(item, list):
        raise CapsemNativeError(f"{field} must be a list")
    return item


def _require_string_array(value: dict[str, Any], field: str) -> list[str]:
    items = _require_array(value, field)
    for item in items:
        if not isinstance(item, str) or not item.strip():
            raise CapsemNativeError(f"{field} must contain only non-empty strings")
    return items


def _require_enum(value: dict[str, Any], field: str, allowed: list[str]) -> str:
    item = value.get(field)
    if not isinstance(item, str) or item not in allowed:
        raise CapsemNativeError(f"{field} must be one of {', '.join(allowed)}")
    return item
