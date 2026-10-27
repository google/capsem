"""The enabled registry publishers get the authority they actually use."""

from helpers.constants import PROJECT_ROOT
from helpers.workflow_contract import workflow_document, workflow_job, workflow_step


def test_npm_trusted_publication_has_oidc_in_caller_and_reusable_workflow() -> None:
    binary = PROJECT_ROOT / ".github/workflows/release.yaml"
    clients = PROJECT_ROOT / ".github/workflows/release-client-packages.yaml"
    caller = workflow_job(binary, "publish-client-packages")
    reusable = workflow_document(clients)
    assert caller["permissions"]["id-token"] == "write"
    assert reusable["permissions"]["id-token"] == "write"
    assert caller["uses"] == "./.github/workflows/release-client-packages.yaml"
    assert set(caller["secrets"]) == {"PYPI_TOKEN"}
    trigger = reusable.get("on", reusable.get(True))
    assert set(trigger["workflow_call"]["secrets"]) == {"PYPI_TOKEN"}
    prerequisite = workflow_step(binary, "client-publication-prerequisites", "Require stable registry publisher configuration")
    assert set(prerequisite["env"]) == {"PYPI_TOKEN"}
    publish = workflow_step(clients, "publish", "Publish and verify the exact client cohort")
    assert not {"NPM_TOKEN", "NODE_AUTH_TOKEN", "CARGO_REGISTRY_TOKEN"} & publish["env"].keys()
