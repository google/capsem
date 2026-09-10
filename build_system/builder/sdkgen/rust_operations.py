"""Typed Rust operations reuse the gateway DTOs instead of generating duplicate models."""

from __future__ import annotations

import json

from .operations import Route
from .python import module_name
from .schema import Schema

HEADER = "// Generated from sdk/specification/openapi.json. Do not edit.\n"
RESERVED = frozenset(["as", "async", "await", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern", "false", "fn", "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "pub", "ref", "return", "self", "Self", "static", "struct", "super", "trait", "true", "type", "unsafe", "use", "where", "while", "abstract", "become", "box", "do", "final", "macro", "override", "priv", "typeof", "unsized", "virtual", "yield", "try", "gen"])


def type_name(schema: Schema) -> str:
    if schema.ref:
        return "capsem_api::" + schema.ref.rsplit("/", 1)[1]
    if schema.type == "array" and schema.items:
        return f"Vec<{type_name(schema.items)}>"
    if schema.type == "integer" and schema.minimum in (None, 0):
        return ("u" if schema.minimum == 0 else "i") + ("32" if schema.format == "int32" else "64")
    if schema.type == "string" and not schema.enum:
        return "Vec<u8>" if schema.format == "binary" else "String"
    if schema.type == "boolean":
        return "bool"
    raise ValueError(f"unsupported Rust operation schema: {schema}")


def wire_value(schema: Schema, value: str) -> str:
    if schema.ref:
        return f"crate::operations::enum_value({value})?"
    if schema.type == "array" and schema.items and schema.items.ref:
        return f"crate::operations::enum_values({value})?"
    if schema.type in ("string", "integer", "boolean"):
        return f"{value.removeprefix('&')}.to_string()"
    raise ValueError("unsupported Rust wire parameter")


def render_operations(routes: list[Route]) -> dict[str, str]:
    files: dict[str, str] = {}
    exports = []
    for route in sorted(routes, key=lambda value: value.operation.operation_id):
        op = route.operation
        name = module_name(op.operation_id)
        if name in RESERVED or not name.isidentifier() or name + ".rs" in files:
            raise ValueError(f"invalid or colliding Rust operation: {name}")
        properties = {p.name: p.schema_ for p in op.parameters}
        required = {p.name for p in op.parameters if p.required}
        if op.request_body:
            if "body" in properties:
                raise ValueError("request body collides with parameter")
            properties["body"] = op.request_body.schema
            required.add("body")
        if any(not key.isidentifier() or key in RESERVED for key in properties):
            raise ValueError("unsupported Rust parameter identifier")
        params = op.operation_id[0].upper() + op.operation_id[1:] + "Params"
        lines = ["use crate::transport::{CallOptions, Request, Transport};", ""]
        if properties:
            lines += ["#[derive(Debug, Clone, serde::Deserialize)]", f"pub struct {params} {{"]
            for key, schema in properties.items():
                kind = type_name(schema)
                lines.append(f"    pub {key}: {kind if key in required else f'Option<{kind}>'},")
            lines += ["}", ""]
        result = type_name(op.responses["200"].schema)
        arguments = ["transport: &Transport"]
        if properties:
            arguments.append(f"input: &{params}")
        arguments.append("options: CallOptions")
        signature = f"pub async fn {name}({', '.join(arguments)}) -> crate::Result<{result}> {{"
        lines += ([signature] if len(signature) <= 120 else [f"pub async fn {name}(",
                  *(f"    {arg}," for arg in arguments), f") -> crate::Result<{result}> {{"])
        query = [p for p in op.parameters if p.location == "query"]
        if query:
            pairs = ", ".join(f"({json.dumps(p.name)}, {wire_value(p.schema_, '&input.' + p.name)})"
                              for p in query if p.required)
            optional = any(not p.required for p in query)
            lines.append(f"    let {'mut ' if optional else ''}query = {'vec!' if optional else ''}[{pairs}];")
        for p in (p for p in query if not p.required):
            lines += [f"    if let Some(value) = &input.{p.name} {{",
                      f"        query.push(({json.dumps(p.name)}, {wire_value(p.schema_, 'value')}));", "    }"]
        path = [p for p in op.parameters if p.location == "path"]
        for p in path:
            lines.append(f"    let path_{p.name} = {wire_value(p.schema_, '&input.' + p.name)};")
        lines += ["    let request = Request {"]
        if path:
            pairs = ", ".join(f'({json.dumps(p.name)}, path_{p.name}.as_str())' for p in path)
            lines.append(f"        parameters: &[{pairs}],")
        if query:
            lines.append("        query: &query,")
        if op.request_body:
            binary = op.request_body.media_type == "application/octet-stream"
            lines.append(f"        body: Some({'input.body.clone()' if binary else 'serde_json::to_vec(&input.body)?'}),")
            if binary:
                lines.append("        content_type: crate::transport::MediaType::Binary,")
        binary = op.responses["200"].media_type == "application/octet-stream"
        if binary:
            lines.append("        accept: crate::transport::MediaType::Binary,")
        lines += ["        options,", "        ..Default::default()", "    };"]
        method = f".request(reqwest::Method::{route.method.name}, {json.dumps(route.path)}, request)"
        suffix = ".await" if binary else ".await?"
        call = "transport" + method + suffix
        if len(call) > 72:
            call = f"transport\n        {method}\n        {suffix}"
        lines.append(f"    {call}" if binary else f"    let bytes = {call};")
        if not binary:
            lines.append("    Ok(serde_json::from_slice(&bytes)?)")
        lines.append("}")
        files[name + ".rs"] = HEADER + "\n".join(lines) + "\n"
        exports += [f"mod {name};", f"pub use {name}::{{{name}, {params}}};" if properties else f"pub use {name}::{name};"]
    files["mod.rs"] = HEADER + "\n".join(exports) + "\n"
    return files
