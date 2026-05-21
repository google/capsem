from __future__ import annotations

from pathlib import Path
import textwrap

import pytest
from click.testing import CliRunner
from pydantic import ValidationError

from capsem.admin.cli import cli
from capsem.builder.security_packs import (
    DetectionPackV1,
    PolicyPackV1,
    dump_detection_pack_json,
    dump_detection_pack_schema_json,
    dump_policy_pack_json,
    dump_policy_pack_schema_json,
    dump_policy_pack_toml,
    validate_detection_pack_json,
    validate_detection_pack_toml,
    validate_detection_pack_yaml,
    validate_policy_pack_json,
    validate_policy_pack_toml,
)


PROJECT_ROOT = Path(__file__).resolve().parents[1]
POLICY_SCHEMA_PATH = PROJECT_ROOT / "schemas" / "capsem.policy-pack.v1.schema.json"
DETECTION_SCHEMA_PATH = (
    PROJECT_ROOT / "schemas" / "capsem.detection-pack.v1.schema.json"
)


def _policy_json() -> str:
    return """
    {
      "schema": "capsem.policy-pack.v1",
      "id": "corp-default-policy",
      "version": "2026.0521.1",
      "status": "active",
      "owner": "corp",
      "rules": [
        {
          "id": "block-metadata",
          "name": "Block cloud metadata",
          "event_family": "http",
          "event_type": "http.request",
          "priority": 10,
          "condition": "subject.request.host == '169.254.169.254'",
          "decision": "block",
          "reason": "metadata endpoints are not reachable from corp VMs"
        }
      ]
    }
    """


def _detection_toml() -> str:
    return textwrap.dedent(
        """
        schema = "capsem.detection-pack.v1"
        id = "corp-default-detections"
        version = "2026.0521.1"
        status = "active"
        owner = "corp"
        description = "Default corp detections."

        [field_mapping.http]
        Host = "subject.request.host"
        Url = "subject.request.url"

        [[sources]]
        id = "metadata-access"
        type = "sigma"
        format = "yaml"
        content = '''
        title: Metadata endpoint access
        id: 11111111-1111-4111-8111-111111111111
        status: test
        logsource:
          product: capsem
          category: http
        detection:
          selection:
            Host: 169.254.169.254
          condition: selection
        level: high
        '''

        [findings]
        default_severity = "high"
        default_confidence = "medium"
        tags = ["attack.discovery"]
        """
    )


def _detection_yaml() -> str:
    return textwrap.dedent(
        """
        schema: capsem.detection-pack.v1
        id: corp-default-detections
        version: 2026.0521.1
        status: active
        owner: corp
        description: Default corp detections.
        field_mapping:
          http:
            Host: subject.request.host
            Url: subject.request.url
        sources:
          - id: metadata-access
            type: sigma
            format: yaml
            content: |
              title: Metadata endpoint access
              id: 11111111-1111-4111-8111-111111111111
              status: test
              logsource:
                product: capsem
                category: http
              detection:
                selection:
                  Host: 169.254.169.254
                condition: selection
              level: high
        findings:
          default_severity: high
          default_confidence: medium
          tags:
            - attack.discovery
        """
    )


def test_policy_pack_json_enters_and_leaves_through_pydantic() -> None:
    pack = validate_policy_pack_json(_policy_json())
    dumped = dump_policy_pack_json(pack)
    reparsed = PolicyPackV1.model_validate_json(dumped)

    assert reparsed == pack
    assert pack.rules[0].decision.value == "block"
    assert '"schema": "capsem.policy-pack.v1"' in dumped


def test_policy_pack_toml_json_toml_round_trip_is_canonical(tmp_path: Path) -> None:
    pack = validate_policy_pack_json(_policy_json())
    toml = dump_policy_pack_toml(pack)

    path = tmp_path / "policy-pack.toml"
    path.write_text(toml, encoding="utf-8")
    from_toml = validate_policy_pack_toml(path)
    from_json = validate_policy_pack_json(dump_policy_pack_json(from_toml))

    assert dump_policy_pack_toml(from_json) == toml


def test_policy_pack_rewrite_requires_payload() -> None:
    payload = _policy_json().replace('"decision": "block"', '"decision": "rewrite"')

    with pytest.raises(ValidationError, match="rewrite decision requires rewrite"):
        validate_policy_pack_json(payload)


def test_detection_pack_toml_enters_and_leaves_through_pydantic(tmp_path: Path) -> None:
    path = tmp_path / "detection-pack.toml"
    path.write_text(_detection_toml(), encoding="utf-8")

    pack = validate_detection_pack_toml(path)
    dumped = dump_detection_pack_json(pack)
    reparsed = DetectionPackV1.model_validate_json(dumped)

    assert reparsed == pack
    assert pack.sources[0].type == "sigma"
    assert pack.field_mapping["http"]["Host"] == "subject.request.host"


def test_detection_pack_yaml_enters_through_pydantic(tmp_path: Path) -> None:
    path = tmp_path / "detection-pack.yml"
    path.write_text(_detection_yaml(), encoding="utf-8")

    pack = validate_detection_pack_yaml(path)

    assert pack.id == "corp-default-detections"
    assert pack.sources[0].format == "yaml"


def test_detection_pack_rejects_enforcement_decision(tmp_path: Path) -> None:
    path = tmp_path / "detection-pack.toml"
    path.write_text(
        _detection_toml()
        + '\n[[rules]]\nid = "bad"\ndecision = "block"\n',
        encoding="utf-8",
    )

    with pytest.raises(ValidationError):
        validate_detection_pack_toml(path)


def test_security_pack_schema_exports_are_stable() -> None:
    assert POLICY_SCHEMA_PATH.read_text(
        encoding="utf-8",
    ) == dump_policy_pack_schema_json() + "\n"
    assert DETECTION_SCHEMA_PATH.read_text(
        encoding="utf-8",
    ) == dump_detection_pack_schema_json() + "\n"


def test_capsem_admin_policy_validate_and_schema(tmp_path: Path) -> None:
    path = tmp_path / "policy-pack.json"
    path.write_text(_policy_json(), encoding="utf-8")

    validate = CliRunner().invoke(
        cli,
        ["policy", "validate", str(path), "--json"],
    )
    schema = CliRunner().invoke(cli, ["policy", "schema"])

    assert validate.exit_code == 0, validate.output
    assert '"ok": true' in validate.output
    assert '"pack_id": "corp-default-policy"' in validate.output
    assert schema.exit_code == 0, schema.output
    assert '"capsem.policy-pack.v1"' in schema.output


def test_capsem_admin_detection_validate_and_schema(tmp_path: Path) -> None:
    path = tmp_path / "detection-pack.yml"
    path.write_text(_detection_yaml(), encoding="utf-8")

    validate = CliRunner().invoke(
        cli,
        ["detection", "validate", str(path), "--json"],
    )
    schema = CliRunner().invoke(cli, ["detection", "schema"])

    assert validate.exit_code == 0, validate.output
    assert '"ok": true' in validate.output
    assert '"pack_id": "corp-default-detections"' in validate.output
    assert schema.exit_code == 0, schema.output
    assert '"capsem.detection-pack.v1"' in schema.output
