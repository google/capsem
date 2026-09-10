"""Render runtime validators that compile against the public TypeScript models."""

from __future__ import annotations

import json

from .schema import Schema, schema_order
from .typescript import HEADER, render_models


def expression(schema: Schema) -> str:
    if not (schema.model_fields_set - {"description"}):
        return "z.json()"
    if schema.ref is not None:
        return f"z.lazy(() => {schema.ref.rsplit('/', 1)[1]}Schema)"
    if schema.one_of is not None:
        return "z.union([" + ", ".join(expression(member) for member in schema.one_of) + "])"
    if isinstance(schema.type, list):
        kind = next(kind for kind in schema.type if kind != "null")
        return expression(schema.model_copy(update={"type": kind})) + ".nullable()"
    if schema.enum is not None:
        raise ValueError("inline enums must become named OpenAPI schemas")
    if schema.type == "object":
        if isinstance(schema.additional_properties, Schema):
            return f"z.record(z.string(), {expression(schema.additional_properties)})"
        constructor = "z.strictObject" if schema.additional_properties is False else "z.object"
        fields = [f"{json.dumps(key)}: {expression(field)}{'' if key in schema.required else '.exactOptional()'}"
                  for key, field in schema.properties.items()]
        return constructor + "({" + ("\n  " + ",\n  ".join(fields) + ",\n" if fields else "") + "})"
    if schema.type == "array" and schema.items is not None:
        return f"z.array({expression(schema.items)})"
    if schema.type is None or schema.type == "array":
        raise ValueError("missing type")
    if schema.format == "binary":
        return "z.instanceof(Uint8Array)"
    result = {"string": "z.string()", "integer": "z.int()", "number": "z.number()",
              "boolean": "z.boolean()", "null": "z.null()"}[schema.type]
    return result + (f".min({schema.minimum})" if schema.minimum is not None else "")


def render_validators(schemas: dict[str, Schema]) -> dict[str, str]:
    schema_order(schemas)
    render_models(schemas)  # Apply the same exported identifier and enum checks.
    files = {}
    exports = []
    for name, schema in sorted(schemas.items()):
        qualifier = "" if schema.enum is not None else "type "
        imports = ['import {z} from "zod";', f'import {qualifier}{{{name}}} from "../models/{name}.js";']
        imports += [f'import {{{dep}Schema}} from "./{dep}.js";'
                    for dep in sorted(schema.references() - {name})]
        body = f"z.enum({name})" if schema.enum is not None else expression(schema)
        files[name + ".ts"] = HEADER + "\n".join(imports) + f"\n\nexport const {name}Schema: z.ZodType<{name}> = {body};\n"
        exports.append(f'export {{{name}Schema}} from "./{name}.js";')
    files["index.ts"] = HEADER + "\n".join(exports) + "\n"
    return files
