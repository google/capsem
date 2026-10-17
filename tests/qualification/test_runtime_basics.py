"""Mandatory for every image: it runs as its own config says, in the workload.

Nothing here is the harness's opinion of the image. The entrypoint, user and
working directory are read from the candidate's OCI config, and the workload
must honor each one: a session whose PID 1 is not the image's own command, or
whose exec lands in the VM, does not qualify.
"""

import pytest
from helpers.image_session import DEFAULT_TARGET, WORKSPACE, workload_exec

from tests.qualification.image_command import assert_image_command

pytestmark = pytest.mark.integration


def test_the_workload_runs_the_images_own_command(service, candidate, session):
    assert_image_command(service.client(), session, candidate)


@pytest.mark.parametrize(
    "target", ["workload", DEFAULT_TARGET], ids=["explicit", "omitted"]
)
def test_an_exec_runs_as_the_images_user_in_its_working_directory(
    service, candidate, session, target
):
    result = workload_exec(
        service.client(), session, "id -u; id -un; pwd", target=target
    )
    assert result.get("exit_code") == 0, result
    uid, name, cwd = result["stdout_text"].splitlines()
    user = candidate.user.split(":", 1)[0]
    assert user in (uid, name), (candidate.user, uid, name)
    assert cwd == candidate.working_dir, (candidate.working_dir, cwd)


def test_the_workspace_belongs_to_the_images_user(service, session):
    result = workload_exec(
        service.client(),
        session,
        f"umask 077; echo kept > {WORKSPACE}/private; "
        f"stat -c '%u %a' {WORKSPACE}/private; id -u; cat {WORKSPACE}/private",
    )
    assert result.get("exit_code") == 0, result
    owner_mode, uid, body = result["stdout_text"].splitlines()
    assert owner_mode == f"{uid} 600", (owner_mode, uid)
    assert body == "kept"
