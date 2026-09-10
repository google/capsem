"""Generate typed async fetch operations, including runtime wire validation."""

from __future__ import annotations

import json

from .operations import Route
from .schema import Schema
from .typescript import HEADER, type_name
from .typescript_validation import expression

RESERVED = frozenset({
    "break", "case", "catch", "class", "const", "continue", "debugger", "default", "delete",
    "do", "else", "enum", "export", "extends", "false", "finally", "for", "function", "if",
    "import", "in", "instanceof", "new", "null", "return", "super", "switch", "this", "throw",
    "true", "try", "typeof", "var", "void", "while", "with", "yield", "let", "static",
    "implements", "interface", "package", "private", "protected", "public", "await",
})


def render_operations(routes: list[Route]) -> dict[str, str]:
    files = {}
    exports = []
    for route in sorted(routes, key=lambda value: value.operation.operation_id):
        operation = route.operation
        name = operation.operation_id
        if name in RESERVED:
            raise ValueError(f"unsupported operation identifier: {name}")
        if name + ".ts" in files:
            raise ValueError(f"colliding operation identifier: {name}")
        properties = {parameter.name: parameter.schema_ for parameter in operation.parameters}
        required = [parameter.name for parameter in operation.parameters if parameter.required]
        if operation.request_body:
            if "body" in properties:
                raise ValueError("request body collides with parameter")
            properties["body"] = operation.request_body.schema
            required.append("body")
        response = operation.responses["200"]
        binary = response.media_type == "application/octet-stream"
        schemas = [*properties.values(), response.schema]
        arguments = ""
        if properties:
            fields = [f"    {json.dumps(key)}{'' if key in required else '?'}: {type_name(schema)};"
                      for key, schema in properties.items()]
            arguments = "  parameters: {\n" + "\n".join(fields) + "\n  }" + ("" if required else " = {}") + ",\n"
        lines = [f"export async function {name}(\n  transport: Transport,\n{arguments}  options: CallOptions = {{}},\n): Promise<{type_name(response.schema)}> {{"]
        if properties:
            validator = expression(Schema(type="object", properties=properties, required=required))
            lines.append(f"  const input = {validator}.parse(parameters);")
        lines += [f"  {'return' if binary else 'const payload ='} await transport.request(Method.{route.method.name}, {json.dumps(route.path)}, {{",
                  f"    signal: options.signal, accept: MediaType.{'BINARY' if binary else 'JSON'},"]
        for location, keyword in (("path", "parameters"), ("query", "query")):
            fields = [f"{json.dumps(p.name)}: input[{json.dumps(p.name)}]"
                      for p in operation.parameters if p.location == location]
            if fields:
                lines.append(f"    {keyword}: {{{', '.join(fields)}}},")
        if operation.request_body:
            body_binary = operation.request_body.media_type == "application/octet-stream"
            value = "input.body" if body_binary else "JSON.stringify(input.body)"
            lines.append(f"    body: {value}, contentType: MediaType.{'BINARY' if body_binary else 'JSON'},")
        lines.append("  });")
        if not binary:
            lines.append(f"  return {expression(response.schema)}.parse(JSON.parse(new TextDecoder().decode(payload)));")
        lines.append("}")
        imports = ['import {z} from "zod";', 'import {Transport, Method, MediaType, type CallOptions} from "../transport.js";']
        dependencies = sorted({ref for schema in schemas for ref in schema.references()})
        imports += [f'import type {{{dep}}} from "../models/{dep}.js";' for dep in dependencies]
        imports += [f'import {{{dep}Schema}} from "../validation/{dep}.js";' for dep in dependencies]
        files[name + ".ts"] = HEADER + "\n".join(imports) + "\n\n" + "\n".join(lines) + "\n"
        exports.append(f'export {{{name}}} from "./{name}.js";')
    files["index.ts"] = HEADER + "\n".join(exports) + "\n"
    return files
