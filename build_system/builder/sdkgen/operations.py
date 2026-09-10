"""Read the gateway's HTTP operations without silently dropping wire semantics."""

from __future__ import annotations

import json
import re
from dataclasses import dataclass
from enum import StrEnum
from pathlib import Path
from typing import Literal

from pydantic import BaseModel, ConfigDict, Field

from .schema import Schema, read_schemas

MediaType = Literal["application/json", "application/octet-stream", "text/plain"]


class Strict(BaseModel):
    model_config = ConfigDict(extra="forbid", strict=True, populate_by_name=True)


class Media(Strict):
    schema_: Schema = Field(alias="schema")


class Content(Strict):
    content: dict[MediaType, Media]

    @property
    def media_type(self) -> MediaType:
        if len(self.content) != 1:
            raise ValueError("operation requires one unambiguous content type")
        return next(iter(self.content))

    @property
    def schema(self) -> Schema:
        return self.content[self.media_type].schema_


class RequestBody(Content):
    required: Literal[True]


class Response(Content):
    description: str

    @property
    def media_type(self) -> MediaType:
        # Logs also serve plain text to browsers. SDKs explicitly request JSON.
        if (set(self.content) == {"application/json", "text/plain"}
                and self.content["text/plain"].schema_.type == "string"):
            return "application/json"
        return super().media_type


class Parameter(Strict):
    name: str
    location: Literal["path", "query"] = Field(alias="in")
    required: bool
    schema_: Schema = Field(alias="schema")
    description: str | None = None
    style: Literal["form"] | None = None
    explode: Literal[False] | None = None


class Operation(Strict):
    operation_id: str = Field(alias="operationId", pattern=r"^[a-z][A-Za-z0-9]+$")
    description: str | None = None
    parameters: list[Parameter] = Field(default_factory=list)
    request_body: RequestBody | None = Field(default=None, alias="requestBody")
    responses: dict[Literal["200", "202", "default"], Response]

    @property
    def success_status(self) -> Literal["200", "202"]:
        return "200" if "200" in self.responses else "202"

    @property
    def success(self) -> Response:
        return self.responses[self.success_status]


class Method(StrEnum):
    GET = "GET"
    POST = "POST"
    DELETE = "DELETE"


@dataclass(frozen=True)
class Route:
    path: str
    method: Method
    operation: Operation


def _validate(route: Route, names: set[str]) -> None:
    operation = route.operation
    parameters = operation.parameters
    if len({p.name for p in parameters}) != len(parameters):
        raise ValueError("duplicate operation parameter names")
    placeholders = set(re.findall(r"\{([^}]+)\}", route.path))
    path_parameters = {p.name for p in parameters if p.location == "path"}
    if placeholders != path_parameters or any(p.location == "path" and not p.required for p in parameters):
        raise ValueError("path placeholders must match required path parameters")
    if not route.path.startswith("/") or "?" in route.path or "#" in route.path:
        raise ValueError("operation path must be absolute without query or fragment")
    schemas = [p.schema_ for p in parameters]
    for parameter in parameters:
        if parameter.schema_.type == "array" and (
            parameter.location != "query" or parameter.style != "form" or parameter.explode is not False
        ):
            raise ValueError("array parameters require explicit comma-separated form encoding")
    if set(operation.responses) not in ({"200", "default"}, {"202", "default"}):
        raise ValueError("operation requires typed success and default error responses")
    success = operation.success
    if success.media_type not in ("application/json", "application/octet-stream"):
        raise ValueError("success response must be JSON or binary")
    schemas.append(success.schema)
    if operation.request_body is not None:
        if operation.request_body.media_type not in ("application/json", "application/octet-stream"):
            raise ValueError("request body must be JSON or binary")
        schemas.append(operation.request_body.schema)
    schemas.extend(media.schema_ for media in operation.responses["default"].content.values())
    missing = {name for schema in schemas for name in schema.references()} - names
    if missing:
        raise ValueError(f"missing operation schema references: {sorted(missing)}")


def read_operations(path: Path) -> list[Route]:
    document = json.loads(path.read_text())
    schemes = document["components"].get("securitySchemes")
    if (document.get("security") != [{"bearerAuth": []}]
            or schemes != {"bearerAuth": {"type": "http", "scheme": "bearer"}}):
        raise ValueError("SDK operations require the shared bearer authentication contract")
    names = set(read_schemas(path))
    routes: list[Route] = []
    ids: set[str] = set()
    for route_path, methods in sorted(document["paths"].items()):
        for method, value in sorted(methods.items()):
            operation = Operation.model_validate(value)
            route = Route(route_path, Method(method.upper()), operation)
            _validate(route, names)
            if operation.operation_id in ids:
                raise ValueError(f"duplicate operation id: {operation.operation_id}")
            ids.add(operation.operation_id)
            routes.append(route)
    if not routes:
        raise ValueError("OpenAPI document contains no operations")
    return routes
