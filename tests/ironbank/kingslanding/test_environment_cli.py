"""Exact saved revisions cross real CLI and workload output boundaries."""

import subprocess
import uuid

import pytest
from helpers.constants import DEFAULT_CPUS, DEFAULT_RAM_MB
from helpers.service import vm_name

from tests.fixtures.oci.registry import publish_catalog, registry
from tests.ironbank.kingslanding.test_environment_attached import IMAGE, STARTUP
from tests.ironbank.kingslanding.test_environment_startup import IMAGE_CONFIG, request, save
from tests.ironbank.kingslanding.test_image_catalog import entry, use_catalog
from tests.ironbank.kingslanding.test_run import cli, environment, service

__all__ = ["service"]
pytestmark = pytest.mark.integration

def test_cli_run_saved_image_preserves_stdout_exit_and_timeout_cleanup(service, tmp_path):
    artifacts = {}
    with registry(tmp_path, image_config=IMAGE, artifacts=artifacts) as (image, certificate, _):
        authority = image.split("/", 1)[0]
        publish_catalog(artifacts, "library/catalog", "stable", entry(image))
        use_catalog(service, f"{authority}/library/catalog:stable", certificate)
        reference = save(service, {"name": "cli-attached", "image": {"reference": "redis"},
                                  "startup": {"script": STARTUP, "timeout_seconds": 10},
                                  "variables": {"BOUND_REVISION": "retained-variable"}})
        selected = f'{reference["environment_id"]}@{reference["revision_id"]}'
        base = [*cli(service, "run"), "--ram", str(DEFAULT_RAM_MB // 1024), "--cpu", str(DEFAULT_CPUS)]
        ran = subprocess.run([*base, "--environment", selected], env=environment(service),
                             capture_output=True, timeout=90, check=False)
        assert ran.returncode == 7 and ran.stdout == b"workload-retained-variable:1", (ran.returncode, ran.stdout, ran.stderr)
        assert ran.stderr.endswith(b"workload-error"), ran.stderr
        assert request(service, "GET", "/vms/list")["sandboxes"] == []
        timed = subprocess.run([*base, "--timeout", "1", "--environment", selected], env=environment(service),
                               capture_output=True, timeout=90, check=False)
        assert timed.returncode == 124 and timed.stdout == b"", (timed.returncode, timed.stdout, timed.stderr)
        assert b"Container timed out" in timed.stderr, timed.stderr
        assert request(service, "GET", "/vms/list")["sandboxes"] == []


def test_cli_create_selects_saved_workload_without_inline_content(service, tmp_path):
    artifacts = {}
    with registry(tmp_path, image_config=IMAGE_CONFIG, artifacts=artifacts) as (image, certificate, _):
        authority = image.split("/", 1)[0]
        publish_catalog(artifacts, "library/catalog", "stable", entry(image))
        use_catalog(service, f"{authority}/library/catalog:stable", certificate)
        # This fixture's original entrypoint checks both approved preparation
        # files, so success requires executing the retained variables and script.
        script = (STARTUP.replace("/workspace/count", "/workspace/start-count")
                  + 'printf "%s" "$BOUND_REVISION" > /workspace/prepared-value\n')
        reference = save(service, {"name": "cli-detached", "image": {"reference": "redis"},
                                  "startup": {"script": script, "timeout_seconds": 10},
                                  "variables": {"BOUND_REVISION": "retained-variable"}})
        selected = f'{reference["environment_id"]}@{reference["revision_id"]}'
        ran = subprocess.run([*cli(service, "create"), "-n", vm_name("cli-saved"),
                              "--ram", str(DEFAULT_RAM_MB // 1024), "--cpu", str(DEFAULT_CPUS),
                              "--environment", selected], env=environment(service), capture_output=True, timeout=90, check=False)
        assert ran.returncode == 0, (ran.stdout, ran.stderr)
        vm = ran.stdout.decode().split()[0]
        uuid.UUID(vm)
        assert request(service, "GET", f"/vms/{vm}/info")["environment"]["boot"] == reference
        assert request(service, "GET", f"/vms/{vm}/container")["state"] == "running"


def test_cli_run_reports_preparation_failure_before_destroying_its_vm(service, tmp_path):
    artifacts = {}
    with registry(tmp_path, image_config=IMAGE, artifacts=artifacts) as (image, certificate, _):
        authority = image.split("/", 1)[0]
        publish_catalog(artifacts, "library/catalog", "stable", entry(image))
        use_catalog(service, f"{authority}/library/catalog:stable", certificate)
        reference = save(service, {"name": "cli-startup-failure", "image": {"reference": "redis"},
                                  "startup": {"script": "printf preparation-detail; exit 17", "timeout_seconds": 10}})
        selected = f'{reference["environment_id"]}@{reference["revision_id"]}'
        ran = subprocess.run([*cli(service, "run"), "--ram", str(DEFAULT_RAM_MB // 1024),
                              "--cpu", str(DEFAULT_CPUS), "--environment", selected],
                             env=environment(service), capture_output=True, timeout=90, check=False)
        assert ran.returncode == 17 and ran.stdout == b"", (ran.returncode, ran.stdout, ran.stderr)
        assert b"startup exit (17)" in ran.stderr and b"preparation-detail" in ran.stderr, ran.stderr
        assert len(ran.stderr) < 64 * 1024 and b"\x1b" not in ran.stderr
        assert request(service, "GET", "/vms/list")["sandboxes"] == []
