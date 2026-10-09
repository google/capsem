"""Compose `.env` loading, `SAMPLE_METADATA_*`, and operator host-env allowlist tests."""

from __future__ import annotations

import logging
from pathlib import Path

import inspect_capsem._compose as compose_mod
import inspect_capsem.containers.compose as c_compose_mod
import inspect_capsem.sandbox as sb_mod
import pytest
from inspect_capsem import CapsemSandboxConfig, CapsemSandboxEnvironment
from inspect_capsem.containers.compose_inputs import (
    DEFAULT_COMPOSE_LIMITS,
    ComposeLimits,
    InterpolationBudget,
)
from inspect_capsem.containers.compose_interpolation import _load_dotenv

from ..helpers import Scripted


def test_compose_variable_interpolation_and_dotenv(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, caplog: pytest.LogCaptureFixture
) -> None:
    """Verify `.env` + operator `CAPSEM_INSPECT_ALLOWED_HOST_ENV` + `${VAR:-/:/?/+}` + `$$`."""
    (tmp_path / ".env").write_text(
        "# comment\nexport DOTENV_ONLY='from_dotenv'\nBOTH_SET=\"from_dotenv_overridden\"\n"
        'INLINE_CMT=bar_val # c\nQUOTED_DBL="esc \\" a\\nb a\\$b # keep" # c\n'
        "QUOTED_SGL='a\\nb' # c\nDOT_D=x#y\nDERIVED=\"${DOTENV_ONLY}/sub\"\nEMPTY_VAR=\n"
    )
    for k, v in (
        ("BOTH_SET", "from_os_env"),
        ("EMPTY_VAR", ""),
        ("W", "wv"),
        ("BARE_HOST_ONLY", "from_os_bare"),
    ):
        monkeypatch.setenv(k, v)
    for k in ("CAPSEM_INSPECT_ALLOWED_HOST_ENV", "UNSET_VAR", "U", "V"):
        monkeypatch.delenv(k, raising=False)

    compose_interp = tmp_path / "compose-interp.yaml"
    compose_interp.write_text(
        "services:\n  app:\n    image: myrepo/${DOTENV_ONLY}:${BOTH_SET}\n    environment:\n"
        "      DEF_COLON: ${EMPTY_VAR:-colon_fallback}\n"
        "      DEF_PLAIN_EMPTY: ${EMPTY_VAR-plain_fallback}\n"
        "      DEF_PLAIN_UNSET: ${UNSET_VAR-unset_fallback}\n      ALT_COLON_SET: ${W:+alt_w}\n"
        "      ALT_COLON_EMPTY: ${EMPTY_VAR:+alt_empty}\n      ALT_COLON_UNSET: ${U:+alt_u}\n"
        "      ALT_PLAIN_SET: ${W+alt_plain_w}\n"
        "      ALT_PLAIN_EMPTY: ${EMPTY_VAR+alt_plain_empty}\n"
        "      ALT_PLAIN_UNSET: ${U+alt_plain_u}\n      NESTED_DEF: ${U:-${V:-inner}}\n"
        "      BARE_VAR: $BARE_HOST_ONLY\n"
        "      ESCAPED_BARE: $$W\n      ESCAPED: $$LITERAL_DOLLAR\n"
        "      FROM_INLINE: ${INLINE_CMT}\n      FROM_QUOTED_DBL: ${QUOTED_DBL}\n"
        "      FROM_QUOTED_SGL: ${QUOTED_SGL}\n      FROM_DOT_D: ${DOT_D}\n"
        "      FROM_DERIVED: ${DERIVED}\n"
    )
    with caplog.at_level(logging.WARNING):
        parsed_default = c_compose_mod.parse_host_compose_yaml_file(
            compose_interp, allowed_host_env=("BOTH_SET", "W", "BARE_HOST_ONLY")
        )
    assert parsed_default["services"]["app"]["image"] == "myrepo/from_dotenv:from_dotenv_overridden"
    assert parsed_default["services"]["app"]["environment"]["BARE_VAR"] == ""
    assert "Ignoring non-allowlisted host environment variable 'W'" in caplog.text
    assert "'DOTENV_ONLY'" not in caplog.text

    monkeypatch.setenv("CAPSEM_INSPECT_ALLOWED_HOST_ENV", "BOTH_SET,EMPTY_VAR,W,BARE_HOST_ONLY")
    svc_app = c_compose_mod.parse_host_compose_yaml_file(compose_interp)["services"]["app"]
    assert svc_app["image"] == "myrepo/from_dotenv:from_os_env"
    expected_env = {
        "DEF_COLON": "colon_fallback",
        "DEF_PLAIN_EMPTY": "",
        "DEF_PLAIN_UNSET": "unset_fallback",
        "ALT_COLON_SET": "alt_w",
        "ALT_COLON_EMPTY": "",
        "ALT_COLON_UNSET": "",
        "ALT_PLAIN_SET": "alt_plain_w",
        "ALT_PLAIN_EMPTY": "alt_plain_empty",
        "ALT_PLAIN_UNSET": "",
        "NESTED_DEF": "inner",
        "BARE_VAR": "from_os_bare",
        "ESCAPED_BARE": "$W",
        "ESCAPED": "$LITERAL_DOLLAR",
        "FROM_INLINE": "bar_val",
        "FROM_QUOTED_DBL": 'esc " a\nb a$b # keep',
        "FROM_QUOTED_SGL": "a\\nb",
        "FROM_DOT_D": "x#y",
        "FROM_DERIVED": "from_dotenv/sub",
    }
    assert svc_app["environment"] == expected_env

    narrowed = c_compose_mod.parse_host_compose_yaml_file(
        compose_interp, allowed_host_env=("BOTH_SET",)
    )
    assert narrowed["services"]["app"]["image"] == "myrepo/from_dotenv:from_os_env"
    assert narrowed["services"]["app"]["environment"]["ALT_COLON_SET"] == ""
    assert narrowed["services"]["app"]["environment"]["BARE_VAR"] == ""

    for bad_pat in ("*", "**", "?*", "[*]", "[A-Z]*", "[!_]*", "*SUFFIX"):
        with pytest.raises(ValueError, match="Wildcard-only or invalid pattern"):
            CapsemSandboxConfig(allowed_host_env=(bad_pat,))
        monkeypatch.setenv("CAPSEM_INSPECT_ALLOWED_HOST_ENV", bad_pat)
        with pytest.raises(ValueError, match="Wildcard-only or invalid pattern"):
            c_compose_mod.parse_host_compose_yaml_file(compose_interp)


def test_dotenv_syntax_errors_and_required_var_checks(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch
) -> None:
    monkeypatch.setenv("W", "wv")
    monkeypatch.delenv("CAPSEM_INSPECT_ALLOWED_HOST_ENV", raising=False)
    for bad_dotenv in ('K="multi\n', 'K="multi\\\n', "K='multi\n"):
        with pytest.raises(ValueError, match=r"Unterminated .*quoted value") as exc_info:
            _load_dotenv(bad_dotenv, {}, InterpolationBudget(DEFAULT_COMPOSE_LIMITS))
        assert "multi" not in str(exc_info.value)
    for bad_tail in (
        'K="secret_v" "y"\n',
        "K='secret_v' 'y'\n",
        'K="secret_v" OTHER=1\n',
        'K="secret_v" w # c\n',
    ):
        with pytest.raises(ValueError, match="Unexpected trailing characters") as exc_info:
            _load_dotenv(bad_tail, {}, InterpolationBudget(DEFAULT_COMPOSE_LIMITS))
        assert "secret_v" not in str(exc_info.value)

    (tmp_path / ".env").write_text("EMPTY_VAR=\nSECRET_VAL=top_secret_token_999\n")
    (tmp_path / "c1.yaml").write_text(
        "services:\n  app:\n    image: ${EMPTY_VAR:?must not be empty}\n"
    )
    with pytest.raises(ValueError, match="must not be empty"):
        compose_mod.coerce_config(str(tmp_path / "c1.yaml"))
    (tmp_path / "c_redact.yaml").write_text(
        "services:\n  app:\n    image: ${EMPTY_VAR:?err ${SECRET_VAL}}\n"
    )
    with pytest.raises(ValueError, match="Required Compose variable 'EMPTY_VAR'") as exc_info:
        compose_mod.coerce_config(str(tmp_path / "c_redact.yaml"))
    assert "top_secret_token_999" not in str(exc_info.value)

    for expr in ("${W:?need W}", "${W?need W}", "${W:?}"):
        (tmp_path / "c2.yaml").write_text(f"services:\n  app:\n    image: {expr}\n")
        with pytest.raises(ValueError, match=r"not allowlisted.*CAPSEM_INSPECT_ALLOWED_HOST_ENV"):
            compose_mod.coerce_config(str(tmp_path / "c2.yaml"))
    for bad_expr in ("${U", "prefix-${U-unclosed", "${A B}", "${}"):
        (tmp_path / "c3.yaml").write_text(f"services:\n  app:\n    image: {bad_expr}\n")
        with pytest.raises(ValueError, match="Invalid Compose variable interpolation"):
            compose_mod.coerce_config(str(tmp_path / "c3.yaml"))

    (tmp_path / "c_small.yaml").write_text("services:\n  app:\n    image: alpine:3.20\n")
    limits = ComposeLimits(maximum_bytes=10, maximum_nodes=64, maximum_depth=16)
    with pytest.raises(ValueError, match="Compose input byte limit exceeded"):
        c_compose_mod.parse_host_compose_yaml_file(tmp_path / "c_small.yaml", limits=limits)


async def test_cybergym_sample_metadata_compose_interpolation(
    tmp_path: Path, monkeypatch: pytest.MonkeyPatch, caplog: pytest.LogCaptureFixture
) -> None:
    """`SAMPLE_METADATA_*` interpolates idempotently from `metadata`, not `.env` or `os.environ`."""
    monkeypatch.setenv("SAMPLE_METADATA_HOST_LEAK", "leaked_from_host")
    monkeypatch.setenv("CAPSEM_INSPECT_ALLOWED_HOST_ENV", "SAMPLE_*")
    (tmp_path / ".env").write_text("SAMPLE_METADATA_HOST_LEAK=leaked_from_dotenv\n")
    compose_path = tmp_path / "compose.yaml"
    compose_path.write_text(
        "services:\n  default:\n    image: ${SAMPLE_METADATA_IMAGE_VULNERABLE}\n    environment:\n"
        "      - EXECUTOR_COMMAND=${SAMPLE_METADATA_EXECUTOR_COMMAND}\n      - BARE_KEY\n"
        "      - SAMPLE_METADATA_HOST_LEAK\n      - LEAK_INTERP=${SAMPLE_METADATA_HOST_LEAK}\n"
    )
    pre_coerced = compose_mod.coerce_config(str(compose_path))
    ctrl = Scripted()
    monkeypatch.setattr(sb_mod, "SdkCapsemController", lambda: ctrl)
    metadata = {
        "image_vulnerable": "cybergym/vuln-target:1.2",
        "executor command": "/usr/local/bin/run-poc --fast",
        "bare_key": "ignored_without_prefix",
    }
    with caplog.at_level(logging.WARNING):
        envs = await CapsemSandboxEnvironment.sample_init("cybergym_task", pre_coerced, metadata)
    try:
        assert "Ignoring non-allowlisted host environment variable" not in caplog.text
        assert ctrl.started[0]["image"] == "cybergym/vuln-target:1.2"
        assert ctrl.started[0]["env"] == {
            "EXECUTOR_COMMAND": "/usr/local/bin/run-poc --fast",
            "BARE_KEY": "",
            "SAMPLE_METADATA_HOST_LEAK": "",
            "LEAK_INTERP": "",
        }
    finally:
        await CapsemSandboxEnvironment.sample_cleanup("cybergym_task", None, envs, False)
