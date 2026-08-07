"""Update-route contract tests for the service HTTP API."""


def test_update_routes_plan_cli_commands_without_mutation(client):
    check = client.post("/update/check", {"dry_run": True})
    assert check["status"] == "planned"
    assert check["command"]["args"] == ["update", "--check"]

    apply = client.post("/update/apply", {"dry_run": True})
    assert apply["status"] == "planned"
    assert apply["command"]["args"] == ["update", "--yes"]


def test_update_apply_requires_confirmation_for_live_command(client):
    body = client.post("/update/apply", {})
    assert body["error"] == "update apply requires confirmed=true or dry_run=true"
