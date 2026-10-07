"""Pure cases carried from Pierre Tholoniat's original bb61fc82d tests."""

from pathlib import Path

import inspect_capsem.containers.compose as c_compose_mod
import pytest
from inspect_capsem.containers.compose import ComposeInputs, ComposeLimits

LIMITS = ComposeLimits(65536, 4096, 64)


def test_compose_interpolation_and_dotenv(tmp_path: Path, monkeypatch: pytest.MonkeyPatch) -> None:

    class FixtureCompose:
        @staticmethod
        def parse_compose_yaml_file(path):
            files = {str(path): path.read_text()}
            dotenv = path.parent / ".env"
            if dotenv.is_file():
                files[str(dotenv)] = dotenv.read_text()
            return c_compose_mod.parse_compose_yaml_file(
                path,
                inputs=ComposeInputs(
                    environment={"BOTH_SET": "from_os_env", "EMPTY_VAR": "", "W": "wv"}, files=files
                ),
                limits=LIMITS,
            )

        @staticmethod
        def _load_dotenv(path):
            from inspect_capsem.containers.compose_inputs import InterpolationBudget

            return c_compose_mod._load_dotenv(path.read_text(), {}, InterpolationBudget(LIMITS))

    fixture = FixtureCompose()
    "Verify Compose interpolation, long-form specs, build args/target, depends_on, and launch flags."
    (tmp_path / ".env").write_text(
        '# comment\nexport DOTENV_ONLY=\'from_dotenv\'\nBOTH_SET="from_dotenv_overridden"\nINLINE_CMT=bar_val # inline comment stripped\nQUOTED_DBL_CMT="x"  # c\nQUOTED_SGL_CMT=\'q\' # c\nQUOTED_NOSPACE_CMT="x"#c\nQUOTED_TRAILING="x" trailing\nQUOTED_DOT_TAIL="x" a.b\nQUOTED_SGL_TRAILING=\'a\'b\nQUOTED_ESCQ_CMT="esc \\" q" # c\nQUOTED_ESCQ_ONLY="esc \\" q"\nQUOTED_ESC_N="a\\nb"\nQUOTED_ESC_DOLLAR="a\\$b"\nQUOTED_SGL_ESC=\'a\\nb\'\nDOT_A=a1 # comment\nDOT_B="b # keep"\nDOT_C=#onlycomment\nDOT_D=x#y\nDOT_E= #c\nDOT_F=v\t#tab\nSPACES_CMT=  v  # c\nBARE_HASH=v #\nDERIVED="${DOTENV_ONLY}/sub"\nEMPTY_VAR=\n'
    )
    monkeypatch.setenv("BOTH_SET", "from_os_env")
    monkeypatch.setenv("EMPTY_VAR", "")
    monkeypatch.setenv("W", "wv")
    monkeypatch.delenv("UNSET_VAR", raising=False)
    monkeypatch.delenv("U", raising=False)
    monkeypatch.delenv("V", raising=False)
    compose_interp = tmp_path / "compose-interp.yaml"
    compose_interp.write_text(
        "services:\n  app:\n    image: myrepo/${DOTENV_ONLY}:${BOTH_SET}\n    environment:\n      DEF_COLON: ${EMPTY_VAR:-colon_fallback}\n      DEF_PLAIN_EMPTY: ${EMPTY_VAR-plain_fallback}\n      DEF_PLAIN_UNSET: ${UNSET_VAR-unset_fallback}\n      ALT_COLON_SET: ${W:+alt_w}\n      ALT_COLON_EMPTY: ${EMPTY_VAR:+alt_empty}\n      ALT_COLON_UNSET: ${U:+alt_u}\n      ALT_PLAIN_SET: ${W+alt_plain_w}\n      ALT_PLAIN_EMPTY: ${EMPTY_VAR+alt_plain_empty}\n      ALT_PLAIN_UNSET: ${U+alt_plain_u}\n      NESTED_DEF: ${U:-${V:-inner}}\n      BARE_VAR: $W\n      ESCAPED_BARE: $$W\n      ESCAPED: $$LITERAL_DOLLAR\n      FROM_INLINE: ${INLINE_CMT}\n      FROM_QUOTED_DBL_CMT: ${QUOTED_DBL_CMT}\n      FROM_QUOTED_SGL_CMT: ${QUOTED_SGL_CMT}\n      FROM_QUOTED_NOSPACE_CMT: ${QUOTED_NOSPACE_CMT}\n      FROM_QUOTED_TRAILING: ${QUOTED_TRAILING}\n      FROM_QUOTED_DOT_TAIL: ${QUOTED_DOT_TAIL}\n      FROM_QUOTED_SGL_TRAILING: ${QUOTED_SGL_TRAILING}\n      FROM_QUOTED_ESCQ_CMT: ${QUOTED_ESCQ_CMT}\n      FROM_QUOTED_ESCQ_ONLY: ${QUOTED_ESCQ_ONLY}\n      FROM_QUOTED_ESC_N: ${QUOTED_ESC_N}\n      FROM_QUOTED_ESC_DOLLAR: ${QUOTED_ESC_DOLLAR}\n      FROM_QUOTED_SGL_ESC: ${QUOTED_SGL_ESC}\n      FROM_DOT_A: ${DOT_A}\n      FROM_DOT_B: ${DOT_B}\n      FROM_DOT_C: ${DOT_C}\n      FROM_DOT_D: ${DOT_D}\n      FROM_DOT_E: ${DOT_E}\n      FROM_DOT_F: ${DOT_F}\n      FROM_SPACES_CMT: ${SPACES_CMT}\n      FROM_BARE_HASH: ${BARE_HASH}\n      FROM_DERIVED: ${DERIVED}\n"
    )
    parsed_interp = fixture.parse_compose_yaml_file(compose_interp)
    svc_app = parsed_interp["services"]["app"]
    assert svc_app["image"] == "myrepo/from_dotenv:from_os_env"
    assert svc_app["environment"] == {
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
        "BARE_VAR": "wv",
        "ESCAPED_BARE": "$W",
        "ESCAPED": "$LITERAL_DOLLAR",
        "FROM_INLINE": "bar_val",
        "FROM_QUOTED_DBL_CMT": "x",
        "FROM_QUOTED_SGL_CMT": "q",
        "FROM_QUOTED_NOSPACE_CMT": "x",
        "FROM_QUOTED_TRAILING": "x",
        "FROM_QUOTED_DOT_TAIL": "x",
        "FROM_QUOTED_SGL_TRAILING": "a",
        "FROM_QUOTED_ESCQ_CMT": 'esc " q',
        "FROM_QUOTED_ESCQ_ONLY": 'esc " q',
        "FROM_QUOTED_ESC_N": "a\nb",
        "FROM_QUOTED_ESC_DOLLAR": "a$b",
        "FROM_QUOTED_SGL_ESC": "a\\nb",
        "FROM_DOT_A": "a1",
        "FROM_DOT_B": "b # keep",
        "FROM_DOT_C": "#onlycomment",
        "FROM_DOT_D": "x#y",
        "FROM_DOT_E": "#c",
        "FROM_DOT_F": "v\t#tab",
        "FROM_SPACES_CMT": "v",
        "FROM_BARE_HASH": "v",
        "FROM_DERIVED": "from_dotenv/sub",
    }
    for bad_dotenv in ('K="multi\n', 'K="multi\\\n', "K='multi\n"):
        unterm_path = tmp_path / ".env.unterm"
        unterm_path.write_text(bad_dotenv)
        with pytest.raises(ValueError, match=r"Unterminated .*quoted value"):
            fixture._load_dotenv(unterm_path)
    for bad_tail in (
        'K="x" "y"\n',
        "K='x' 'y'\n",
        'K="x" OTHER=1\n',
        'K="x" foo bar\n',
        'K="x" w # c\n',
    ):
        tail_path = tmp_path / ".env.tail"
        tail_path.write_text(bad_tail)
        with pytest.raises(ValueError, match="Unexpected trailing characters"):
            fixture._load_dotenv(tail_path)
    compose_err_colon = tmp_path / "compose-err-colon.yaml"
    compose_err_colon.write_text("services:\n  app:\n    image: ${EMPTY_VAR:?must not be empty}\n")
    with pytest.raises(ValueError, match="must not be empty"):
        fixture.parse_compose_yaml_file(compose_err_colon)
    compose_err_plain = tmp_path / "compose-err-plain.yaml"
    compose_err_plain.write_text("services:\n  app:\n    image: ${UNSET_VAR?must be set}\n")
    with pytest.raises(ValueError, match="must be set"):
        fixture.parse_compose_yaml_file(compose_err_plain)
    compose_empty_ok = tmp_path / "compose-empty-ok.yaml"
    compose_empty_ok.write_text("services:\n  app:\n    image: alpine:3.20${EMPTY_VAR?err}\n")
    assert (
        fixture.parse_compose_yaml_file(compose_empty_ok)["services"]["app"]["image"]
        == "alpine:3.20"
    )
    for bad_expr in ("${U", "prefix-${U-unclosed", "${A B}", "${}"):
        bad_compose = tmp_path / "compose-bad-interp.yaml"
        bad_compose.write_text(f"services:\n  app:\n    image: {bad_expr}\n")
        with pytest.raises(ValueError, match="Invalid Compose variable interpolation"):
            fixture.parse_compose_yaml_file(bad_compose)
