"""Render small typed Python modules, without per-endpoint boilerplate."""

from __future__ import annotations

import keyword
import re

from .schema import Schema, schema_order

HEADER = '"""Generated from Capsem OpenAPI. Do not edit."""\n\n'
BASE = '''"""Shared validation for JSON values and optional nonnullable fields."""

from __future__ import annotations

from math import isfinite
from typing import Annotated, ClassVar, TypeAlias

from pydantic import (
    AfterValidator,
    BaseModel,
    ConfigDict,
    ValidationError,
    ValidatorFunctionWrapHandler,
    model_validator,
)
from pydantic import JsonValue as PydanticJsonValue


def _finite_json(value: PydanticJsonValue) -> PydanticJsonValue:
    pending = [value]
    while pending:
        item = pending.pop()
        if isinstance(item, float) and not isfinite(item):
            raise ValueError("JSON numbers must be finite")
        if isinstance(item, dict):
            pending.extend(item.values())
        elif isinstance(item, list):
            pending.extend(item)
    return value


JsonValue: TypeAlias = Annotated[PydanticJsonValue, AfterValidator(_finite_json)]


class Model(BaseModel):
    model_config = ConfigDict(strict=True, populate_by_name=True)
    nonnullable_optional: ClassVar[frozenset[str]] = frozenset()
    private_input: ClassVar[bool] = False

    @model_validator(mode="wrap")
    @classmethod
    def redact_private_validation(cls, value: object, handler: ValidatorFunctionWrapHandler) -> object:
        try:
            return handler(value)
        except ValidationError:
            if not cls.private_input:
                raise
            # Replace the input and field locations as well as the rendered
            # message: callers can inspect errors(), not only str(error).
            raise ValidationError.from_exception_data(cls.__name__, [{
                "type": "value_error", "loc": (), "input": None,
                "ctx": {"error": ValueError("invalid private request")},
            }], hide_input=True) from None

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
    return (not (schema.model_fields_set - {"description"})
            or (isinstance(schema.type, list) and "null" in schema.type)
            or schema.type == "null"
            or any(nullable(member) for member in schema.one_of or []))


def type_name(schema: Schema) -> str:
    if not (schema.model_fields_set - {"description"}):
        return "JsonValue"
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
    elif schema.format == "ipv4":
        primitive = "IPv4Address"
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
    if any(field.write_only for field in schema.properties.values()):
        lines.append("    private_input = True")
    optional = [alias for key, field in schema.properties.items()
                if key not in schema.required and not nullable(field)
                for alias in ((key, key + "_") if keyword.iskeyword(key) else (key,))]
    if optional:
        lines.append(f"    nonnullable_optional = frozenset({optional!r})")
    if schema.additional_properties is False:
        privacy = ", hide_input_in_errors=True" if any(field.write_only for field in schema.properties.values()) else ""
        lines.append(f'    model_config = ConfigDict(strict=True, populate_by_name=True, extra="forbid"{privacy})')
    for key, field in schema.properties.items():
        attr = key + "_" if keyword.iskeyword(key) else key
        if not attr.isidentifier() or attr.startswith("_") or attr in {"nonnullable_optional", "private_input"}:
            raise ValueError(f"unsupported field identifier: {key}")
        annotation = type_name(field)
        default = ""
        if key not in schema.required:
            if not nullable(field):
                annotation += " | None"
            default = " = None"
        if attr != key:
            default = f" = Field({('default=None, ' if default else '')}alias={key!r})"
        if field.write_only:
            default = f" = Field({('default=None, ' if key not in schema.required else '')}{('alias=' + repr(key) + ', ' if attr != key else '')}repr=False)"
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
        if "IPv4Address" in body:
            imports.append("from ipaddress import IPv4Address")
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
        base_names = (["JsonValue"] if re.search(r"\bJsonValue\b", body) else [])
        if "(Model)" in body:
            base_names.append("Model")
        if base_names:
            local.append(f"from .model_base import {', '.join(base_names)}")
        imports += sorted(local)
        source = HEADER + "from __future__ import annotations\n\n"
        spacing = "\n\n\n" if body.startswith("class ") else "\n\n"
        source += "\n".join(imports).strip() + spacing + body
        filename = module_name(name) + ".py"
        if filename in files:
            raise ValueError(f"colliding schema module: {filename}")
        files[filename] = source
        exported = f"from .{module_name(name)} import {name} as {name}"
        if len(exported) > 88:
            exported = f"from .{module_name(name)} import (\n    {name} as {name},\n)"
        exports.append(exported)
    files["__init__.py"] = HEADER + "\n".join(exports) + "\n"
    return files
