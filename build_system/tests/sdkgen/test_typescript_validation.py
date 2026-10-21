"""Runtime TypeScript validation retains the schema's wire constraints."""

from capsem_builder.sdkgen.schema import Schema
from capsem_builder.sdkgen.typescript_validation import expression, render_validators


def test_nullable_and_optional_are_separate_validation_operations() -> None:
    schema = Schema.model_validate({"type": "object", "properties": {
        "nullable": {"type": ["string", "null"]}, "optional": {"type": "boolean"},
    }, "required": ["nullable"]})
    source = render_validators({"Example": schema})["Example.ts"]
    assert '"nullable": z.string().nullable()' in source
    assert '"optional": z.boolean().exactOptional()' in source


def test_numeric_constraints_and_closed_objects_survive() -> None:
    assert expression(Schema.model_validate({"type": "integer", "minimum": 0})) == "z.int().min(0)"
    assert expression(Schema.model_validate({"type": "object", "additionalProperties": False})) == "z.strictObject({})"
    assert expression(Schema.model_validate({"type": "string", "format": "ipv4"})) == "z.ipv4()"


def test_connection_timestamps_and_safe_revision_are_validated() -> None:
    assert expression(Schema.model_validate({"type": ["string", "null"], "format": "date-time"})) == (
        "z.iso.datetime({offset: true}).nullable()"
    )
    assert expression(Schema.model_validate({
        "type": "integer", "minimum": 0, "maximum": 9007199254740991,
    })) == "z.int().min(0).max(9007199254740991)"


def test_recursive_references_are_lazy_and_keep_named_types() -> None:
    schema = Schema.model_validate({"type": "object", "properties": {
        "child": {"$ref": "#/components/schemas/Tree"},
    }})
    source = render_validators({"Tree": schema})["Tree.ts"]
    assert "TreeSchema: z.ZodType<Tree>" in source
    assert 'z.lazy(() => TreeSchema)' in source
    assert 'from "./Tree.js"' not in source


def test_fixed_private_token_pattern_and_uuid_format_survive_generation() -> None:
    assert expression(Schema.model_validate({"type": "string", "pattern": "^[0-9a-f]{64}$"})) == (
        'z.string().length(64).regex(new RegExp("^[0-9a-f]{64}$"))'
    )
    assert expression(Schema.model_validate({"type": "string", "format": "uuid"})) == "z.uuid()"


def test_text_and_array_budgets_survive_generation() -> None:
    assert expression(Schema.model_validate({"type": "string", "maxLength": 1024})) == "z.string().max(1024)"
    assert expression(Schema.model_validate({
        "type": "array", "items": {"type": "boolean"}, "maxItems": 32,
    })) == "z.array(z.boolean()).max(32)"
