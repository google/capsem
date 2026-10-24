"""Native latent ReadFile rail, owned inside the real authenticated coordinator.

Public VirtioFS import/export does not exercise GuestToHost::FileContent. The
explicit feature-only Rust artifact owns that private typed capability; Python
supplies isolated fixture paths and never receives IPC authority or a token.
"""

import subprocess

import pytest
from capsem_builder.cache.config import load_paths
from capsem_builder.cache.leases import release_path, retain_generation
from capsem_builder.gate.config import load
from capsem_builder.gate.tools.native_file_content import selected
from helpers.bounded import bounded
from helpers.constants import ASSETS_DIR, PROJECT_ROOT
from helpers.service import make_service_home_run_dirs, preserve_tmp_dir_on_failure
from log_streams import read_log_stream

pytestmark = pytest.mark.integration


def test_guest_file_content_original_approve_deny_and_cancel():
    config = load(PROJECT_ROOT)
    settings = config.functional.native_file_content
    directory, key = selected(config)
    paths = load_paths(PROJECT_ROOT)
    stage = paths.policy.stages[settings.cache_stage]
    assert stage.lease_template is not None
    lease = paths.stage(settings.cache_stage) / stage.lease_template.format(key=key)
    retain_generation(paths, settings.cache_stage, key)
    home, run = make_service_home_run_dirs()
    corp = home / "corp.toml"
    corp.write_text('refresh_policy = "24h"\n')
    failed = True
    try:
        command = bounded(
            [str(directory / settings.artifact_name), "--exact", settings.selector, "--nocapture"],
            settings.timeout_seconds,
            env={
                "HOME": str(home),
                "CAPSEM_HOME": str(home),
                "CAPSEM_RUN_DIR": str(run),
                "CAPSEM_NATIVE_ACCEPTANCE_HOME": str(home),
                "CAPSEM_TRAY_HEADLESS": "1",
                "CAPSEM_CREDENTIAL_STORE_PATH": str(home / "credential-store.json"),
                "CAPSEM_CORP_CONFIG": str(corp),
                "CAPSEM_TEST_ASSETS_DIR": str(ASSETS_DIR),
            },
        )
        log = run / "native-file-content.log"
        with log.open("w") as output:
            result = subprocess.run(
                command, cwd=PROJECT_ROOT, stdout=output, stderr=subprocess.STDOUT
            )
        text = read_log_stream(log)
        assert result.returncode == 0, text
        assert "running 1 test" in text and "1 passed; 0 failed; 0 ignored" in text, text
        failed = False
    finally:
        if failed:
            preserve_tmp_dir_on_failure(home, force=True)
        release_path(lease)
