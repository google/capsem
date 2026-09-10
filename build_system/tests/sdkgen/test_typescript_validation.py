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


def test_recursive_references_are_lazy_and_keep_named_types() -> None:
    schema = Schema.model_validate({"type": "object", "properties": {
        "child": {"$ref": "#/components/schemas/Tree"},
    }})
    source = render_validators({"Tree": schema})["Tree.ts"]
    assert "TreeSchema: z.ZodType<Tree>" in source
    assert 'z.lazy(() => TreeSchema)' in source
    assert 'from "./Tree.js"' not in source
