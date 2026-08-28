"""The release hostname must never be polled before production identity converges."""

from pathlib import Path

from helpers.workflow_contract import parsed_commands, workflow_job, workflow_step

PROJECT_ROOT = Path(__file__).resolve().parents[2]
WORKFLOW = PROJECT_ROOT / ".github/workflows/release-channel.yaml"
ACTIVATION_RATIONALE = (
    "A successful Pages upload is not proof that the custom hostname selected it. "
    "Run 33141871462 spent fourteen minutes comparing candidate JSON with prior-deployment "
    "HTML because canonical deployment identity was checked only after the byte poll."
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
        "scripts/cloudflare_pages_rollback.py" in command.argv and "verify" in command.argv
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
