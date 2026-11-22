"""Saved image/startup acceptance through public routes and a TLS OCI catalog.

Only the configured catalog CA supplies registry trust. The fixture entrypoint
records what startup prepared, proving ordering before a real Redis workload.
"""

import concurrent.futures
import copy
import hashlib
import json
import secrets
import shlex
import ssl
import time
import uuid
from contextlib import closing
from urllib.parse import urlsplit
from urllib.request import urlopen

import pytest
from helpers.constants import DEFAULT_CPUS, DEFAULT_RAM_MB
from helpers.gateway import TcpHttpClient
from helpers.mock_server import start_mock_server, stop_process
from helpers.service import exec_output_text, vm_name, wait_exec_ready
from helpers.service_gateway import ServiceGateway

from tests.fixtures.oci.registry import publish_catalog, registry
from tests.ironbank.kingslanding.test_image_catalog import entry, use_catalog
from tests.ironbank.kingslanding.test_run import service
from tests.ironbank.kingslanding.test_workload_terminal import expect_output, terminal

__all__ = ["service"]

pytestmark = pytest.mark.integration

STARTUP = """set -eu
count=0
if [ -f /workspace/start-count ]; then count=$(cat /workspace/start-count); fi
printf '%s' "$((count + 1))" > /workspace/start-count
printf '%s' "$BOUND_REVISION" > /workspace/prepared-value
printf 'approved startup output\\n'
sleep 3
"""

IMAGE_CONFIG = {
    "Env": ["PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"],
    "WorkingDir": "/workspace",
    "Entrypoint": ["/bin/sh", "-c"],
    "Cmd": [(
        'set -eu; test "$(cat /workspace/prepared-value)" = "$BOUND_REVISION"; '
        'cp /workspace/start-count /workspace/entrypoint-count; '
        'printf started >> /workspace/entrypoint-runs; exec redis-server --loglevel warning'
    )],
}


def request(service, method, path, body=None, *, expected=200):
    code, value = service.client().call_json(method, path, body, timeout=90)
    assert code == expected, (method, path, code, value)
    return value


def save(service, specification):
    saved = request(service, "POST", "/environments/create", {
        "request_id": str(uuid.uuid4()), "specification": specification,
        "reason": "hermetic startup acceptance",
    })
    assert saved["application"] == "applied", saved
    return saved["result"]


def create(service, reference, name):
    return request(service, "POST", "/vms/create", {
        "name": name, "environment": reference,
        "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS,
    })["id"]


def observe(service, vm, reference, count):
    info = request(service, "GET", f"/vms/{vm}/info")["environment"]
    assert info["boot"] == reference, info
    output = request(service, "POST", f"/vms/{vm}/exec", {"command": (
        'printf "%s:%s:%s:%s" "$BOUND_REVISION" "$(cat /workspace/start-count)" '
        '"$(cat /workspace/prepared-value)" "$(cat /workspace/entrypoint-count)"'
    )})
    assert output["exit_code"] == 0, output
    assert exec_output_text(output) == f"retained-variable:{count}:retained-variable:{count}"
    assert request(service, "GET", f"/vms/{vm}/container")["state"] == "running"
    redis = request(service, "POST", f"/vms/{vm}/exec", {"command": "redis-cli PING"})
    assert redis["exit_code"] == 0 and exec_output_text(redis).strip() == "PONG", redis
    return info["owner_generation"]


def test_image_update_keeps_existing_workloads_pinned_and_updates_future_creations(service, tmp_path):
    artifacts = {}
    with registry(tmp_path, image_config=IMAGE_CONFIG, artifacts=artifacts) as (image, certificate, _):
        location, original_digest = image.split("@")
        authority, repository = location.split("/", 1)
        publish_catalog(artifacts, "library/catalog", "stable", entry(image))
        use_catalog(service, f"{authority}/library/catalog:stable", certificate)
        selected = save(service, {
            "name": "image-update-check",
            "image": {"reference": "redis"},
            "startup": {"script": STARTUP.replace("sleep 3\n", ""), "timeout_seconds": 10},
            "variables": {"BOUND_REVISION": "retained-variable"},
        })
        vm = create(service, selected, vm_name("image-update"))
        generation = observe(service, vm, selected, 1)

        with urlopen(
            f"https://{authority}/v2/{repository}/manifests/{original_digest}",
            context=ssl.create_default_context(cafile=str(certificate)),
            timeout=5,
        ) as response:
            manifest = json.load(response)
        manifest["annotations"] = {"org.opencontainers.image.version": "updated"}
        candidate = json.dumps(manifest, sort_keys=True).encode()
        candidate_digest = f"sha256:{hashlib.sha256(candidate).hexdigest()}"
        artifacts[f"/v2/{repository}/manifests/{candidate_digest}"] = (candidate, manifest["mediaType"])
        catalog = entry(image)
        catalog["redis"]["versions"].extend(entry(f"{location}@{candidate_digest}")["redis"]["versions"])
        publish_catalog(artifacts, "library/catalog", "stable", catalog)

        path = f"/environments/{selected['environment_id']}/revisions/{selected['revision_id']}/image-update"
        update = request(service, "GET", path)
        assert update["environment"] == selected
        assert update["current"]["resolved_digest"] == original_digest
        assert update["available"]["resolved_digest"] == candidate_digest
        assert observe(service, vm, selected, 1) == generation
        workload = request(service, "GET", f"/vms/{vm}/container")
        assert workload["resolved"] == image
        adoption = {
            "action": "update_image",
            "request_id": str(uuid.uuid4()),
            "base_revision_id": selected["revision_id"],
            "expected_digest": candidate_digest,
            "reason": "approve the newer image for future sandboxes",
        }
        changes = f"/environments/{selected['environment_id']}/changes"
        receipt = request(service, "POST", changes, adoption)
        assert receipt["decision"] == "approved" and receipt["application"] == "applied"
        assert request(service, "POST", changes, adoption) == receipt
        advanced = receipt["result"]
        assert advanced["environment_id"] == selected["environment_id"]
        assert advanced["revision_id"] != selected["revision_id"]
        updated_vm = create(service, advanced, vm_name("image-updated"))
        observe(service, updated_vm, advanced, 1)
        assert request(service, "GET", f"/vms/{updated_vm}/container")["resolved"] == f"{location}@{candidate_digest}"
        assert observe(service, vm, selected, 1) == generation
        request(service, "POST", f"/vms/{vm}/stop", {})
        request(service, "POST", f"/vms/{vm}/start", {})
        assert observe(service, vm, selected, 2) != generation
        assert request(service, "GET", f"/vms/{vm}/container")["resolved"] == image


def test_withdrawn_pinned_image_refuses_restart_without_advancing_the_revision(service, tmp_path):
    artifacts = {}
    with registry(tmp_path, image_config=IMAGE_CONFIG, artifacts=artifacts) as (image, certificate, requests):
        location, original_digest = image.split("@")
        authority = location.split("/", 1)[0]
        original_catalog = publish_catalog(artifacts, "library/catalog", "stable", entry(image))
        use_catalog(service, f"{authority}/library/catalog:stable", certificate)
        selected = save(service, {
            "name": "withdrawn-image-check",
            "image": {"reference": "redis", "update_policy": "pinned"},
            "startup": {"script": STARTUP.replace("sleep 3\n", ""), "timeout_seconds": 10},
            "variables": {"BOUND_REVISION": "retained-variable"},
        })
        vm = create(service, selected, vm_name("withdrawn-image"))
        running_generation = observe(service, vm, selected, 1)
        environment = f"/environments/{selected['environment_id']}"
        before = request(service, "GET", f"{environment}/revisions/list")
        replacement_digest = "sha256:" + "0" * 64
        assert replacement_digest != original_digest
        withdrawn_catalog = publish_catalog(
            artifacts, "library/catalog", "stable", entry(f"{location}@{replacement_digest}"),
        )
        refreshed = request(service, "GET", "/images?refresh=true")
        assert refreshed["catalog"]["digest"] == withdrawn_catalog
        assert observe(service, vm, selected, 1) == running_generation

        request(service, "POST", f"/vms/{vm}/stop", {})
        recorded_requests = len(requests)
        refused = request(service, "POST", f"/vms/{vm}/start", {}, expected=403)
        assert "saved image is unlisted, revoked or incompatible" in refused["error"], refused
        assert not any(path.startswith("/v2/library/redis/") for path in requests[recorded_requests:])
        assert request(service, "GET", f"{environment}/revisions/list") == before

        publish_catalog(artifacts, "library/catalog", "stable", entry(image))
        restored = request(service, "GET", "/images?refresh=true")
        assert restored["catalog"]["digest"] == original_catalog
        request(service, "POST", f"/vms/{vm}/start", {})
        assert observe(service, vm, selected, 2) != running_generation
        assert request(service, "GET", f"/vms/{vm}/container")["resolved"] == image


def test_saved_environment_remote_mcp_is_scoped_to_its_guest(service, tmp_path):
    log = tmp_path / "mcp-upstream.jsonl"
    upstream, ready = start_mock_server(addr="127.0.0.1:0", request_log=log)
    probe = """import json, subprocess, sys
from contextlib import closing

with subprocess.Popen(['/run/capsem-mcp-server'], stdin=subprocess.PIPE,
                      stdout=subprocess.PIPE, text=True) as relay:
    def send(method, params=None, *, notification=False):
        message = {'jsonrpc': '2.0', 'method': method}
        if params is not None:
            message['params'] = params
        if not notification:
            message['id'] = method
        relay.stdin.write(json.dumps(message) + '\\n')
        relay.stdin.flush()
        if not notification:
            return json.loads(relay.stdout.readline())

    with closing(relay.stdin):
        assert 'result' in send('initialize', {
            'protocolVersion': '2025-06-18', 'capabilities': {},
            'clientInfo': {'name': 'environment-mcp-test', 'version': '1'},
        })
        send('notifications/initialized', notification=True)
        print(json.dumps({
            'tools': send('tools/list')['result']['tools'],
            'call': send('tools/call', {
                'name': 'scoped__fixture_lookup', 'arguments': {'query': sys.argv[1]},
            }),
            'resource': send('resources/read', {'uri': 'capsem://scoped/doc://fixture'}),
        }))
"""
    try:
        endpoint = f"{ready['base_url']}/mcp"
        port = urlsplit(endpoint).port
        spec = {
            "name": "scoped-mcp-allowed",
            "access": {"custom": {"fixture": {
                "name": "Local MCP fixture", "action": "allow",
                "match": f'http.host == "127.0.0.1" && tcp.port == "{port}"',
            }}},
            "mcps": {
                "servers": {"scoped": {
                    "kind": "remote", "url": endpoint, "headers": {"X-Workspace": "approved"},
                }},
                "permissions": {
                    "scoped": {"default": "block", "tools": {"fixture_lookup": "allow"}},
                },
            },
        }
        allowed = save(service, spec)
        blocked_spec = copy.deepcopy(spec)
        blocked_spec["name"] = "scoped-mcp-blocked"
        blocked_spec["mcps"]["permissions"]["scoped"]["tools"].clear()
        blocked = save(service, blocked_spec)
        resource_spec = copy.deepcopy(spec)
        resource_spec["name"] = "scoped-mcp-resource"
        resource_spec["mcps"]["permissions"]["scoped"] = {
            "default": "allow", "tools": {"fixture_lookup": "block"},
        }
        resource_only = save(service, resource_spec)
        replies = {}
        instances = {}
        for decision, reference in [("blocked", blocked), ("allowed", allowed), ("resource", resource_only)]:
            vm = create(service, reference, vm_name(f"mcp-{decision}"))
            instances[decision] = vm
            assert wait_exec_ready(service.client(), vm)
            response = request(service, "POST", f"/vms/{vm}/exec", {
                "command": f"python3 -c {shlex.quote(probe)} {decision}",
                "timeout_secs": 30,
            })
            assert response["exit_code"] == 0, response
            replies[decision] = json.loads(exec_output_text(response))

        vm = instances["allowed"]
        route = f"/vms/{vm}/mcps"
        inventory = request(service, "GET", route)
        assert inventory["vm_id"] == vm
        (server,) = [entry for entry in inventory["servers"] if entry["name"] == "scoped"]
        assert server["kind"] == "remote" and server["connected"], inventory
        tool_list = request(service, "GET", f"{route}/scoped/tools")
        assert tool_list["owner_generation"] == inventory["owner_generation"]
        (tool,) = [entry for entry in tool_list["tools"] if entry["original_name"] == "fixture_lookup"]
        assert tool["input_schema"]["properties"]["query"]["type"] == "string"
        assert tool["definition_digest"]
        request(service, "GET", f"{route}/unknown/tools", expected=404)

        gateway = ServiceGateway(service)
        gateway.start()
        try:
            with closing(TcpHttpClient(gateway.base_url, gateway.token)) as operator:
                secret = secrets.token_hex(32)
                status, grant = operator.call_json("POST", "/agent-access", {
                    "request_id": str(uuid.uuid4()), "ownership_token": secret, "lease_seconds": 300,
                    "scope": {"operations": ["inspect"], "sessions": [{
                        "session_id": vm, "owner_generation": inventory["owner_generation"],
                    }]},
                })
                assert status == 200, grant
                with closing(TcpHttpClient(gateway.base_url, f"{grant['access']['id']}.{secret}")) as agent:
                    assert agent.call_json("GET", f"{route}/scoped/tools") == (200, tool_list)
                    assert agent.call_json("GET", f"/vms/{instances['blocked']}/mcps")[0] == 403
        finally:
            gateway.stop()

        assert replies["allowed"]["call"]["result"]["content"] == [
            {"type": "text", "text": "capsem-mock-server:mcp:fixture_lookup"},
        ]
        for reply in replies.values():
            assert "scoped__fixture_lookup" in {tool["name"] for tool in reply["tools"]}
        assert "environment.mcps.scoped.default" in replies["blocked"]["call"]["error"]["message"]
        for decision in ("blocked", "allowed"):
            assert "environment.mcps.scoped.default" in replies[decision]["resource"]["error"]["message"]
        assert "environment.mcps.scoped.tool_0" in replies["resource"]["call"]["error"]["message"]
        assert replies["resource"]["resource"]["result"]["contents"][0]["text"] == "capsem-mock-server:mcp:resource"
        posts = [
            row
            for line in log.read_text().splitlines()
            if (row := json.loads(line))["path"] == "/mcp" and row["method"] == "POST"
        ]
        assert posts and all(row["headers"].get("x-workspace") == "approved" for row in posts)
        messages = [json.loads(row["request_body"]) for row in posts]
        calls = [message["params"] for message in messages if message["method"] == "tools/call"]
        assert [(call["name"], call["arguments"]) for call in calls] == [
            ("fixture_lookup", {"query": "allowed"}),
        ]
        assert [message["params"]["uri"] for message in messages if message["method"] == "resources/read"] == [
            "doc://fixture",
        ]
    finally:
        stop_process(upstream)


def test_saved_sandbox_mcp_runs_approved_stdio_through_the_vm_owner(service, tmp_path):
    workload = {
        "User": "999:999", "WorkingDir": "/tmp",
        "Entrypoint": ["redis-server", "--save", "", "--appendonly", "no"],
    }
    peer = r'''while IFS= read -r message; do
    id=$(printf '%s' "$message" | sed -n 's/.*"id"[ ]*:[ ]*\([0-9][0-9]*\).*/\1/p')
    test -n "$id" || continue
    case "$message" in
        *'"initialize"'*) result='{"protocolVersion":"2024-11-05","capabilities":{"tools":{}},"serverInfo":{"name":"fixture","version":"1"}}' ;;
        *'"tools/list"'*) result='{"tools":[{"name":"check","inputSchema":{"type":"object"}}]}' ;;
        *'"resources/list"'*) result='{"resources":[]}' ;;
        *'"prompts/list"'*) result='{"prompts":[]}' ;;
        *'"tools/call"'*) result=$(printf '{"content":[{"type":"text","text":"%s|%s|%s|%s"}]}' "$(id -u)" "$PWD" "$MCP_FIXTURE" "$1") ;;
        *) exit 2 ;;
    esac
    printf '{"jsonrpc":"2.0","id":%s,"result":%s}\n' "$id" "$result"
done'''
    probe = """import json, subprocess, time

with subprocess.Popen(['/run/capsem-mcp-server'], stdin=subprocess.PIPE,
                      stdout=subprocess.PIPE, text=True) as relay:
    def send(method, identity=None, params=None):
        request = {'jsonrpc': '2.0', 'method': method}
        if identity is not None:
            request['id'] = identity
        if params is not None:
            request['params'] = params
        relay.stdin.write(json.dumps(request) + '\\n')
        relay.stdin.flush()
        if identity is not None:
            return json.loads(relay.stdout.readline())
    try:
        assert 'result' in send('initialize', 0, {
            'protocolVersion': '2024-11-05', 'capabilities': {},
            'clientInfo': {'name': 'sandbox-mcp-acceptance', 'version': '1'},
        })
        send('notifications/initialized')
        for attempt in range(5):
            tools = send('tools/list', 1 + attempt)['result']['tools']
            if 'guest__check' in {tool['name'] for tool in tools}:
                break
            time.sleep(1)
        print(json.dumps(send('tools/call', 10, {'name': 'guest__check'})))
    finally:
        relay.stdin.close()
"""
    artifacts = {}
    with registry(tmp_path, image_config=workload, artifacts=artifacts) as (image, certificate, _):
        authority = image.split("/", 1)[0]
        publish_catalog(artifacts, "library/catalog", "stable", entry(image))
        use_catalog(service, f"{authority}/library/catalog:stable", certificate)
        saved = save(service, {
            "name": "sandbox-mcp", "image": {"reference": "redis"},
            "mcps": {
                "servers": {"guest": {
                    "kind": "sandbox", "executable": "/bin/sh",
                    "args": ["-ec", peer, "fixture", "literal-token"],
                    "workdir": "/workspace", "variables": {"MCP_FIXTURE": "approved"},
                }},
                "permissions": {"guest": {"default": "block", "tools": {"check": "allow"}}},
            },
        })
        vm = create(service, saved, vm_name("sandbox-mcp"))
        result = request(service, "POST", f"/vms/{vm}/exec", {
            "target": "vm", "command": f"python3 -c {shlex.quote(probe)}", "timeout_secs": 30,
        })
        assert result["exit_code"] == 0, result
        call = json.loads(exec_output_text(result))
        assert call["result"]["content"] == [{"type": "text", "text": "999|/workspace|approved|literal-token"}], call


@pytest.mark.parametrize("operation", ["clone", "start", "fork"])
def test_advanced_saved_revision_uses_new_controls_from_carried_image_bytes(
    operation, service, tmp_path
):
    artifacts = {}
    with registry(tmp_path, image_config=IMAGE_CONFIG, artifacts=artifacts) as (
        image,
        certificate,
        requests,
    ):
        authority, repository = image.split("@", 1)[0].split("/", 1)
        publish_catalog(artifacts, "library/catalog", "stable", entry(image))
        use_catalog(service, f"{authority}/library/catalog:stable", certificate)
        initial = {
            "name": "carried-revision",
            "image": {"reference": "redis"},
            "startup": {
                "script": STARTUP.replace("sleep 3\n", ""),
                "timeout_seconds": 10,
            },
            "variables": {"BOUND_REVISION": "retained-variable", "OLD_ONLY": "old"},
        }
        original = save(service, initial)
        name = vm_name("carried-source")
        source = create(service, original, name)
        generation = observe(service, source, original, 1)
        request(service, "POST", f"/vms/{source}/stop", {})

        def revise(base, specification):
            return request(
                service,
                "POST",
                f"/environments/{base['environment_id']}/changes",
                {
                    "request_id": str(uuid.uuid4()),
                    "base_revision_id": base["revision_id"],
                    "specification": specification,
                    "reason": "verify fresh carried workload controls",
                },
            )["result"]

        next_script = (
            'set -eu\ntest -z "${OLD_ONLY+x}"\n'
            'test "$(cat /workspace/prepared-value)" = retained-variable\n'
            "printf next-approved > /workspace/choice\n" + initial["startup"]["script"]
        )
        next_revision = revise(
            original,
            {
                **initial,
                "startup": {"script": next_script, "timeout_seconds": 20},
                "variables": {"BOUND_REVISION": "next-variable"},
            },
        )
        head = revise(
            next_revision,
            {**initial, "startup": {"script": "exit 99", "timeout_seconds": 1}},
        )
        assert (
            len(
                {
                    original["revision_id"],
                    next_revision["revision_id"],
                    head["revision_id"],
                }
            )
            == 3
        )

        prefix = f"/v2/{repository}/"
        before = [path for path in requests if path.startswith(prefix)]
        assert before, requests
        workload_digests = {path.rsplit(":", 1)[1] for path in before}
        blobs = service.home_dir / "cache" / "oci-images" / "blobs"

        def cached_digests():
            return {
                hashlib.sha256(path.read_bytes()).hexdigest()
                for path in blobs.rglob("*")
                if path.is_file()
            }

        assert workload_digests <= cached_digests()
        blobs.rename(blobs.with_name("test-original-blobs"))

        if operation == "clone":
            target = request(
                service,
                "POST",
                "/vms/create",
                {
                    "from": name,
                    "name": vm_name("carried-clone"),
                    "environment": next_revision,
                    "ram_mb": DEFAULT_RAM_MB,
                    "cpus": DEFAULT_CPUS,
                },
            )["id"]
        else:
            # Seed the stopped fixture's approved next-start binding while its
            # carried controls still come from the last real boot.
            service.stop(cleanup=False)
            registry_path = service.tmp_dir / "persistent_registry.json"
            registry_data = json.loads(registry_path.read_text())
            entry_data = registry_data["vms"][name]
            assert entry_data["environment"] == original
            entry_data["environment"] = next_revision
            registry_path.write_text(json.dumps(registry_data))
            service.start()

            target = source
            if operation == "fork":
                ready = request(service, "GET", f"/vms/{source}/readiness?intent=fork")
                assert ready["selection"]["generation"] == generation
                assert ready["selection"]["environment"] == next_revision
                target = request(
                    service,
                    "POST",
                    f"/vms/{source}/fork",
                    {"name": vm_name("carried-fork"), "expected": ready["selection"]},
                )["id"]
            ready = request(service, "GET", f"/vms/{target}/readiness?intent=start")
            assert ready["selection"]["environment"] == next_revision
            assert ready["selection"]["generation"] == (
                None if operation == "fork" else generation
            )
            request(
                service,
                "POST",
                f"/vms/{target}/start",
                {"expected": ready["selection"]},
            )
        try:
            info = request(service, "GET", f"/vms/{target}/info")["environment"]
            assert (
                info["boot"] == next_revision and info["owner_generation"] != generation
            )
            result = request(
                service,
                "POST",
                f"/vms/{target}/exec",
                {
                    "command": (
                        'set -eu; test -z "${OLD_ONLY+x}"; test "$BOUND_REVISION" = next-variable; '
                        'test "$(cat /workspace/entrypoint-count)" = 2; '
                        'test "$(cat /workspace/choice)" = next-approved; redis-cli PING'
                    )
                },
            )
            assert (
                result["exit_code"] == 0 and exec_output_text(result).strip() == "PONG"
            ), result
            assert (
                request(service, "GET", f"/vms/{target}/container")["state"]
                == "running"
            )
            assert [path for path in requests if path.startswith(prefix)] == before
            assert workload_digests.isdisjoint(cached_digests())
        finally:
            request(service, "DELETE", f"/vms/{target}/delete")


def test_saved_startup_precedes_entrypoint_on_create_cold_resume_and_fork(service, tmp_path):
    artifacts = {}
    with registry(tmp_path, image_config=IMAGE_CONFIG, artifacts=artifacts) as (image, certificate, requests):
        authority = image.split("/", 1)[0]
        publish_catalog(artifacts, "library/catalog", "stable", entry(image))
        use_catalog(service, f"{authority}/library/catalog:stable", certificate)
        specification = {"name": "startup-acceptance", "image": {"reference": "redis"},
                         "startup": {"script": STARTUP, "timeout_seconds": 10},
                         "variables": {"BOUND_REVISION": "retained-variable"}}
        reference = save(service, specification)
        name = vm_name("startup-order")
        saw_preparing = False
        with concurrent.futures.ThreadPoolExecutor(max_workers=1) as executor:
            future = executor.submit(create, service, reference, name)
            deadline = time.monotonic() + 90
            while time.monotonic() < deadline and not future.done():
                rows = request(service, "GET", "/vms/list")["sandboxes"]
                for row in rows:
                    if row.get("name") != name:
                        continue
                    code, status = service.client().call_json("GET", f'/vms/{row["id"]}/container', timeout=5)
                    if code == 200 and status["state"] == "preparing":
                        assert set(status) == {"state", "image", "digest", "resolved"}, status
                        saw_preparing = True
                time.sleep(0.02)
            vm = future.result(timeout=5)
        assert saw_preparing, "startup must be observable before create becomes ready"
        first = observe(service, vm, reference, 1)
        # A detached startup finishes before any viewer connects. Its output
        # belongs to the held console, including a later reconnect's replay.
        for _ in range(2):
            with terminal(service, vm) as socket:
                expect_output(socket, b"approved startup output")
        assert any(path.endswith(f'/manifests/{image.rsplit("@", 1)[1]}') for path in requests)

        changed = copy.deepcopy(specification)
        changed["startup"]["script"] = "exit 99"
        edited = request(service, "POST", f'/environments/{reference["environment_id"]}/changes', {
            "request_id": str(uuid.uuid4()), "base_revision_id": reference["revision_id"],
            "specification": changed, "reason": "shared edits must not retarget existing owners",
        })
        assert edited["result"]["revision_id"] != reference["revision_id"]
        request(service, "POST", f"/vms/{vm}/stop", {})
        request(service, "POST", f"/vms/{vm}/resume", {})
        resumed = observe(service, vm, reference, 2)
        assert resumed != first
        fork = request(service, "POST", f"/vms/{vm}/fork", {"name": vm_name("startup-fork")})["id"]
        request(service, "POST", f"/vms/{fork}/resume", {})
        assert observe(service, fork, reference, 3) not in {first, resumed}
        observe(service, vm, reference, 2)
        managed_id, token = str(uuid.uuid4()), secrets.token_hex(32)
        managed = request(service, "POST", "/vms/create", {
            "environment": reference,
            "managed": {"request_id": managed_id, "ownership_token": token},
            "ram_mb": DEFAULT_RAM_MB, "cpus": DEFAULT_CPUS,
        })
        observe(service, managed["id"], reference, 1)
        closed = request(service, "POST", f"/vms/managed/{managed_id}/close", {
            "ownership_token": token, "outcome": "completed",
        })
        assert closed["state"] == "closed", closed


@pytest.mark.parametrize(("action", "failure"), [
    ("exit", "startup exit (17)"), ("timeout", "startup timed_out"),
    ("remove-shell", "startup entrypoint_exec"),
])
def test_failed_startup_blocks_entrypoint_and_retains_workspace(service, tmp_path, action, failure):
    artifacts = {}
    script = """set -eu
action=$(cat /workspace/startup-action 2>/dev/null || :)
case "$action" in
exit) printf preserved > /workspace/failure-evidence; printf '\\033[31m'; head -c 70000 /dev/zero | tr '\\000' x; exit 17;;
timeout) printf timer-evidence; sleep 30;;
remove-shell) rm /bin/sh;;
esac
""" + STARTUP.replace("sleep 3\n", "")
    with registry(tmp_path, image_config=IMAGE_CONFIG, artifacts=artifacts) as (image, certificate, _):
        authority = image.split("/", 1)[0]
        publish_catalog(artifacts, "library/catalog", "stable", entry(image))
        use_catalog(service, f"{authority}/library/catalog:stable", certificate)
        specification = {"name": "startup-failure", "image": {"reference": "redis"},
                         "startup": {"script": script, "timeout_seconds": 2},
                         "variables": {"BOUND_REVISION": "retained-variable"}}
        reference = save(service, specification)
        vm = create(service, reference, vm_name("startup-failure"))
        observe(service, vm, reference, 1)
        client = service.client()
        code, _, data = client.call("POST", f"/vms/{vm}/files/content?path=/workspace/startup-action",
                                   body=action.encode(), headers={"Content-Type": "application/octet-stream"})
        assert code == 200, (code, data)
        request(service, "POST", f"/vms/{vm}/stop", {})
        error = request(service, "POST", f"/vms/{vm}/resume", {}, expected=500)
        status = request(service, "GET", f"/vms/{vm}/container")
        assert status["state"] == "failed", status
        assert failure in status["error"] and failure in error["error"], (status, error)
        assert len(status["error"].encode()) < 64 * 1024
        assert "\x1b" not in status["error"]
        code, _, data = client.call("GET", f"/vms/{vm}/files/content?path=/workspace/entrypoint-runs")
        assert code == 200 and data == b"started", (code, data)
        if action == "exit":
            assert status["error"].endswith("[startup output truncated]"), status
            code, _, data = client.call("GET", f"/vms/{vm}/files/content?path=/workspace/failure-evidence")
            assert code == 200 and data == b"preserved", (code, data)
        if action == "remove-shell":
            request(service, "POST", f"/vms/{vm}/stop", {})
            error = request(service, "POST", f"/vms/{vm}/resume", {}, expected=500)
            assert "startup shell_unavailable" in error["error"], error
            status = request(service, "GET", f"/vms/{vm}/container")
            assert status["state"] == "failed" and "startup shell_unavailable" in status["error"], status
