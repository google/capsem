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
