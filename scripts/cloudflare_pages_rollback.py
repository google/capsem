#!/usr/bin/env python3
"""Capture and restore the exact prior Cloudflare Pages production deployment."""

from __future__ import annotations

import argparse
import json
import os
import sys
import time
import urllib.error
import urllib.request
from collections.abc import Callable
from pathlib import Path
from typing import cast
from urllib.parse import quote

API_ROOT = "https://api.cloudflare.com/client/v4"
STATE_SCHEMA = "capsem.cloudflare_pages_rollback.v1"
Requester = Callable[[str, str], dict[str, object]]
STEP_OUTCOMES = frozenset({"failure", "skipped", "success"})


class RollbackError(RuntimeError):
    """Cloudflare did not prove an exact production capture or restoration."""


def cloudflare_requester(account_id: str, api_token: str) -> Requester:
    if not account_id or not api_token:
        raise RollbackError("Cloudflare account id and API token are required")

    def request(method: str, path: str) -> dict[str, object]:
        url = f"{API_ROOT}/accounts/{quote(account_id, safe='')}{path}"
        request = urllib.request.Request(
            url,
            method=method,
            headers={"Authorization": f"Bearer {api_token}"},
        )
        try:
            with urllib.request.urlopen(request, timeout=30) as response:
                payload = json.loads(response.read().decode("utf-8"))
        except urllib.error.HTTPError as error:
            detail = error.read().decode("utf-8", errors="replace")
            error.close()
            raise RollbackError(
                f"Cloudflare {method} {path} failed: HTTP {error.code}: {detail}"
            ) from None
        except (OSError, UnicodeDecodeError, json.JSONDecodeError) as error:
            raise RollbackError(f"Cloudflare {method} {path} failed: {error}") from error
        if not isinstance(payload, dict):
            raise RollbackError(f"Cloudflare {method} {path} returned a non-object")
        return cast(dict[str, object], payload)

    return request


def _result(payload: dict[str, object], operation: str) -> dict[str, object]:
    result = payload.get("result")
    if payload.get("success") is not True or not isinstance(result, dict):
        raise RollbackError(f"Cloudflare {operation} did not return a successful object: {payload}")
    return cast(dict[str, object], result)


def canonical_deployment(payload: dict[str, object], project: str) -> dict[str, object]:
    result = _result(payload, "project lookup")
    if result.get("name") != project:
        raise RollbackError(
            f"Cloudflare project lookup returned {result.get('name')!r}, not {project!r}"
        )
    deployment = result.get("canonical_deployment")
    if not isinstance(deployment, dict):
        raise RollbackError(f"Cloudflare Pages project {project} has no production deployment")
    deployment = cast(dict[str, object], deployment)
    deployment_id = deployment.get("id")
    url = deployment.get("url")
    stage = deployment.get("latest_stage")
    if not isinstance(deployment_id, str) or not deployment_id:
        raise RollbackError("canonical production deployment has no id")
    if not isinstance(url, str) or not url.startswith("https://"):
        raise RollbackError("canonical production deployment has no HTTPS URL")
    if not isinstance(stage, dict):
        raise RollbackError("canonical production deployment has no latest stage")
    stage = cast(dict[str, object], stage)
    if stage.get("status") != "success":
        raise RollbackError("canonical production deployment is not a successful build")
    if deployment.get("environment") != "production":
        raise RollbackError("canonical deployment is not a production deployment")
    return {
        "schema": STATE_SCHEMA,
        "project": project,
        "deployment_id": deployment_id,
        "deployment_url": url,
    }


def capture_production(request: Requester, project: str) -> dict[str, object]:
    path = f"/pages/projects/{quote(project, safe='')}"
    return canonical_deployment(request("GET", path), project)


def load_state(path: Path, project: str) -> dict[str, object]:
    value = json.loads(path.read_text(encoding="utf-8"))
    if not isinstance(value, dict) or value.get("schema") != STATE_SCHEMA:
        raise RollbackError(f"unsupported rollback state: {path}")
    if value.get("project") != project:
        raise RollbackError(f"rollback state belongs to {value.get('project')!r}, not {project!r}")
    deployment_id = value.get("deployment_id")
    if not isinstance(deployment_id, str) or not deployment_id:
        raise RollbackError("rollback state has no deployment id")
    return cast(dict[str, object], value)


def restore_production(
    request: Requester,
    state: dict[str, object],
    *,
    attempts: int,
    delay_seconds: float,
    sleep: Callable[[float], None] = time.sleep,
) -> dict[str, object]:
    project = str(state["project"])
    deployment_id = str(state["deployment_id"])
    base = f"/pages/projects/{quote(project, safe='')}"
    rounds = max(attempts, 1)
    last_error: RollbackError | None = None
    for attempt in range(rounds):
        try:
            rollback = _result(
                request(
                    "POST",
                    f"{base}/deployments/{quote(deployment_id, safe='')}/rollback",
                ),
                "production rollback",
            )
            if rollback.get("id") != deployment_id:
                raise RollbackError(
                    "Cloudflare rollback response did not identify the prior deployment"
                )
        except RollbackError as error:
            last_error = error
            try:
                observed = canonical_deployment(request("GET", base), project)
                if observed["deployment_id"] == deployment_id:
                    return observed
            except RollbackError as lookup_error:
                last_error = lookup_error
            if attempt + 1 < rounds:
                sleep(delay_seconds)
            continue
        return wait_for_canonical(
            request,
            project,
            deployment_id,
            attempts=rounds,
            delay_seconds=delay_seconds,
            sleep=sleep,
        )
    raise RollbackError(f"Cloudflare rollback request never converged: {last_error}")


def wait_for_canonical(
    request: Requester,
    project: str,
    deployment_id: str,
    *,
    attempts: int,
    delay_seconds: float,
    sleep: Callable[[float], None] = time.sleep,
) -> dict[str, object]:
    observed: dict[str, object] | None = None
    last_error: RollbackError | None = None
    for attempt in range(max(attempts, 1)):
        try:
            observed = canonical_deployment(
                request("GET", f"/pages/projects/{quote(project, safe='')}"), project
            )
            last_error = None
            if observed["deployment_id"] == deployment_id:
                return observed
        except RollbackError as error:
            last_error = error
        if attempt + 1 < max(attempts, 1):
            sleep(delay_seconds)
    if last_error is not None:
        raise RollbackError(
            f"Cloudflare canonical deployment could not be verified: {last_error}"
        ) from last_error
    raise RollbackError(
        f"Cloudflare canonical deployment remained {observed and observed['deployment_id']!r}; "
        f"expected canonical {deployment_id!r}"
    )


def write_state(path: Path, state: dict[str, object]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    pending = path.with_name(f".{path.name}.tmp")
    pending.write_text(json.dumps(state, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    pending.replace(path)


def activation_decision(production_outcome: str, validation_outcome: str) -> dict[str, bool]:
    """Decide recovery without confusing an untouched preview failure with activation."""
    for label, outcome in (
        ("production", production_outcome),
        ("validation", validation_outcome),
    ):
        if outcome not in STEP_OUTCOMES:
            raise RollbackError(f"unsupported {label} step outcome: {outcome!r}")
    if production_outcome == "skipped":
        return {"restore": False, "activation_success": False}
    activation_success = production_outcome == validation_outcome == "success"
    return {"restore": not activation_success, "activation_success": activation_success}


def write_activation_decision(path: Path, production_outcome: str, validation_outcome: str) -> None:
    decision = activation_decision(production_outcome, validation_outcome)
    with path.open("a", encoding="utf-8") as output:
        output.write(f"restore={str(decision['restore']).lower()}\n")
        output.write(f"activation-success={str(decision['activation_success']).lower()}\n")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=("capture", "decision", "restore", "verify"))
    parser.add_argument("--project", default="release")
    parser.add_argument("--state", type=Path)
    parser.add_argument("--account-id", default=os.environ.get("CLOUDFLARE_ACCOUNT_ID"))
    parser.add_argument("--api-token", default=os.environ.get("CLOUDFLARE_API_TOKEN"))
    parser.add_argument("--attempts", type=int, default=30)
    parser.add_argument("--delay-seconds", type=float, default=10)
    parser.add_argument("--production-outcome")
    parser.add_argument("--validation-outcome")
    parser.add_argument("--github-output", type=Path)
    parser.add_argument("--deployment-id")
    args = parser.parse_args()
    try:
        if args.action == "decision":
            if not args.production_outcome or not args.validation_outcome:
                parser.error("decision requires both step outcomes")
            if args.github_output is None:
                parser.error("decision requires --github-output")
            write_activation_decision(
                args.github_output,
                args.production_outcome,
                args.validation_outcome,
            )
            return 0
        if args.action in {"capture", "restore"} and args.state is None:
            parser.error(f"{args.action} requires --state")
        request = cloudflare_requester(args.account_id or "", args.api_token or "")
        if args.action == "capture":
            assert args.state is not None
            state = capture_production(request, args.project)
            write_state(args.state, state)
            print(f"captured production deployment {state['deployment_id']}")
        elif args.action == "restore":
            assert args.state is not None
            state = load_state(args.state, args.project)
            restored = restore_production(
                request,
                state,
                attempts=args.attempts,
                delay_seconds=args.delay_seconds,
            )
            print(f"restored production deployment {restored['deployment_id']}")
        else:
            if not args.deployment_id:
                parser.error("verify requires --deployment-id")
            verified = wait_for_canonical(
                request,
                args.project,
                args.deployment_id,
                attempts=args.attempts,
                delay_seconds=args.delay_seconds,
            )
            print(f"verified canonical production deployment {verified['deployment_id']}")
    except (OSError, ValueError, json.JSONDecodeError, RollbackError) as error:
        print(f"Cloudflare Pages rollback failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
