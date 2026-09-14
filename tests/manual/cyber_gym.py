#!/usr/bin/env python3
"""Cyber-gym smoke: an agent VM goes at a target service over a private network.

A hands-on validation of the private-network data plane, run against the
binaries under ``cache/target/cargo/debug`` -- no install, no mutation of an
installed service. It stands up a throwaway ``capsem-service`` on its own
socket (exactly as the ironbank suite does), boots two container VMs on one
named network -- the "target" runs a TCP listener (the guest's own
capsem-bench-rs), and the "agent" attacks it:

  1. Reachability -- the agent reaches the target's service on its private address.
  2. Connection churn -- 200 short-lived private connections in a row, every
     one answered. This is the case that used to lose one reply in ~2000 before
     the IPC channel released its descriptor in the wrong order; a CTF agent
     hammering a service is exactly that churn.
  3. Private DNS -- <vm>.<network>.capsem.internal resolves to the member address.
  4. Isolation -- a third VM that never joined the network gets nothing.
  5. Audit -- the network's ledger admits and records the flow from both ends.

Usage (run under the build_system interpreter, and bound it so no VM leaks):
    just _sign                         # build + codesign the host binaries
    python3 build_system/scripts/ci/run-bounded-command.py --timeout-seconds 1200 \
        -- uv run --project build_system --frozen python tests/manual/cyber_gym.py

The Redis image fixture must be materialized (the kingslanding suite does this;
otherwise run `just focus-test kingslanding slow` once). The binaries are read
from cache/target/cargo/debug, or CAPSEM_RELEASE_BIN_DIR if set; nothing here
touches an installed service.
"""

from __future__ import annotations

import contextlib
import os
import shlex
import subprocess
import sys
import time
from pathlib import Path

# `helpers.*` import as top-level (tests/ on the path); `tests.fixtures.*`
# import as a package (the repo root on the path). Only sys.path calls may sit
# above these first-party imports, or E402 fires -- and the imports cannot move
# up, they need the path set first.
sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "tests"))
sys.path.insert(0, str(Path(__file__).resolve().parents[2]))

from helpers.constants import BIN_DIR, CODE_PROFILE_ID
from helpers.service import ServiceInstance

from tests.fixtures.oci.registry import registry

# The ironbank suite sets this in tests/conftest.py; a standalone run must too,
# or capsem-service starts a tray on the macOS menu bar.
os.environ.setdefault("CAPSEM_TRAY_HEADLESS", "1")

# Run a command inside the container's network namespace, the way the
# kingslanding suite does. The guest writes the workload pid here at startup.
# `capsem-bench-rs` is injected into the guest filesystem (redis-cli is not),
# so it is the connection tool the agent uses against the target.
IN_CONTAINER = 'nsenter -t "$(cat /var/tmp/capsem-container/workload.pid)" -n'
# Same, but with the container's own resolver bound in: the guest's
# resolv.conf names a loopback proxy the container namespace cannot reach,
# so a name lookup has to run against the container gateway (as launch.py
# configures it). Mirrors the kingslanding datagram suite.
AS_CONTAINER = (
    "printf 'nameserver 10.0.1.1\\n' > /var/tmp/container-resolv.conf && "
    "unshare -m sh -c 'mount --bind /var/tmp/container-resolv.conf /etc/resolv.conf"
    ' && exec nsenter -t "$(cat /var/tmp/capsem-container/workload.pid)" -n "$@"\' sh'
)
NETWORK = "gym"
PORT = 5201
CHURN = 200


class Report:
    """Collects PASS/FAIL lines and decides the exit code."""

    def __init__(self) -> None:
        self.rows: list[tuple[bool, str]] = []

    def check(self, ok: bool, label: str, detail: str = "") -> bool:
        self.rows.append((ok, f"{label}{f'  ({detail})' if detail else ''}"))
        print(f"  [{'PASS' if ok else 'FAIL'}] {label}{f'  {detail}' if detail else ''}")
        return ok

    def ok(self) -> bool:
        return all(ok for ok, _ in self.rows)


def boot(service, tmp_path, reference, certificate, name, *publish):
    """Boot one Redis container VM under `name`; return its /vms/list row."""
    stdout = tmp_path / f"{name}.stdout"
    stderr = tmp_path / f"{name}.stderr"
    out, err = stdout.open("wb"), stderr.open("wb")
    command = [
        str(BIN_DIR / "capsem"),
        "--uds-path", str(service.uds_path),
        "run", "--profile", CODE_PROFILE_ID,
        "--registry-ca", str(certificate),
        "-n", name, *publish, reference,
    ]
    env = {
        **os.environ,
        "CAPSEM_HOME": str(service.home_dir),
        "CAPSEM_RUN_DIR": str(service.tmp_dir),
        "CAPSEM_PROFILES_DIR": str(service.profiles_dir),
    }
    process = subprocess.Popen(command, env=env, stdout=out, stderr=err)

    deadline = time.time() + 180
    while time.time() < deadline:
        if process.poll() is not None:
            raise RuntimeError(f"{name} exited early:\n{stderr.read_text()}")
        if b"Ready to accept connections tcp" in stdout.read_bytes():
            break
        time.sleep(0.5)
    else:
        raise RuntimeError(f"{name} never became ready:\n{stdout.read_text()}")

    rows = [r for r in service.client().get("/vms/list")["sandboxes"] if r.get("name") == name]
    if len(rows) != 1:
        raise RuntimeError(f"expected one {name} VM, saw {rows}")
    return process, rows[0]


def guest(service, vm_id, shell, timeout=40):
    """Run `shell` inside the VM; return the exec response."""
    return service.client().post(
        f"/vms/{vm_id}/exec", {"command": shell, "timeout_secs": timeout}, timeout=timeout + 5
    )


def serve(service, vm_id):
    """Detach a bench server in the target's container namespace."""
    inner = f"{IN_CONTAINER} capsem-bench-rs throughput --serve 0.0.0.0:{PORT}"
    guest(service, vm_id, f"setsid sh -c {shlex.quote(inner)} </dev/null >/var/tmp/serve.log 2>&1 &")


# The client is `capsem-bench-rs throughput` with `--direction latency`; there
# is no separate `latency` subcommand.
CLIENT = "capsem-bench-rs throughput --direction latency --streams 1 --seconds 1"


def probe_once(service, vm_id, addr, timeout=15):
    """One fresh connection: a 1s latency run against `addr`. True on success."""
    inner = f"{IN_CONTAINER} {CLIENT} --address {addr}"
    return guest(service, vm_id, inner, timeout=timeout).get("exit_code") == 0


def churn(service, vm_id, addr, count, timeout=180):
    """`count` back-to-back fresh connections in one guest loop; return successes."""
    inner = f"{CLIENT} --address {addr}"
    loop = (
        f"{IN_CONTAINER} sh -c '"
        f"n=0; i=0; while [ $i -lt {count} ]; do "
        f"{inner} >/dev/null 2>&1 && n=$((n+1)); i=$((i+1)); done; echo OK=$n'"
    )
    response = guest(service, vm_id, loop, timeout=timeout)
    import re

    match = re.search(r"OK=(\d+)", response.get("stdout", ""))
    return int(match.group(1)) if match else 0


def main() -> int:
    report = Report()
    service = ServiceInstance()
    service.start()
    client = service.client()
    tmp_path = service.tmp_dir
    booted: list = []
    try:
        with registry(tmp_path) as (reference, certificate, _requests):
            print("\n== boot two members on one private network ==")
            _, target = boot(service, tmp_path, reference, certificate, "target")
            _, agent = boot(service, tmp_path, reference, certificate, "agent")
            booted += [target["id"], agent["id"]]
            network = client.post("/networks", {"name": NETWORK})
            for vm in (target, agent):
                client.put(f"/networks/{network['id']}/members/{vm['id']}")
            # The address is the membership's lease in the network's subnet,
            # usable once the target's cable is plugged.
            addr = ""
            for _ in range(20):
                members = client.get(f"/networks/{network['id']}")["members"]
                addr = next(
                    (m["address"] for m in members if m["vm_id"] == target["id"] and m["state"] == "ready"),
                    "",
                )
                if addr:
                    break
                time.sleep(1)
            if not addr:
                raise RuntimeError("target never received a private address after joining")
            print(f"  target {addr}  network {NETWORK} ({network['id']})")

            print("\n== the agent goes at the target ==")
            serve(service, target["id"])
            addr_port = f"{addr}:{PORT}"
            reachable = False
            for _ in range(60):
                if probe_once(service, agent["id"], addr_port):
                    reachable = True
                    break
                time.sleep(1)
            report.check(reachable, "agent reaches the target service on its private address")

            ok = churn(service, agent["id"], addr_port, CHURN, timeout=240)
            report.check(ok == CHURN, f"{CHURN} short-lived private connections all answered", f"{ok}/{CHURN}")

            # The bench client takes a numeric SocketAddr, so validate the
            # private name with a real DNS lookup that must return the address.
            name = f"target.{NETWORK}.capsem.internal"
            resolved = guest(
                service, agent["id"], f"{AS_CONTAINER} getent hosts {name}", timeout=15
            ).get("stdout", "")
            report.check(addr in resolved, f"{name} resolves to the target's address", resolved.strip() or "no answer")

            print("\n== isolation: a VM off the network ==")
            _, outsider = boot(service, tmp_path, reference, certificate, "outsider")
            booted.append(outsider["id"])
            blocked = not probe_once(service, outsider["id"], addr_port, timeout=15)
            report.check(blocked, "a VM that never joined the network reaches nothing")

            print("\n== audit: the network ledger records the flow ==")

            def audited():
                for event in client.get(f"/networks/{network['id']}/logs").get("events", []):
                    facts = event.get("event", {}).get("network", {})
                    decision = event.get("event", {}).get("decision", {})
                    if (
                        facts.get("source", {}).get("vm", {}).get("id") == agent["id"]
                        and facts.get("destination", {}).get("vm", {}).get("id") == target["id"]
                        and facts.get("destination", {}).get("port") == PORT
                        and decision.get("effective") == "allow"
                    ):
                        return True
                return False

            found = False
            for _ in range(20):
                if audited():
                    found = True
                    break
                time.sleep(1)
            report.check(found, "the agent->target flow is admitted and audited from both ends")

        print(f"\n== {'ALL CHECKS PASSED' if report.ok() else 'FAILURES ABOVE'} ==")
        return 0 if report.ok() else 1
    finally:
        for vm_id in booted:
            with contextlib.suppress(Exception):
                client.delete(f"/vms/{vm_id}/delete")
        service.stop()


if __name__ == "__main__":
    raise SystemExit(main())
