"""HTTP generation refuses to drop request parameters, media types or auth."""

from __future__ import annotations

import json
from pathlib import Path

import pytest
from capsem_builder.sdkgen.operations import read_operations

ROOT = Path(__file__).resolve().parents[3]
SPEC = ROOT / "sdk/specification/openapi.json"


def test_every_documented_operation_is_read_with_its_wire_contract() -> None:
    source = json.loads(SPEC.read_text())
    routes = read_operations(SPEC)
    assert {(route.path, route.method) for route in routes} == {
        (path, method.upper()) for path, methods in source["paths"].items() for method in methods
    }
    for route in routes:
        original = source["paths"][route.path][route.method.lower()]
        assert route.operation.model_dump(by_alias=True, exclude_unset=True) == original


def test_upload_download_and_csv_parameters_are_preserved() -> None:
    operations = {route.operation.operation_id: route.operation for route in read_operations(SPEC)}
    upload = operations["uploadVmFile"]
    assert upload.request_body is not None
    assert upload.request_body.required
    assert upload.request_body.media_type == "application/octet-stream"
    assert operations["downloadVmFile"].responses["200"].media_type == "application/octet-stream"
    assert operations["getHypervisorLogs"].responses["200"].media_type == "application/json"
    timeline = operations["getVmTimeline"]
    layer = next(parameter for parameter in timeline.parameters if parameter.name == "layers")
    assert layer.style == "form" and layer.explode is False


def test_a_single_accepted_response_preserves_its_schema(tmp_path: Path) -> None:
    document = json.loads(SPEC.read_text())
    operation = document["paths"]["/vms/{id}/info"]["get"]
    success = operation["responses"].pop("200")
    operation["responses"]["202"] = success
    path = tmp_path / "accepted.json"
    path.write_text(json.dumps(document))
    parsed = next(route.operation for route in read_operations(path) if route.operation.operation_id == "getVmInfo")
    assert parsed.success_status == "202"
    assert parsed.success.model_dump(by_alias=True, exclude_unset=True) == success


@pytest.mark.parametrize("mutation", [
    "missing_path_parameter", "optional_path_parameter", "duplicate_parameter", "header_parameter",
    "duplicate_operation", "unsupported_method", "missing_security", "unknown_response",
    "multiple_success", "unknown_schema", "unsupported_media", "missing_operation_id",
    "empty_paths", "query_in_route", "csv_style", "missing_default", "ambiguous_media",
    "plain_success", "plain_request", "ambiguous_accepted",
])
def test_contract_changes_that_cannot_be_rendered_fail(mutation: str, tmp_path: Path) -> None:
    document = json.loads(SPEC.read_text())
    operation = document["paths"]["/vms/{id}/info"]["get"]
    match mutation:
        case "missing_path_parameter":
            operation["parameters"] = []
        case "optional_path_parameter":
            operation["parameters"][0]["required"] = False
        case "duplicate_parameter":
            operation["parameters"].append(operation["parameters"][0])
        case "header_parameter":
            operation["parameters"][0]["in"] = "header"
        case "duplicate_operation":
            document["paths"]["/duplicate/{id}"] = {"get": operation}
        case "unsupported_method":
            document["paths"]["/vms/{id}/info"] = {"patch": operation}
        case "missing_security":
            document.pop("security")
        case "unknown_response":
            operation["responses"]["200"]["content"]["application/json"]["schema"] = {}
        case "multiple_success":
            operation["responses"]["201"] = operation["responses"]["200"]
        case "ambiguous_accepted":
            operation["responses"]["202"] = operation["responses"]["200"]
        case "unknown_schema":
            operation["responses"]["200"]["content"]["application/json"]["schema"] = {
                "$ref": "#/components/schemas/Missing",
            }
        case "unsupported_media":
            operation["responses"]["200"]["content"]["text/html"] = {"schema": {"type": "string"}}
        case "missing_operation_id":
            operation.pop("operationId")
        case "empty_paths":
            document["paths"] = {}
        case "query_in_route":
            document["paths"]["/vms/{id}/info?extra=true"] = document["paths"].pop("/vms/{id}/info")
        case "csv_style":
            timeline = document["paths"]["/vms/{id}/timeline"]["get"]
            next(p for p in timeline["parameters"] if p["name"] == "layers").pop("style")
        case "missing_default":
            operation["responses"].pop("default")
        case "ambiguous_media":
            operation["responses"]["200"]["content"]["application/octet-stream"] = {
                "schema": {"type": "string", "format": "binary"},
            }
        case "plain_success":
            operation["responses"]["200"]["content"] = {"text/plain": {"schema": {"type": "string"}}}
        case "plain_request":
            document["paths"]["/update/apply"]["post"]["requestBody"]["content"] = {
                "text/plain": {"schema": {"type": "string"}},
            }
    path = tmp_path / "openapi.json"
    path.write_text(json.dumps(document))
    with pytest.raises(ValueError):
        read_operations(path)
