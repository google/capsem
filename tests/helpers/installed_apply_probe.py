"""One original installed Apply request; never retry or infer HTTP success."""

from __future__ import annotations

import argparse
import json
from pathlib import Path

from capsem_builder.release.tools.release_apply_receipt import (
    activation_receipt as activation_receipt,
)
from capsem_builder.release.tools.release_apply_receipt import system_monotonic_ns
from capsem_builder.release.tools.release_transition import load_update_audit

from helpers.http_measurement import validate_limits
from helpers.http_transport import Transport
from helpers.installed_gateway import GatewayTransport


def _json(owner, args, method, path, body=None):
    response = owner.measure_request(
        method,
        path,
        body=None if body is None else json.dumps(body).encode(),
        headers={"Content-Type": "application/json"},
        timeout=args.timeout,
        max_response_bytes=args.maximum,
    )
    if response.status != 200:
        raise ValueError("installed settings request refused")
    return json.loads(response.body)


def automatic_value(value):
    def walk(nodes):
        for node in nodes:
            if node.get("kind") == "leaf" and node.get("id") == "app.auto_update":
                return node.get("effective_value")
            found = walk(node.get("children", []))
            if found is not None:
                return found
        return None

    result = walk(value["tree"])
    if type(result) is not bool:
        raise ValueError("automatic update setting is missing or invalid")
    return result


def set_automatic(owner, args, enabled):
    value = _json(owner, args, "PATCH", "/settings/edit", {"app.auto_update": enabled})
    if automatic_value(value) is not enabled:
        raise ValueError("automatic update setting did not settle")
    if automatic_value(_json(owner, args, "GET", "/settings/info")) is not enabled:
        raise ValueError("automatic update setting was not persisted")


def request_apply(owner, args):
    if args.transport not in {"service_uds", "gateway_http"}:
        raise ValueError("original Apply requires explicit transport provenance")
    if automatic_value(_json(owner, args, "GET", "/settings/info")):
        raise ValueError("automatic updates must be disabled before original Apply")
    rows = load_update_audit(args.audit, after_line=args.after_line, max_bytes=args.maximum)
    if any(
        row.get("source") == args.source and row.get("candidate_manifest_sha256") == args.digest
        for row in rows
    ):
        raise ValueError("candidate was handled before original Apply request")
    started = system_monotonic_ns()
    result = {
        "schema": "capsem.installed_apply_request.v1",
        "source": args.source,
        "candidate_manifest_sha256": args.digest,
        "source_commit": args.source_commit,
        "after_line": args.after_line,
        "attempts": 1,
        "transport": args.transport,
        "started_ns": started,
    }
    try:
        response = owner.measure_request(
            "POST",
            "/update/apply",
            body=b'{"confirmed":true}',
            headers={"Content-Type": "application/json"},
            timeout=args.timeout,
            max_response_bytes=args.maximum,
        )
    except (ConnectionError, TimeoutError) as error:
        result.update(outcome="transport_failure", error_type=type(error).__name__)
        if error.__cause__ is not None:
            result["cause_type"] = type(error.__cause__).__name__
    else:
        result.update(
            http_status=response.status,
            response_bytes=len(response.body),
            headers_ns=response.headers_ns,
            first_body_byte_ns=response.first_body_byte_ns,
            complete_ns=response.complete_ns,
        )
        if not 200 <= response.status < 300:
            result["outcome"] = "http_failure"
        else:
            try:
                payload = json.loads(response.body)
                status = payload.get("status") if isinstance(payload, dict) else None
                code = payload.get("exit_code") if isinstance(payload, dict) else None
            except (ValueError, UnicodeError):
                status, code = None, None
            result["outcome"] = (
                "succeeded"
                if status == "succeeded" and type(code) is int and code == 0
                else "application_failure"
                if status == "failed"
                else "invalid_response"
            )
    result["elapsed_ns"] = system_monotonic_ns() - started
    args.output.write_text(json.dumps(result, sort_keys=True) + "\n")
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("command", choices=("disable-automatic", "restore-automatic", "request"))
    address = parser.add_mutually_exclusive_group(required=True)
    address.add_argument("--socket")
    address.add_argument("--gateway-run-dir", type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--state", type=Path)
    parser.add_argument("--audit", type=Path)
    parser.add_argument("--after-line-file", type=Path)
    parser.add_argument("--source")
    parser.add_argument("--digest")
    parser.add_argument("--source-commit")
    parser.add_argument("--timeout", required=True, type=float)
    parser.add_argument("--maximum", required=True, type=int)
    args = parser.parse_args()
    validate_limits(args.timeout, args.maximum)
    args.transport = "gateway_http" if args.gateway_run_dir is not None else "service_uds"
    owner = (
        GatewayTransport(args.gateway_run_dir, maximum=args.maximum)
        if args.gateway_run_dir is not None
        else Transport(socket_path=args.socket)
    )
    try:
        if args.command == "disable-automatic":
            previous = automatic_value(_json(owner, args, "GET", "/settings/info"))
            args.output.write_text(json.dumps({"previous": previous}) + "\n")
            set_automatic(owner, args, False)
        elif args.command == "restore-automatic":
            if args.state is None:
                parser.error("restore requires --state")
            previous = json.loads(args.state.read_text())["previous"]
            if type(previous) is not bool:
                raise ValueError("invalid original automatic setting")
            set_automatic(owner, args, previous)
        else:
            if not all(
                (args.source, args.digest, args.source_commit, args.audit, args.after_line_file)
            ):
                parser.error("request requires exact source/digest/commit/audit/marker")
            args.after_line = int(args.after_line_file.read_text().strip())
            request_apply(owner, args)
    finally:
        owner.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
