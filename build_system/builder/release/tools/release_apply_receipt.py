"""Keep the original request outcome distinct from exact native activation."""

from __future__ import annotations

from .release_transition import validate_transition_verdict


def activation_receipt(
    request: dict, verdict: dict, *, kind: str, source: str, digest: str, source_commit: str
) -> dict:
    for key, expected in (
        ("source", source),
        ("candidate_manifest_sha256", digest),
        ("source_commit", source_commit),
        ("attempts", 1),
    ):
        if request.get(key) != expected:
            raise ValueError(f"original Apply receipt has wrong {key}")
    if request.get("outcome") not in {
        "succeeded",
        "http_failure",
        "application_failure",
        "invalid_response",
        "transport_failure",
    }:
        raise ValueError("original Apply receipt has no terminal outcome")
    status = request.get("http_status")
    if request["outcome"] == "succeeded" and (type(status) is not int or not 200 <= status < 300):
        raise ValueError("original Apply success has no successful HTTP receipt")
    if request["outcome"] == "transport_failure" and status is not None:
        raise ValueError("original transport failure cannot manufacture a complete HTTP status")
    validate_transition_verdict(
        verdict, kind=kind, result="activated", source=source, candidate_manifest_sha256=digest
    )
    return {
        "schema": "capsem.installed_apply.v1",
        "activation": "verified",
        "original_request": request,
        "transition": verdict,
    }
