"""Prompt attribution reads only the original user protocol fields."""

import copy
import json
from pathlib import Path

import pytest

from tests.qualification.test_agent_mcp import contains_proof_prompt

TOKEN = "049b8ed775cb49ad8627b8ed953dae39"
FIXTURE = (
    Path(__file__).resolve().parents[1]
    / "fixtures/protocols/google_code_assist/agy_1_1_3_mcp_result_request.json"
)


def captured():
    return json.loads(FIXTURE.read_text())


def test_actual_google_result_request_retains_its_original_user_proof():
    payload = captured()
    assert contains_proof_prompt(payload, TOKEN)
    assert not contains_proof_prompt(payload, "different")
    # Neither the successful result nor function arguments establishes a user turn.
    payload["request"]["contents"] = payload["request"]["contents"][1:]
    assert not contains_proof_prompt(payload, TOKEN)


@pytest.mark.parametrize("role", ["model", "assistant", "system", "tool", "", None])
def test_google_proof_in_non_user_role_is_not_a_turn(role):
    payload = captured()
    payload["request"]["contents"][0]["role"] = role
    assert not contains_proof_prompt(payload, TOKEN)


@pytest.mark.parametrize(
    "change",
    [
        lambda text: text.replace("<USER_REQUEST>\n", "", 1),
        lambda text: text.replace("<USER_REQUEST>\n", "<USER_REQUEST>", 1),
        lambda text: "prefix\n" + text,
        lambda text: text.replace("\n</USER_REQUEST>", "", 1),
        lambda text: text.replace("\n</USER_REQUEST>", "\n</USER_REQUEST_WRONG>", 1),
        lambda text: text.replace("\n</USER_REQUEST>", "</USER_REQUEST>", 1),
        lambda text: text.replace("CAPSEM_MCP_PROOF=", "Narrative CAPSEM_MCP_PROOF=", 1),
        lambda text: text.replace(TOKEN + ".", TOKEN + "extra.", 1),
        lambda text: text.replace("CAPSEM_MCP_PROOF=" + TOKEN + ".", "No proof here.", 1),
        lambda text: text.replace(
            "Return only the actual text returned by that tool.", "other instruction"
        ),
        lambda text: text.replace("\n", "\r\n"),
    ],
)
def test_google_proof_rejects_malformed_or_wrong_original_wrapper(change):
    payload = captured()
    part = payload["request"]["contents"][0]["parts"][0]
    part["text"] = change(part["text"])
    assert not contains_proof_prompt(payload, TOKEN)


def test_google_proof_does_not_search_metadata_system_tools_or_nested_text():
    payload = captured()
    original = payload["request"]["contents"][0]["parts"][0]["text"]
    for replacement in [
        "<USER_REQUEST>\nNo proof.\n</USER_REQUEST>\n<ADDITIONAL_METADATA>\n"
        + original
        + "\n</ADDITIONAL_METADATA>",
        "<ADDITIONAL_METADATA>\n" + original + "\n</ADDITIONAL_METADATA>",
    ]:
        changed = copy.deepcopy(payload)
        changed["request"]["contents"][0]["parts"][0]["text"] = replacement
        changed["request"]["systemInstruction"]["parts"].append({"text": original})
        assert not contains_proof_prompt(changed, TOKEN)
    payload["request"]["contents"][0]["parts"] = [
        {"functionCall": {"args": {"text": original}}},
        {"inlineData": {"text": original}},
    ]
    assert not contains_proof_prompt(payload, TOKEN)


@pytest.mark.parametrize(
    "payload",
    [
        {"request": None},
        {"request": {"contents": None}},
        {"request": {"contents": [None]}},
        {"request": {"contents": [{"role": "user", "parts": None}]}},
        {"request": {"contents": [{"role": "user", "parts": [None, {"text": None}]}]}},
    ],
)
def test_google_proof_ignores_non_protocol_shapes(payload):
    assert not contains_proof_prompt(payload, TOKEN)


@pytest.mark.parametrize("field", ["messages", "input"])
@pytest.mark.parametrize("blocks", [True, False])
def test_existing_anthropic_openai_user_prompt_attribution_remains(field, blocks):
    text = "CAPSEM_MCP_PROOF=" + TOKEN + ". original instruction"
    content = [{"type": "text", "text": text}] if blocks else text
    payload = {field: [{"role": "user", "content": content}]}
    assert contains_proof_prompt(payload, TOKEN)
    assert not contains_proof_prompt(payload, "different")
    payload[field][0]["role"] = "assistant"
    assert not contains_proof_prompt(payload, TOKEN)
