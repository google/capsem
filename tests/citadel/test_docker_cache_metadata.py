"""Identity labels must not invalidate the work they merely describe."""

import re

import pytest
from helpers.gate import PROJECT_ROOT

CACHE_METADATA_RATIONALE = (
    "Docker passes every in-scope ARG to subsequent RUN instructions, even "
    "when the shell never references it. Declare INPUT_IDENTITY and its label "
    "after all build work: changing helper metadata must not reinstall apt, "
    "Rust or profile dependencies. See skills/dev-cache/SKILL.md."
)


def _metadata_is_last(source: str) -> bool:
    instructions = [line.strip() for line in source.splitlines() if line.strip()]
    declarations = [
        i
        for i, line in enumerate(instructions)
        if re.match(r"ARG\s+INPUT_IDENTITY(?:\s|=|$)", line)
    ]
    if len(declarations) != 1:
        return False
    suffix = instructions[declarations[0] + 1 :]
    return any(
        line.startswith("LABEL ") and "INPUT_IDENTITY" in line for line in suffix
    ) and all(line.startswith(("#", "LABEL ")) for line in suffix)


@pytest.mark.parametrize(
    "path",
    sorted(
        [
            *PROJECT_ROOT.glob("build_system/docker/Dockerfile.*"),
            *PROJECT_ROOT.glob("config/docker/Dockerfile.*.j2"),
        ]
    ),
)
def test_builder_identity_only_labels_completed_work(path):
    source = path.read_text()
    if "ARG INPUT_IDENTITY" in source:
        assert _metadata_is_last(source), f"{path}: {CACHE_METADATA_RATIONALE}"


def test_metadata_guard_rejects_implicit_run_inputs_and_early_labels():
    valid = "FROM base\nRUN install-tools\nARG INPUT_IDENTITY\nLABEL identity=${INPUT_IDENTITY}\n"
    assert _metadata_is_last(valid), CACHE_METADATA_RATIONALE
    for invalid in (
        valid.replace("FROM base\n", "FROM base\nARG INPUT_IDENTITY\n"),
        valid + "RUN unrelated-work\n",
        valid + "COPY source /src\n",
        valid.replace("ARG INPUT_IDENTITY\n", ""),
        "FROM base\nARG INPUT_IDENTITY\nLABEL identity=${INPUT_IDENTITY}\nRUN install-tools\n",
    ):
        assert not _metadata_is_last(invalid), CACHE_METADATA_RATIONALE


@pytest.mark.parametrize(
    "argument",
    [
        "PNPM_VERSION",
        "RUST_TOOLCHAIN",
        "RUST_TARGETS",
        "TAURI_CLI_VERSION",
        "CARGO_NEXTEST_VERSION",
        "CARGO_LLVM_COV_VERSION",
    ],
)
def test_host_tool_versions_do_not_invalidate_earlier_install_layers(argument):
    source = (PROJECT_ROOT / "build_system/docker/Dockerfile.host-builder").read_text()
    after = source.split(f"ARG {argument}\n", maxsplit=1)[1]
    first_run = re.search(r"^RUN .*?(?=\n[^ \t]|\Z)", after, re.MULTILINE | re.DOTALL)
    assert first_run and re.search(r"\$\{?" + argument + r"\b", first_run[0]), (
        f"Declare {argument} immediately before the first RUN that consumes it. "
        + CACHE_METADATA_RATIONALE
    )
