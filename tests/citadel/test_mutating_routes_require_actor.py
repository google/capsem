"""Mutation registration must preserve the compiler's sealed actor requirement."""

from __future__ import annotations

import re
from pathlib import Path

import pytest
from rust_sources import masked_code

ROOT = Path(__file__).resolve().parents[2]
SERVICE = ROOT / "crates/capsem-service/src"
OWNER = "caller_auth/routing.rs"
MUTATIONS = {"post", "put", "patch", "delete"}
READ_ONLY = {
    "get",
    "get_service",
    "head",
    "head_service",
    "options",
    "options_service",
    "trace",
    "trace_service",
    "MethodRouter",
}
ACTOR_RATIONALE = """
Issue #407 requires every mutating service handler to extract a sealed
AuthenticatedActor. Use caller_auth::routing constructors: their ActorArguments
bound makes the compiler require that proof as the first extractor. A caller
DTO, JSON body or optional actor cannot satisfy it. Raw Axum constructors,
MethodRouter mutation chains and arbitrary-service routes bypass this proof.
The sole outbound OTLP HTTP request is a reqwest client, not API registration.
"""


def violations(relative: str, source: str) -> list[str]:
    code = masked_code(source)
    found = []
    if re.search(r"(?m)^\s*impl(?:\s*<[^{};]*>)?\s+(?:\w+::)*Handler\s*<", code):
        found.append("custom Handler implementations bypass standard actor extraction")
    if relative == OWNER:
        tuples = re.findall(r"ActorArguments\s+for\s*\(([^)]*)\)", code)
        if len(tuples) != 2 or any(
            not re.match(r"\s*M\s*,\s*AuthenticatedActor\s*(?:,|$)", value) for value in tuples
        ):
            found.append("actor argument implementations must require sealed first proof")
        if not re.search(r"\bT\s*:[^,;{}]*\bActorArguments\b", code):
            found.append("mutation constructors must enforce the actor tuple bound")
        return found
    if re.search(r"\bActorArguments\b", code):
        found.append("only the checked routing owner may implement actor tuple proofs")
    for match in re.finditer(
        r"(?P<prefix>(?:\b\w+\s*::\s*)*)\brouting\b"
        r"(?:\s*::\s*(?P<members>\{[^{}]*\}|\*|\w+))?",
        code,
    ):
        prefix = re.sub(r"\s+", "", match["prefix"])
        trusted = prefix.endswith("caller_auth::")
        members = match["members"]
        if (
            not prefix
            and members is None
            and not re.search(r"\buse\b[^;]*$", code[: match.start()])
        ):
            continue
        if trusted and members in (None, "*"):
            continue
        if members is None or members == "*":
            found.append("untyped routing namespace or wildcard import")
            continue
        names = {part.strip().split()[0] for part in members.strip("{}").split(",") if part.strip()}
        if not names <= (MUTATIONS if trusted else READ_ONLY):
            found.append("untyped HTTP routing constructor")
    outbound = []
    if relative == "service_runtime/telemetry_export.rs":
        outbound = [
            match.span()
            for match in re.finditer(
                r"\brelay_client\s*\(\s*\)\s*\?\s*\.post\s*\(\s*collector\s*\)",
                code,
            )
        ]
    for match in re.finditer(
        r"\.\s*(?:(?:post|put|patch|delete)(?:_service)?|on|on_service|any_service|"
        r"route_service|nest_service|fallback|fallback_service)\s*\(",
        code,
    ):
        if any(start <= match.start() < end for start, end in outbound):
            continue
        found.append("unchecked mutation chain or arbitrary-service route")
    return found


def test_service_mutation_registration_requires_sealed_actor_proof() -> None:
    failures = []
    for path in SERVICE.rglob("*.rs"):
        relative = path.relative_to(SERVICE)
        if path.name == "tests.rs" or "tests" in relative.parts:
            continue
        failures.extend(
            f"{relative}: {failure}"
            for failure in violations(relative.as_posix(), path.read_text())
        )
    assert not failures, ACTOR_RATIONALE + "\n" + "\n".join(failures)


@pytest.mark.parametrize(
    "source",
    [
        "use axum::routing::post; Router::new().route(path, post(handler));",
        "use axum::routing::post as get; Router::new().route(path, get(handler));",
        "use axum::{routing::{patch as edit}};",
        "use axum::routing as routes; routes::delete(handler);",
        "use axum::{routing as raw}; raw::post(handler);",
        "use axum::routing::*;",
        "Router::new().route(path, axum::routing::put(handler));",
        "get(read).post(write)",
        "get(read).delete(write)",
        "get(read).post_service(service)",
        "MethodRouter::new().on(filter, write)",
        "Router::new().route_service(path, service)",
        "Router::new().nest_service(path, service)",
        "Router::new().fallback(write)",
        "impl<M> ActorArguments for (M, Json<CallerActor>) {}",
        "impl<T> axum::handler::Handler<T, S> for Fake {}",
    ],
)
def test_unchecked_registration_shapes_fail(source: str) -> None:
    assert violations("fixture.rs", source), ACTOR_RATIONALE


@pytest.mark.parametrize(
    "source",
    [
        "use crate::caller_auth::routing::{post, put, patch, delete};",
        "use caller_auth::routing::post as mutate; mutate(handler)",
        "use axum::routing::get; get(read).merge(post(write))",
        "use axum::{routing::{get as read}};",
        "use axum::routing::{get_service, head, options, trace};",
        "let present = entries.iter().any(|entry| entry.active);",
        "mod routing;",
        'let text = r##"axum::routing::post(handler) /* raw data */"##;',
        "/* outside /* nested */ axum::routing::post */ get(read)",
        'let text = "// not a comment: axum::routing::post";',
    ],
)
def test_proven_constructors_and_noncode_are_allowed(source: str) -> None:
    assert not violations("fixture.rs", source), ACTOR_RATIONALE


@pytest.mark.parametrize(
    "replacement", ["CallerActor", "Json<AuthenticatedActor>", "Option<AuthenticatedActor>"]
)
def test_body_dto_and_optional_actors_cannot_replace_sealed_proof(replacement: str) -> None:
    source = (SERVICE / OWNER).read_text()
    assert violations(OWNER, source.replace("AuthenticatedActor", replacement)), ACTOR_RATIONALE


def test_actor_bound_cannot_be_hidden_in_a_comment_or_string() -> None:
    source = (SERVICE / OWNER).read_text().replace("T: ActorArguments + 'static", "T: 'static")
    source += (
        "\n// T: ActorArguments + 'static\nconst FAKE: &str = \"T: ActorArguments + 'static\";"
    )
    assert violations(OWNER, source), ACTOR_RATIONALE


def test_literal_mask_preserves_offsets_and_handles_lifetimes() -> None:
    source = """fn f<'a>(v: &'a str) {
    /* outside /* nested */ comment */
    let raw = br###"/* text */ \" fn fake() {}"###;
    let character = '\\u{7b}';
    let text = "escaped \\" and // data";
    real();
}"""
    masked = masked_code(source)
    assert len(masked) == len(source)
    assert [i for i, c in enumerate(masked) if c == "\n"] == [
        i for i, c in enumerate(source) if c == "\n"
    ]
    assert "fn f<'a>" in masked and "real();" in masked
    assert "fake" not in masked and "comment" not in masked
