"""Write-only credential fields serialize for transport but never appear in repr."""
from importlib import import_module

import pytest
from capsem_builder.sdkgen.generation import synchronize
from capsem_builder.sdkgen.python import render_models
from capsem_builder.sdkgen.schema import Schema
from pydantic import ValidationError


def test_write_only_request_hides_repr_and_validation_input(tmp_path, monkeypatch):
    schema = Schema.model_validate({"type": "object", "additionalProperties": False,
        "required": ["value"], "properties": {"value": {"type": "string", "writeOnly": True}}})
    synchronize(tmp_path / "private_fixture", {"": render_models({"PrivateRequest": schema})}, check=False)
    monkeypatch.syspath_prepend(str(tmp_path))
    model = import_module("private_fixture.private_request").PrivateRequest
    assert model.model_config.get("hide_input_in_errors") is True
    request = model(value="private-secret-token")
    assert "private-secret-token" not in repr(request)
    assert request.model_dump(mode="json") == {"value": "private-secret-token"}
    with pytest.raises(ValidationError) as failure:
        model(value=["private-secret-token"])
    assert "private-secret-token" not in str(failure.value)
    assert "private-secret-token" not in repr(failure.value.errors(include_input=True))


def test_ordinary_request_does_not_gain_secret_field_handling():
    schema = Schema.model_validate({"type": "object", "required": ["value"],
        "properties": {"value": {"type": "string"}}})
    source = render_models({"PublicRequest": schema})["public_request.py"]
    assert "repr=False" not in source


def test_managed_pattern_and_nested_validation_never_expose_private_input(tmp_path, monkeypatch):
    schemas = {
        "Managed": Schema.model_validate({"type": "object", "additionalProperties": False,
            "required": ["request_id", "ownership_token"], "properties": {
                "request_id": {"type": "string", "format": "uuid"},
                "ownership_token": {"type": "string", "pattern": "^[0-9a-f]{64}$", "writeOnly": True},
            }}),
        "Create": Schema.model_validate({"type": "object", "properties": {
            "managed": {"$ref": "#/components/schemas/Managed"},
            "required_public": {"type": "string"}}, "required": ["managed", "required_public"]}),
    }
    synchronize(tmp_path / "managed_fixture", {"": render_models(schemas)}, check=False)
    monkeypatch.syspath_prepend(str(tmp_path))
    model = import_module("managed_fixture.create").Create
    value = {"request_id": "fc120782-2272-4b3d-a2e6-a26ee2f8ab61", "ownership_token": "af" * 32}
    valid = model(managed=value, required_public="okay")
    assert "af" * 32 not in repr(valid)
    assert valid.model_dump(mode="json") == {"managed": value, "required_public": "okay"}
    for fields in [{"ownership_token": "private-secret-invalid"}, {"request_id": "invalid"},
                   {"ownership_token": "af" * 32 + "\n"}]:
        with pytest.raises(ValidationError) as failure:
            model(managed={**value, **fields}, required_public="okay")
        assert "private-secret" not in str(failure.value)
        assert "af" * 32 not in repr(failure.value.errors(include_input=True))
    # A parent error can otherwise retain the entire raw input, even when the
    # nested credential model itself validated and redacts its own errors.
    with pytest.raises(ValidationError) as failure:
        model(managed=value)
    assert "af" * 32 not in str(failure.value)
    assert "af" * 32 not in repr(failure.value.errors(include_input=True))
