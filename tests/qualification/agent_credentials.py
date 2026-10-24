"""Fixed existing synthetic authentication fixtures, never host credentials."""

import json
import shlex
import uuid

from helpers.agy_auth import synthetic_home_setup
from helpers.image_session import WORKSPACE, workload_exec


def environment(expect: dict) -> dict[str, str]:
    if expect.get("credential_fixture") == "agy_oauth":
        return {}
    return {expect["credential_env"]: "capsem-qualification-test-key"}


def command_environment(expect: dict) -> str:
    return shlex.join([f"{key}={value}" for key, value in environment(expect).items()])


def prepare(client, vm_id: str, expect: dict) -> None:
    if expect.get("credential_fixture") != "agy_oauth":
        return
    script = (
        "import json, os\nfrom pathlib import Path\n"
        "HOME = os.environ['HOME']\n"
        f"WORKSPACE = {json.dumps(WORKSPACE)}\nNONCE = {json.dumps(uuid.uuid4().hex)}\n"
        + synthetic_home_setup()
    )
    result = workload_exec(client, vm_id, "python3 -c " + shlex.quote(script))
    assert result.get("exit_code") == 0, result
