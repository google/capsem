"""Compose sees only supplied evaluator inputs and bounded syntax."""

from pathlib import Path

import pytest
from inspect_capsem.containers.compose import (
    ComposeInputs,
    ComposeLimits,
    extract_compose_fields,
    parse_compose_yaml,
    parse_compose_yaml_file,
)

LIMITS = ComposeLimits(maximum_bytes=65536, maximum_nodes=1024, maximum_depth=32)


def test_no_ambient_environment_or_filesystem_access(monkeypatch: pytest.MonkeyPatch):
    monkeypatch.setenv("HOST_SECRET", "ungranted-host-value")

    def forbidden(*args, **kwargs):
        raise AssertionError("parser accessed host filesystem")

    for name in ["read_text", "is_file", "is_dir", "exists", "resolve"]:
        monkeypatch.setattr(Path, name, forbidden)
    result = parse_compose_yaml(
        "services:\n  app:\n    image: alpine\n    environment: [HOST_SECRET]\n",
        limits=LIMITS,
    )
    assert extract_compose_fields(result)["environment"] == {"HOST_SECRET": ""}
    with pytest.raises(PermissionError, match="not supplied"):
        parse_compose_yaml_file(
            Path("/ungranted/compose.yaml"), inputs=ComposeInputs(), limits=LIMITS
        )
    supplied = ComposeInputs(
        environment={"HOST_SECRET": "explicit-value"},
        files={
            "/project/compose.yaml": "services:\n  app:\n    image: ${IMAGE}\n",
            "/project/.env": "IMAGE=alpine:3.20\n",
        },
    )
    parsed = parse_compose_yaml_file(Path("/project/compose.yaml"), inputs=supplied, limits=LIMITS)
    assert parsed["services"]["app"]["image"] == "alpine:3.20"


def test_inputs_are_detached_from_mutable_caller_maps():
    environment = {"VALUE": "original"}
    files = {"Dockerfile": "FROM alpine\nUSER worker\nWORKDIR /work\n"}
    inputs = ComposeInputs(environment=environment, files=files)
    environment["VALUE"] = "changed"
    files["Dockerfile"] = "FROM scratch\n"
    parsed = parse_compose_yaml(
        "services: {app: {image: '${VALUE}'}}", inputs=inputs, limits=LIMITS
    )
    assert parsed["services"]["app"]["image"] == "original"
    result = extract_compose_fields(
        {"services": {"app": {"build": {"context": "."}}}}, inputs=inputs
    )
    assert result["user"] == "worker"
    assert result["working_dir"] == "/work"


@pytest.mark.parametrize(
    "text", ["services: &loop {app: *loop}", "x: [[[[]]]]", "x: ${A:-${B:-${C:-${D:-z}}}}"]
)
def test_cycle_and_depth_limits_refuse_unbounded_expansion(text: str):
    with pytest.raises(ValueError, match=r"cyclic|depth"):
        parse_compose_yaml(
            text, limits=ComposeLimits(maximum_bytes=4096, maximum_nodes=128, maximum_depth=3)
        )


def test_size_node_and_variable_expansion_limits():
    small = ComposeLimits(maximum_bytes=32, maximum_nodes=8, maximum_depth=8)
    with pytest.raises(ValueError, match="byte"):
        parse_compose_yaml("x: " + "v" * 40, limits=small)
    with pytest.raises(ValueError, match="node"):
        parse_compose_yaml("x: [1,2,3,4,5,6,7,8]", limits=small)
    with pytest.raises(ValueError, match="byte"):
        parse_compose_yaml(
            "x: $VALUE$VALUE", inputs=ComposeInputs(environment={"VALUE": "x" * 20}), limits=small
        )
    for limits in [(0, 1, 1), (1, 0, 1), (1, 1, 0)]:
        with pytest.raises(ValueError):
            ComposeLimits(*limits)


def test_errors_do_not_echo_supplied_values():
    inputs = ComposeInputs(environment={"VALUE": "fixture-private-value"})
    for text, dotenv in [
        ("x: ${MISSING:?$VALUE}", ""),
        ("x: value", 'VALUE="fixture-private-value" unexpected tail'),
    ]:
        with pytest.raises(ValueError) as captured:
            parse_compose_yaml(text, inputs=inputs, dotenv=dotenv, limits=LIMITS)
        assert "fixture-private-value" not in str(captured.value)


def test_yaml_merges_preserve_precedence_but_bound_constructor_expansion():
    ordinary = parse_compose_yaml(
        "first: &first {image: alpine, command: first}\n"
        "second: &second {image: busybox, command: second}\n"
        "services: {app: {<<: [*first, *second], command: explicit}}\n",
        limits=LIMITS,
    )
    assert ordinary["services"]["app"] == {"image": "alpine", "command": "explicit"}
    amplified = "n0: &n0 {image: alpine}\n"
    for index in range(1, 11):
        amplified += f"n{index}: &n{index} {{<<: [*n{index - 1}, *n{index - 1}]}}\n"
    with pytest.raises(ValueError, match="node"):
        parse_compose_yaml(
            amplified,
            limits=ComposeLimits(maximum_bytes=65536, maximum_nodes=128, maximum_depth=32),
        )
