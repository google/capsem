"""Generate small async HTTP functions from the gateway operation contract."""

from __future__ import annotations

import keyword
import re

from .operations import Route
from .python import HEADER, module_name, type_name


def render_operations(routes: list[Route]) -> dict[str, str]:
    files: dict[str, str] = {}
    exports = []
    for route in routes:
        operation = route.operation
        name = module_name(operation.operation_id)
        if keyword.iskeyword(name):
            raise ValueError(f"unsupported operation identifier: {name}")
        response = operation.responses["200"]
        binary = response.media_type == "application/octet-stream"
        schemas = [parameter.schema_ for parameter in operation.parameters] + [response.schema]
        arguments = []
        validation = []
        parameters: dict[str, list[str]] = {"path": [], "query": []}
        for parameter in operation.parameters:
            attr = parameter.name + ("_" if keyword.iskeyword(parameter.name) else "")
            if not attr.isidentifier() or attr in {"transport", "body", "payload"}:
                raise ValueError(f"unsupported operation parameter: {attr}")
            annotation = type_name(parameter.schema_)
            default = ""
            if not parameter.required:
                annotation += " | None"
                default = " = None"
            arguments.append(f"    {attr}: {annotation}{default},")
            validation.append(f"    {attr} = TypeAdapter({annotation}).validate_python({attr})")
            parameters[parameter.location].append(f"{parameter.name!r}: {attr}")
        if operation.request_body is not None:
            schemas.append(operation.request_body.schema)
            arguments.append(f"    body: {type_name(operation.request_body.schema)},")
        signature = [f"async def {name}(", "    transport: Transport,"]
        if arguments:
            signature += ["    *,", *arguments]
        lines = [*signature, f") -> {type_name(response.schema)}:", *validation]
        lines += [f"    {'return' if binary else 'payload ='} await transport.request(", f"        Method.{route.method.name}, {route.path!r},"]
        for location, keyword_arg in (("path", "path_parameters"), ("query", "query")):
            if parameters[location]:
                lines.append(f"        {keyword_arg}={{{', '.join(parameters[location])}}},")
        if operation.request_body is not None:
            lines.append("        body=body,")
        if binary:
            lines.append("        accept=MediaType.BINARY,")
        lines.append("    )")
        if not binary:
            lines.append(f"    return TypeAdapter({type_name(response.schema)}).validate_json(payload)")
        body = "\n".join(lines) + "\n"
        imports = []
        if "Annotated[" in body:
            imports += ["from typing import Annotated", ""]
        symbols = [symbol for symbol in ("Field", "StrictBool", "StrictFloat", "StrictInt", "StrictStr", "TypeAdapter")
                   if re.search(rf"\b{symbol}\b", body)]
        if symbols:
            imports += [f"from pydantic import {', '.join(symbols)}", ""]
        imports += [f"from .._transport import {'MediaType, ' if binary else ''}Method, Transport"]
        dependencies = sorted({ref for schema in schemas for ref in schema.references()})
        imports += [f"from ..models.{module_name(dep)} import {dep}" for dep in dependencies]
        if name + ".py" in files:
            raise ValueError(f"colliding operation module: {name}")
        files[name + ".py"] = HEADER + "from __future__ import annotations\n\n" + "\n".join(imports) + "\n\n\n" + body
        exports.append(f"from .{name} import {name} as {name}")
    files["__init__.py"] = HEADER + "\n".join(sorted(exports)) + "\n"
    return files
