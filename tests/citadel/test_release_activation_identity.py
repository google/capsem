"""The release hostname must never be polled before production identity converges."""

from pathlib import Path

from helpers.workflow_contract import parsed_commands, workflow_job, workflow_step

PROJECT_ROOT = Path(__file__).resolve().parents[2]
WORKFLOW = PROJECT_ROOT / ".github/workflows/release-channel.yaml"
ACTIVATION_RATIONALE = (
    "A successful Pages upload is not proof that the custom hostname selected it. "
    "Run 33141871462 spent fourteen minutes comparing candidate JSON with prior-deployment "
    "HTML because canonical deployment identity was checked only after the byte poll. Run "
    "33144251618 then proved that a branch-shaped Direct Upload can update canonical metadata "
    "without moving the project production alias."
)

NO_MANUAL_PURGE_RATIONALE = (
    "Cloudflare Pages activation changes the production deployment behind its custom hostname. "
    "A separate zone purge adds a second credential and failure rail without selecting different "
    "bytes; production and rollback must prove the selected deployment directly."
)


def test_production_identity_precedes_custom_hostname_byte_proof() -> None:
    steps = workflow_job(WORKFLOW, "deploy")["steps"]
    names = [step.get("name") for step in steps]
    identity_step = workflow_step(WORKFLOW, "deploy", "Verify canonical production deployment")
    public_step = workflow_step(WORKFLOW, "deploy", "Validate activated production bytes")
    identity_commands = parsed_commands(identity_step["run"], origin="production identity")

    assert names.index("Activate verified production distribution") < names.index(
        "Verify canonical production deployment"
    ) < names.index("Validate activated production bytes"), ACTIVATION_RATIONALE
    assert any(
        "build_system/scripts/web/cloudflare_pages_rollback.py" in command.argv and "verify" in command.argv
        for command in identity_commands
    ), ACTIVATION_RATIONALE
    assert "steps.production_identity.outcome == 'success'" in public_step["if"], (
        ACTIVATION_RATIONALE
    )
    assert "cloudflare_pages_rollback.py verify" not in public_step["run"], ACTIVATION_RATIONALE


def test_pages_preflight_binds_the_deploy_branch_to_cloudflare_production() -> None:
    preflight = workflow_step(WORKFLOW, "deploy", "Verify Cloudflare Pages project")["run"]

    assert '--production-branch "${{ inputs.deploy_branch || \'main\' }}"' in preflight, (
        ACTIVATION_RATIONALE
    )


def test_direct_upload_branches_only_the_immutable_preview() -> None:
    preview = workflow_step(WORKFLOW, "deploy", "Deploy immutable preview")
    production = workflow_step(WORKFLOW, "deploy", "Activate verified production distribution")
    preview_commands = parsed_commands(preview["with"]["command"], origin="Pages preview")
    production_commands = parsed_commands(production["with"]["command"], origin="Pages production")

    assert len(preview_commands) == len(production_commands) == 1, ACTIVATION_RATIONALE
    assert any(argument.startswith("--branch=") for argument in preview_commands[0].argv), (
        ACTIVATION_RATIONALE
    )
    assert not any(
        argument.startswith("--branch=") for argument in production_commands[0].argv
    ), ACTIVATION_RATIONALE


def test_pages_activation_never_manually_purges_cloudflare_cache() -> None:
    commands = []
    for index, step in enumerate(workflow_job(WORKFLOW, "deploy")["steps"]):
        run = step.get("run")
        if isinstance(run, str):
            commands.extend(parsed_commands(run, origin=f"Pages step {index} run"))
        options = step.get("with")
        if isinstance(options, dict) and isinstance(options.get("command"), str):
            commands.extend(
                parsed_commands(options["command"], origin=f"Pages step {index} command")
            )

    assert not any(
        argument.endswith("cloudflare_cache_purge.py")
        for command in commands
        for argument in command.argv
    ), NO_MANUAL_PURGE_RATIONALE
