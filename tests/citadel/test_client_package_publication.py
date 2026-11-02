"""SDK publication inherits the exact native release authority and quality gates."""

from copy import deepcopy
from pathlib import Path

import pytest
import yaml

ROOT = Path(__file__).resolve().parents[2]
RATIONALE = (
    "Client registries may publish only the verified exact stable native source. "
    "A separate dispatch or source rebuild would detach SDKs from the packages and service they were tested with."
)


def _require_publication_contract(native: dict, client: dict) -> None:
    triggers = client.get("on", client.get(True))
    assert set(triggers) == {"workflow_call"}, RATIONALE
    call = native["jobs"]["publish-client-packages"]
    assert call["needs"] == ["verify-release-downloads"], RATIONALE
    assert call["if"] == "${{ inputs.channel == 'stable' && inputs.publish == true }}", RATIONALE
    assert call["uses"] == "./.github/workflows/release-client-packages.yaml", RATIONALE
    assert call["with"] == {
        "source_commit": "${{ inputs.source_commit }}", "release_tag": "${{ inputs.tag }}",
    }, RATIONALE
    assert set(call["secrets"]) == {"PYPI_TOKEN"}, RATIONALE
    assert set(triggers["workflow_call"]["secrets"]) == {"PYPI_TOKEN"}, RATIONALE
    assert call["permissions"]["id-token"] == "write", RATIONALE
    assert client["permissions"]["id-token"] == "write", RATIONALE
    job = client["jobs"]["publish"]
    checkout = next(step for step in job["steps"] if step.get("uses", "").startswith("actions/checkout@"))
    assert checkout["with"]["ref"] == "${{ inputs.source_commit }}", RATIONALE
    download = next(step for step in job["steps"] if step.get("uses", "").startswith("actions/download-artifact@"))
    assert download["with"] == {"name": "client-packages", "path": "cache/target",
                                "github-token": "${{ github.token }}",
                                "run-id": "${{ inputs.original_run_id || github.run_id }}"}, RATIONALE
    assert native["jobs"]["source-identity"]["if"] == "${{ inputs.client_run_id == '' }}", RATIONALE
    assert "inputs.client_run_id == ''" in native["jobs"]["cleanup-unpublished-version-claim"]["if"], RATIONALE
    recovery = native["jobs"]["resume-client-packages"]
    assert recovery["if"] == "${{ inputs.client_run_id != '' && inputs.channel == 'stable' && inputs.publish == true }}", RATIONALE
    assert recovery["uses"] == call["uses"], RATIONALE
    assert recovery["with"] == {**call["with"], "original_run_id": "${{ inputs.client_run_id }}"}, RATIONALE
    assert recovery["permissions"]["id-token"] == "write", RATIONALE
    resolvers = [step for step in job["steps"] if step.get("name") == "Resolve the qualified original client artifact run"]
    assert len(resolvers) == 1, RATIONALE
    resolver = resolvers[0]
    assert "-m capsem_builder.release.tools.client_recovery" in resolver["run"], RATIONALE
    assert resolver["if"] == "${{ inputs.original_run_id != '' }}", RATIONALE
    assert job["steps"].index(resolver) < job["steps"].index(download), RATIONALE
    publish = next(step for step in job["steps"] if "client_publishers publish" in step.get("run", ""))
    assert not {"NPM_TOKEN", "NODE_AUTH_TOKEN", "CARGO_REGISTRY_TOKEN"} & publish["env"].keys(), RATIONALE


def _require_prerequisites(native: dict) -> None:
    jobs = native["jobs"]
    prerequisites = jobs["client-publication-prerequisites"]
    check = next(step for step in prerequisites["steps"] if "client_publishers check-credentials" in step.get("run", ""))
    assert check["if"] == "${{ inputs.channel == 'stable' && inputs.publish == true }}", RATIONALE
    assert set(check["env"]) == {"PYPI_TOKEN"}, RATIONALE
    for name in ("fast-gate", "runtime-preflight"):
        assert "client-publication-prerequisites" in jobs[name]["needs"], RATIONALE


def _workflows() -> tuple[dict, dict]:
    return tuple(yaml.safe_load((ROOT / ".github/workflows" / name).read_text())
                 for name in ("release.yaml", "release-client-packages.yaml"))


def test_client_publication_is_an_exact_source_native_release_continuation() -> None:
    native, client = _workflows()
    _require_publication_contract(native, client)
    _require_prerequisites(native)


@pytest.mark.parametrize("mutation", [
    "dispatch", "early", "channel", "source", "archive", "credentials",
    "caller-oidc", "callee-oidc", "npm-token", "cargo-token",
    "recovery-source", "recovery-channel", "recovery-oidc", "recovery-resolver",
])
def test_publication_guard_rejects_independent_or_unqualified_routes(mutation: str) -> None:
    native, client = deepcopy(_workflows())
    _require_publication_contract(native, client)
    _require_prerequisites(native)
    if mutation == "recovery-source":
        native["jobs"]["resume-client-packages"]["with"]["source_commit"] = "main"
    elif mutation == "recovery-channel":
        native["jobs"]["resume-client-packages"]["if"] = "${{ inputs.publish == true }}"
    elif mutation == "recovery-oidc":
        native["jobs"]["resume-client-packages"]["permissions"]["id-token"] = "none"
    elif mutation == "recovery-resolver":
        client["jobs"]["publish"]["steps"] = [step for step in client["jobs"]["publish"]["steps"]
                                               if step.get("name") != "Resolve the qualified original client artifact run"]
    elif mutation == "dispatch":
        client.get("on", client.get(True))["workflow_dispatch"] = {}
    elif mutation == "early":
        native["jobs"]["publish-client-packages"]["needs"] = ["create-release"]
    elif mutation == "channel":
        native["jobs"]["publish-client-packages"]["if"] = "${{ inputs.publish == true }}"
    elif mutation in {"source", "archive"}:
        action = "actions/checkout@" if mutation == "source" else "actions/download-artifact@"
        step = next(step for step in client["jobs"]["publish"]["steps"] if step.get("uses", "").startswith(action))
        step["with"]["ref" if mutation == "source" else "name"] = "unqualified"
    elif mutation == "credentials":
        native["jobs"]["fast-gate"]["needs"] = ["source-identity"]
    elif mutation == "caller-oidc":
        native["jobs"]["publish-client-packages"]["permissions"]["id-token"] = "none"
    elif mutation == "callee-oidc":
        client["permissions"]["id-token"] = "none"
    else:
        key = "NPM_TOKEN" if mutation == "npm-token" else "CARGO_REGISTRY_TOKEN"
        native["jobs"]["publish-client-packages"]["secrets"][key] = "legacy-token"
    with pytest.raises(AssertionError):
        _require_publication_contract(native, client)
        _require_prerequisites(native)
