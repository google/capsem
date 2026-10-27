"""Execute the runtime workflow's report consumer, including staged releases."""

from __future__ import annotations

import json
import os
import shlex
import subprocess
from pathlib import Path

import pytest
from helpers.bounded import bounded
from helpers.constants import PROJECT_ROOT
from helpers.workflow_contract import workflow_step


def _consume_report(
    tmp_path: Path, report: dict[str, object], *, functional_ready: bool = True,
) -> tuple[subprocess.CompletedProcess[str], str]:
    """Fixture only the upstream CLI report; execute the actual Bash and jq."""
    source = tmp_path / "cache/target/source-channel"
    source.mkdir(parents=True)
    (source / "manifest.json").write_text('{"version":"1.0.143"}', encoding="utf-8")
    payload = tmp_path / "report.json"
    payload.write_text(json.dumps(report), encoding="utf-8")
    cargo = tmp_path / "cargo"
    cargo.write_text(f"#!/bin/sh\nexec cat {shlex.quote(str(payload))}\n", encoding="utf-8")
    cargo.chmod(0o755)
    output = tmp_path / "outputs"
    step = workflow_step(
        PROJECT_ROOT / ".github/workflows/release-assets.yaml",
        "author-runtime-release",
        "Author the runtime in the channel source manifest",
    )
    script = tmp_path / "consume.sh"
    script.write_text(step["run"], encoding="utf-8")
    result = subprocess.run(
        bounded(["bash", str(script)], 30, env={
            "PATH": f"{tmp_path}{os.pathsep}{os.environ['PATH']}",
            "CHANNEL": "stable",
            "SOURCE_COMMIT": "0" * 40,
            "RUNTIME_REVISION": "0.7.2-000000000000",
            "PUBLICATION_IDENTITY": "runtime-stable-0.7.2-000000000000",
            "GITHUB_REPOSITORY": "google/capsem",
            "GITHUB_OUTPUT": str(output),
            "FUNCTIONAL_READY": str(functional_ready).lower(),
        }),
        cwd=tmp_path, capture_output=True, text=True, check=False,
    )
    return result, output.read_text(encoding="utf-8") if output.exists() else ""


@pytest.mark.parametrize("compatible", [True, False])
@pytest.mark.parametrize("functional_ready", [True, False])
def test_runtime_report_preserves_staging_and_activation(
    tmp_path: Path, compatible: bool, functional_ready: bool,
) -> None:
    result, output = _consume_report(tmp_path, {
        "publication_identity": "runtime-stable-0.7.2-000000000000",
        "compatible_with_current_binary": compatible,
    }, functional_ready=functional_ready)
    assert result.returncode == 0, result.stderr
    assert output == (
        f"product_compatible={str(compatible).lower()}\n"
        f"activation_ready={str(compatible and functional_ready).lower()}\n"
    )


@pytest.mark.parametrize("value", [None, "false", "true", 0, 1, [], {}])
def test_runtime_report_rejects_non_boolean_compatibility(tmp_path: Path, value: object) -> None:
    result, output = _consume_report(tmp_path, {
        "publication_identity": "runtime-stable-0.7.2-000000000000",
        "compatible_with_current_binary": value,
    })
    assert result.returncode != 0, "Malformed reports must not authorize runtime publication"
    assert not output


@pytest.mark.parametrize("identity", [None, "another-runtime"])
def test_runtime_report_rejects_missing_or_wrong_identity(tmp_path: Path, identity: object) -> None:
    report: dict[str, object] = {"compatible_with_current_binary": True}
    if identity is not None:
        report["publication_identity"] = identity
    result, output = _consume_report(tmp_path, report)
    assert result.returncode != 0
    assert not output


def test_runtime_report_requires_compatibility(tmp_path: Path) -> None:
    result, output = _consume_report(tmp_path, {
        "publication_identity": "runtime-stable-0.7.2-000000000000",
    })
    assert result.returncode != 0
    assert not output
