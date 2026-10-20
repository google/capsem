"""Original final-stage metadata cases with explicit text input."""

import inspect_capsem.containers.dockerfile as dockerfile_mod


def test_dockerfile_workdir_extraction_and_resolution() -> None:

    def _defaults(text: str) -> dict[str, str]:
        workdir, user = dockerfile_mod._extract_dockerfile_metadata(text)
        return {
            key: value
            for key, value in [("working_dir", workdir), ("user", user)]
            if value is not None
        }

    assert _defaults("FROM debian:12\nRUN echo hi\n") == {}
    assert _defaults("FROM debian:12\nUSER developer\nUSER root\n") == {}
    assert _defaults("FROM debian:12 AS base\nUSER developer\nFROM base\n") == {"user": "developer"}
    assert _defaults("FROM ubuntu:24.04\nWORKDIR /app\nWORKDIR src\n") == {
        "working_dir": "/app/src"
    }
    assert _defaults("FROM ubuntu:24.04\nWORKDIR relative\n") == {"working_dir": "/relative"}
    assert _defaults(
        'ARG ROOT_DIR=/opt\nFROM ubuntu:24.04 AS base\nARG ROOT_DIR\nENV SUB_DIR=project\nENV EXTRA sub/dir\nWORKDIR "${ROOT_DIR}/${SUB_DIR}"\nFROM base AS final\nWORKDIR ${EXTRA:-fallback}\nWORKDIR $UNSET_VAR\n'
    ) == {"working_dir": "/opt/project/sub/dir"}
    assert (
        _defaults(
            "FROM ubuntu:24.04 AS builder\nWORKDIR /build\nFROM debian:12\nCOPY --from=builder /build /out\n"
        )
        == {}
    )
