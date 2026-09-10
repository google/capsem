"""Render TypeScript interfaces and runtime enums directly from wire schemas."""

from __future__ import annotations

import json
import re

from .schema import Schema, schema_order

HEADER = "// Generated from Capsem OpenAPI. Do not edit.\n\n"


def type_name(schema: Schema) -> str:
    if schema.ref is not None:
        return schema.ref.rsplit("/", 1)[1]
    if schema.one_of is not None:
        return " | ".join(type_name(member) for member in schema.one_of)
    if isinstance(schema.type, list):
        kind = next(kind for kind in schema.type if kind != "null")
        return f"{type_name(schema.model_copy(update={'type': kind}))} | null"
    if schema.enum is not None:
        raise ValueError("inline enums must become named OpenAPI schemas")
    if schema.type == "object":
        if not isinstance(schema.additional_properties, Schema) or schema.properties:
            raise ValueError("inline objects must be named models or typed maps")
        return f"Record<string, {type_name(schema.additional_properties)}>"
    if schema.type == "array" and schema.items is not None:
        return f"Array<{type_name(schema.items)}>"
    if schema.type is None or schema.type == "array":
        raise ValueError("missing type")
    if schema.format == "binary":
        return "Uint8Array"
    return {"string": "string", "integer": "number", "number": "number",
            "boolean": "boolean", "null": "null"}[schema.type]


def _body(name: str, schema: Schema) -> str:
    if schema.enum is not None:
        members = [re.sub(r"\W", "_", value).upper() for value in schema.enum]
        if len(set(members)) != len(members) or not all(re.fullmatch(r"[A-Z_]\w*", m) for m in members):
            raise ValueError(f"enum {name} has colliding or invalid TypeScript names")
        fields = [f"  {member} = {json.dumps(value)},"
                  for member, value in zip(members, schema.enum, strict=True)]
        return f"export enum {name} {{\n" + "\n".join(fields) + "\n}\n"
    if schema.type != "object" or isinstance(schema.additional_properties, Schema):
        return f"export type {name} = {type_name(schema)};\n"
    fields = [f"  {json.dumps(key)}{'' if key in schema.required else '?'}: {type_name(field)};"
              for key, field in schema.properties.items()]
    return f"export interface {name} {{\n" + "\n".join(fields) + "\n}\n"


def render_models(schemas: dict[str, Schema]) -> dict[str, str]:
    schema_order(schemas)
    files: dict[str, str] = {}
    exports = []
    for name, schema in sorted(schemas.items()):
        if not re.fullmatch(r"[A-Z][A-Za-z0-9]*", name):
            raise ValueError(f"invalid schema identifier: {name}")
        imports = [f'import type {{ {dep} }} from "./{dep}.js";'
                   for dep in sorted(schema.references() - {name})]
        files[name + ".ts"] = HEADER + "\n".join(imports) + "\n\n" + _body(name, schema)
        qualifier = "" if schema.enum is not None else "type "
        exports.append(f'export {qualifier}{{ {name} }} from "./{name}.js";')
    files["index.ts"] = HEADER + "\n".join(exports) + "\n"
    return files
