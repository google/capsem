"""An image's Xpra surface opens in the browser through the gateway.

An image labelled `org.capsem.surface=xpra` with `org.capsem.surface.port`
gets one `http_preview` exposure of that container port once its workload
runs, reported in `GET /vms/{id}/container`. The gateway's launcher at
`/vms/{id}/surface/` mints a single-use bootstrap for it; the preview origin
exchanges that for its session cookie and serves the pinned xpra-html5 client
under `/_capsem/surface/`, and every other path reaches the workload's port.

The fixture is the Redis image relabelled, its command replaced by a busybox
`nc` HTTP responder on the declared port: the wiring is the subject here, not
Xpra itself (the claude-desktop image carries the real one).
"""

import http.client
import json
from urllib.parse import urlencode, urlsplit

import pytest

from tests.fixtures.oci.registry import registry
from tests.ironbank.kingslanding.test_run import created, service, wait_for

__all__ = ["service"]

pytestmark = pytest.mark.integration

PORT = 14500
READY = "kingslanding-surface-listening"
# One HTTP/1.0 response per connection, echoing the request line, so a reply
# proves which path reached the workload. The headers are read to the blank
# line first: closing on unread bytes would reset the connection.
RESPONDER = (
    'read -r line; while read -r header; do case "$header" in "$(printf "\\r")"|"") break;; esac; done; '
    'body="workload:${line%$(printf "\\r")}"; '
    'printf "HTTP/1.0 200 OK\\r\\nContent-Type: text/plain\\r\\nContent-Length: %s\\r\\nConnection: close\\r\\n\\r\\n%s" '
    '"${#body}" "$body"'
)
IMAGE_CONFIG = {
    "Env": ["PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"],
    "Entrypoint": ["/bin/sh", "-c"],
    "Cmd": [f"echo {READY}; exec nc -lk -p {PORT} -e /bin/sh -c '{RESPONDER}'"],
    "WorkingDir": "/data",
    "Labels": {"org.capsem.surface": "xpra", "org.capsem.surface.port": str(PORT)},
}


def request(port, method, path, *, host=None, token=None, cookie=None, body=b"", content_type=None):
    """(status, headers, body) of one request to the gateway or a preview origin."""
    headers = {}
    if host is not None:
        headers["Host"] = host
    if token is not None:
        headers["Authorization"] = f"Bearer {token}"
    if cookie is not None:
        headers["Cookie"] = cookie
    if content_type is not None:
        headers["Content-Type"] = content_type
    if body:
        headers["Content-Length"] = str(len(body))
    connection = http.client.HTTPConnection("127.0.0.1", port, timeout=10)
    try:
        connection.request(method, path, body=body, headers=headers)
        response = connection.getresponse()
        return (
            response.status,
            {name.lower(): value for name, value in response.getheaders()},
            response.read(),
        )
    finally:
        connection.close()


def test_an_xpra_image_gets_its_surface_through_the_gateway(service, tmp_path):
    client = service.client()
    with (
        registry(tmp_path, image_config=IMAGE_CONFIG) as (reference, certificate, _),
        created(service, tmp_path, reference, certificate, "surface", ready=READY) as vm,
    ):
        vm_id = vm["id"]
        statuses = []

        def granted():
            statuses.append(client.get(f"/vms/{vm_id}/container"))
            return (statuses[-1].get("surface") or {}).get("exposure_id") is not None

        wait_for(granted, "surface exposure grant", timeout=60)
        status = statuses[-1]
        (tmp_path / "container.json").write_text(json.dumps(status, indent=2))
        exposure_id = status["surface"]["exposure_id"]
        assert status["state"] == "running", status
        assert status["surface"] == {"kind": "xpra", "port": PORT, "exposure_id": exposure_id}, (
            status
        )

        # The grant is an ordinary preview exposure: no host listener.
        exposures = client.get(f"/vms/{vm_id}/exposures")
        (tmp_path / "exposures.json").write_text(json.dumps(exposures, indent=2))
        assert exposures["exposures"] == [
            {
                "id": exposure_id,
                "host_port": None,
                "guest_port": PORT,
                "target": "container",
                "access": "http_preview",
            }
        ], exposures

        gateway = int((service.tmp_dir / "gateway.port").read_text())
        token = (service.tmp_dir / "gateway.token").read_text().strip()

        # The launcher holds nothing secret and is served without a token.
        launcher = request(gateway, "GET", f"/vms/{vm_id}/surface/")
        assert launcher[0] == 200, launcher
        assert launcher[1]["content-type"] == "text/html; charset=utf-8", launcher
        assert "form-action http://*.localhost:" in launcher[1]["content-security-policy"], launcher
        assert b'data-auto="true"' in launcher[2], launcher
        script = request(gateway, "GET", f"/vms/{vm_id}/surface/launch.js")
        assert script[0] == 200 and b"/surface/session" in script[2], script

        # Minting a session is not: it needs the gateway token.
        assert request(gateway, "POST", f"/vms/{vm_id}/surface/session")[0] == 401
        minted = request(gateway, "POST", f"/vms/{vm_id}/surface/session", token=token)
        assert minted[0] == 200, minted
        session = json.loads(minted[2])
        url = urlsplit(session["url"])
        assert url.hostname == f"{exposure_id}.localhost", session
        assert url.path == "/_capsem/bootstrap" and url.query == "", session
        origin = f"{url.hostname}:{url.port}"

        # Without the origin's session the client is not served, and nothing
        # reaches the workload.
        assert request(url.port, "GET", "/_capsem/surface/", host=origin)[0] == 401
        assert request(url.port, "GET", "/hello", host=origin)[0] == 401

        exchanged = request(
            url.port,
            "POST",
            url.path,
            host=origin,
            body=urlencode({"bootstrap_token": session["bootstrap_token"]}).encode(),
            content_type="application/x-www-form-urlencoded",
        )
        assert exchanged[0] == 303, exchanged
        assert exchanged[1]["location"] == "/_capsem/surface/", exchanged
        cookie = exchanged[1]["set-cookie"].split(";", 1)[0]
        assert cookie.startswith("capsem_preview="), exchanged

        page = request(url.port, "GET", "/_capsem/surface/", host=origin, cookie=cookie)
        assert page[0] == 200, page
        assert page[1]["content-type"] == "text/html; charset=utf-8", page
        assert f"connect-src 'self' ws://{origin}" in page[1]["content-security-policy"], page
        assert b'<script src="capsem-surface.js"></script>' in page[2], page
        assert b'<script src="xpra-main.js"></script>' in page[2], page
        assert b"<script>" not in page[2], "the client page keeps no inline script"
        for asset in ("xpra-main.js", "capsem-surface.js", "default-settings.txt"):
            served = request(
                url.port, "GET", f"/_capsem/surface/{asset}", host=origin, cookie=cookie
            )
            assert served[0] == 200 and served[2], (asset, served[0])

        # Everything else on the origin is the workload's declared port.
        replies = []

        def reached():
            replies.append(
                request(url.port, "GET", "/hello?from=kingslanding", host=origin, cookie=cookie)
            )
            return replies[-1][0] == 200

        wait_for(reached, "workload surface port through the preview origin", timeout=30)
        assert replies[-1][2] == b"workload:GET /hello?from=kingslanding HTTP/1.1", replies[-1]

        # The bootstrap was single use.
        replayed = request(
            url.port,
            "POST",
            url.path,
            host=origin,
            body=urlencode({"bootstrap_token": session["bootstrap_token"]}).encode(),
            content_type="application/x-www-form-urlencoded",
        )
        assert replayed[0] >= 400, replayed
