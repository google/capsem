"""Render small typed Python modules, without per-endpoint boilerplate."""

from __future__ import annotations

import keyword
import re

from .schema import Schema, schema_order

HEADER = '"""Generated from Capsem OpenAPI. Do not edit."""\n\n'
BASE = '''"""Shared validation for optional fields that do not permit JSON null."""

from __future__ import annotations

from typing import ClassVar

from pydantic import BaseModel, ConfigDict, model_validator


class Model(BaseModel):
    model_config = ConfigDict(strict=True, populate_by_name=True)
    nonnullable_optional: ClassVar[frozenset[str]] = frozenset()

    @model_validator(mode="before")
    @classmethod
    def reject_explicit_null(cls, value: object) -> object:
        if isinstance(value, dict):
            for key in cls.nonnullable_optional:
                if key in value and value[key] is None:
                    raise ValueError(f"{key} may be omitted but cannot be null")
        return value
'''


def module_name(name: str) -> str:
    return re.sub(r"(?<!^)(?=[A-Z])", "_", name).lower()


def nullable(schema: Schema) -> bool:
    return ((isinstance(schema.type, list) and "null" in schema.type)
            or schema.type == "null"
            or any(nullable(member) for member in schema.one_of or []))


def type_name(schema: Schema) -> str:
    if schema.ref is not None:
        return schema.ref.rsplit("/", 1)[1]
    if schema.one_of is not None:
        members = [type_name(member) for member in schema.one_of]
        return " | ".join(sorted(members, key=lambda member: member == "None"))
    if isinstance(schema.type, list):
        kind = next(kind for kind in schema.type if kind != "null")
        return f"{type_name(schema.model_copy(update={'type': kind}))} | None"
    if schema.enum is not None:
        raise ValueError("inline enums must become named OpenAPI schemas")
    if schema.type == "object":
        if not isinstance(schema.additional_properties, Schema) or schema.properties:
            raise ValueError("inline objects must be named models or typed maps")
        return f"dict[str, {type_name(schema.additional_properties)}]"
    if schema.type == "array" and schema.items is not None:
        return f"list[{type_name(schema.items)}]"
    if schema.type is None or schema.type == "array":
        raise ValueError("missing type")
    primitive = {"string": "StrictStr", "integer": "StrictInt", "number": "StrictFloat",
                 "boolean": "StrictBool", "null": "None"}[schema.type]
    if schema.format == "binary":
        primitive = "bytes"
    if schema.minimum is not None:
        primitive = f"Annotated[{primitive}, Field(ge={schema.minimum!r})]"
    return primitive


def _body(name: str, schema: Schema) -> str:
    if schema.enum is not None:
        members = [re.sub(r"\W", "_", value).upper() for value in schema.enum]
        if len(set(members)) != len(members) or not all(member.isidentifier() for member in members):
            raise ValueError(f"enum {name} has colliding or invalid Python names")
        fields = [f"    {member} = {value!r}" for member, value in zip(members, schema.enum, strict=True)]
        return f"class {name}(StrEnum):\n" + "\n".join(fields) + "\n"
    if schema.type != "object":
        return f"{name}: TypeAlias = {type_name(schema)}\n"
    lines = [f"class {name}(Model):"]
    optional = [alias for key, field in schema.properties.items()
                if key not in schema.required and not nullable(field)
                for alias in ((key, key + "_") if keyword.iskeyword(key) else (key,))]
    if optional:
        lines.append(f"    nonnullable_optional = frozenset({optional!r})")
    if schema.additional_properties is False:
        lines.append('    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid")')
    for key, field in schema.properties.items():
        attr = key + "_" if keyword.iskeyword(key) else key
        if not attr.isidentifier() or attr.startswith("_") or attr == "nonnullable_optional":
            raise ValueError(f"unsupported field identifier: {key}")
        annotation = type_name(field)
        default = ""
        if key not in schema.required:
            if not nullable(field):
                annotation += " | None"
            default = " = None"
        if attr != key:
            default = f" = Field({('default=None, ' if default else '')}alias={key!r})"
        lines.append(f"    {attr}: {annotation}{default}")
    return "\n".join(lines + (["    pass"] if len(lines) == 1 else [])) + "\n"


def render_models(schemas: dict[str, Schema]) -> dict[str, str]:
    schema_order(schemas)
    files = {"model_base.py": BASE}
    exports = []
    for name, schema in sorted(schemas.items()):
        if not name.isidentifier() or keyword.iskeyword(name):
            raise ValueError(f"invalid schema identifier: {name}")
        body = _body(name, schema)
        imports = []
        if "(StrEnum)" in body:
            imports.append("from enum import StrEnum")
        typing = [symbol for symbol in ("Annotated", "TypeAlias") if symbol in body]
        if typing:
            imports.append(f"from typing import {', '.join(typing)}")
        pydantic = [symbol for symbol in (
            "ConfigDict", "Field", "StrictBool", "StrictFloat", "StrictInt", "StrictStr",
        ) if re.search(rf"\b{symbol}\b", body)]
        if pydantic:
            imports += ["", f"from pydantic import {', '.join(pydantic)}"]
        imports.append("")
        dependencies = schema.references() - {name}
        local = [f"from .{module_name(dep)} import {dep}" for dep in sorted(dependencies)]
        if "(Model)" in body:
            local.append("from .model_base import Model")
        imports += sorted(local)
        source = HEADER + "from __future__ import annotations\n\n"
        spacing = "\n\n\n" if body.startswith("class ") else "\n\n"
        source += "\n".join(imports).strip() + spacing + body
        filename = module_name(name) + ".py"
        if filename in files:
            raise ValueError(f"colliding schema module: {filename}")
        files[filename] = source
        exported = f"from .{module_name(name)} import {name} as {name}"
        if len(exported) > 100:
            exported = f"from .{module_name(name)} import (\n    {name} as {name},\n)"
        exports.append(exported)
    files["__init__.py"] = HEADER + "\n".join(exports) + "\n"
    return files
