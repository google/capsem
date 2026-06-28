"""Update-route contract tests for the service HTTP API."""


def test_update_routes_plan_cli_commands_without_mutation(client):
    check = client.post("/update/check", {"dry_run": True})
    assert check["status"] == "planned"
    assert check["command"]["args"] == ["update"]

    binary = client.post("/update/apply", {"action": "binary_profiles", "dry_run": True})
    assert binary["status"] == "planned"
    assert binary["command"]["args"] == ["update", "--yes"]

    assets = client.post("/update/apply", {"action": "assets", "dry_run": True})
    assert assets["status"] == "planned"
    assert assets["command"]["args"] == ["update", "--assets"]


def test_update_apply_requires_confirmation_for_live_command(client):
    body = client.post("/update/apply", {"action": "assets"})
    assert body["error"] == "update apply requires confirmed=true or dry_run=true"


def test_update_check_requires_dry_run_until_non_mutating_runner_exists(client):
    body = client.post("/update/check", {})
    assert (
        body["error"]
        == "update check requires dry_run=true until a non-mutating check runner is available"
    )
