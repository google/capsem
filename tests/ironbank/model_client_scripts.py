"""Composable in-VM model client scripts for Ironbank tests."""

from __future__ import annotations

import json
import textwrap

from ironbank.model_client_config import (
    HERMETIC_AGY_MODEL,
    HERMETIC_AGY_MODEL_DISPLAY,
    HERMETIC_ANTHROPIC_MODEL,
    HERMETIC_GEMINI_MODEL,
    HERMETIC_OPENAI_COMPAT_MODEL,
    HERMETIC_OPENAI_PRICED_MODEL,
    LIVE_OPENAI_RESPONSES_MODEL,
)


def common_result_script_prelude(base_url: str, filename_prefix: str) -> str:
    return f"""
import json
import os
from pathlib import Path
import socket
import subprocess
import urllib.parse
import urllib.request
import uuid

BASE_URL = {json.dumps(base_url.rstrip("/"))}
BASE_DOMAIN = urllib.parse.urlparse(BASE_URL).hostname or ""
HERMETIC_OPENAI_COMPAT_MODEL = {json.dumps(HERMETIC_OPENAI_COMPAT_MODEL)}
HERMETIC_OPENAI_PRICED_MODEL = {json.dumps(HERMETIC_OPENAI_PRICED_MODEL)}
HERMETIC_ANTHROPIC_MODEL = {json.dumps(HERMETIC_ANTHROPIC_MODEL)}
HERMETIC_GEMINI_MODEL = {json.dumps(HERMETIC_GEMINI_MODEL)}
HERMETIC_AGY_MODEL = {json.dumps(HERMETIC_AGY_MODEL)}
HERMETIC_AGY_MODEL_DISPLAY = {json.dumps(HERMETIC_AGY_MODEL_DISPLAY)}
LIVE_OPENAI_RESPONSES_MODEL = {json.dumps(LIVE_OPENAI_RESPONSES_MODEL)}
DNS_QNAME = "model.capsem.test"
DNS_IP = socket.gethostbyname(DNS_QNAME)
NONCE = uuid.uuid4().hex
FILENAME = {json.dumps(filename_prefix)} + "-" + uuid.uuid4().hex + ".txt"
TARGET = "/root/" + FILENAME
PROMPT = "Write uuid4 hex value " + NONCE + " to " + TARGET + "."

def run_tool(arguments):
    command = arguments.get("cmd") or arguments.get("command")
    if command:
        completed = subprocess.run(
            command,
            shell=True,
            cwd="/root",
            capture_output=True,
            text=True,
            timeout=30,
        )
        return "Process exited with code " + str(completed.returncode)
    path = arguments.get("file_path")
    content = arguments.get("content")
    if path and content is not None:
        Path(path).write_text(content, encoding="utf-8")
        return "Process exited with code 0"
    raise RuntimeError("unsupported tool args: " + json.dumps(arguments, sort_keys=True))

def emit_result(provider, domain, path, model, output, reasoning, tool_call_name, call_args, call_response, credential_provider=None, credential_source=None):
    file_text = Path(TARGET).read_text(encoding="utf-8")
    result = {{
        "input": PROMPT,
        "reasoning": reasoning,
        "output": output,
        "tool_call_name": tool_call_name,
        "call_args": call_args,
        "call_response": call_response,
        "provider": provider,
        "credential_provider": credential_provider or provider,
        "credential_source": credential_source,
        "domain": domain,
        "path": path,
        "model": model,
        "target": TARGET,
        "filename": FILENAME,
        "nonce": NONCE,
        "file_text": file_text,
        "file_matches": file_text == NONCE + "\\n",
        "output_contains_nonce": NONCE in output,
        "dns_qname": DNS_QNAME,
        "dns_ip": DNS_IP,
    }}
    print("IRONBANK_CLIENT_RESULT=" + json.dumps(result, sort_keys=True))

def add_openai_auth(headers):
    token = "sk-" + NONCE
    headers["authorization"] = "Bearer " + token
    return token

def add_anthropic_auth(headers):
    token = "sk-ant-" + NONCE
    headers["x-api-key"] = token
    return token

def add_google_auth(headers):
    token = "AIza" + NONCE
    headers["x-goog-api-key"] = token
    return token
"""


def openai_responses_api_script(base_url: str) -> str:
    return textwrap.dedent(
        common_result_script_prelude(base_url, "openai-api")
        + r'''
def parse_sse(body):
    events = []
    for line in body.splitlines():
        if line.startswith("data: ") and line[6:] != "[DONE]":
            events.append(json.loads(line[6:]))
    return events

def post(body):
    headers = {"content-type": "application/json"}
    add_openai_auth(headers)
    req = urllib.request.Request(
        BASE_URL + "/v1/responses",
        data=json.dumps(body).encode(),
        headers=headers,
        method="POST",
    )
    with urllib.request.urlopen(req, timeout=60) as response:
        return response.read().decode()

first_body = {
    "model": HERMETIC_OPENAI_PRICED_MODEL,
    "stream": True,
    "input": PROMPT,
    "tools": [{"type": "function", "name": "exec_command"}],
}
first_events = parse_sse(post(first_body))
tool_item = next(event["item"] for event in first_events if event.get("type") == "response.output_item.done")
call_args = json.loads(tool_item["arguments"])
call_response = run_tool(call_args)
second_body = {
    "model": HERMETIC_OPENAI_PRICED_MODEL,
    "stream": True,
    "input": [
        {"type": "function_call", "call_id": tool_item["call_id"], "name": tool_item["name"], "arguments": tool_item["arguments"]},
        {"type": "function_call_output", "call_id": tool_item["call_id"], "output": call_response},
        {"role": "user", "content": PROMPT},
    ],
    "tools": [{"type": "function", "name": "exec_command"}],
}
second_events = parse_sse(post(second_body))
output = next(event["text"] for event in second_events if event.get("type") == "response.output_text.done")
reasoning = next(event["delta"] for event in second_events if event.get("type") == "response.reasoning_summary_text.delta")
emit_result("openai", BASE_DOMAIN, "/v1/responses", HERMETIC_OPENAI_PRICED_MODEL, output, reasoning, tool_item["name"], call_args, call_response)
'''
    ).strip()


def openai_embeddings_and_image_script(base_url: str) -> str:
    return textwrap.dedent(
        common_result_script_prelude(base_url, "openai-extra")
        + r'''
def post(path, body):
    headers = {"content-type": "application/json"}
    raw_secret = add_openai_auth(headers)
    req = urllib.request.Request(
        BASE_URL + path,
        data=json.dumps(body).encode(),
        headers=headers,
        method="POST",
    )
    with urllib.request.urlopen(req, timeout=60) as response:
        return raw_secret, json.loads(response.read().decode())

embedding_input = "Embed Capsem ledger nonce " + NONCE
raw_secret, embedding = post("/v1/embeddings", {
    "model": "text-embedding-3-small",
    "input": embedding_input,
})
image_prompt = "Draw a small ledger mark for " + NONCE
_, image = post("/v1/images/generations", {
    "model": "gpt-5-image-mini",
    "prompt": image_prompt,
    "size": "256x256",
    "response_format": "b64_json",
})
print("IRONBANK_CLIENT_RESULT=" + json.dumps({
    "provider": "openai",
    "domain": BASE_DOMAIN,
    "embedding_path": "/v1/embeddings",
    "embedding_model": "text-embedding-3-small",
    "embedding_input": embedding_input,
    "embedding_vector": embedding["data"][0]["embedding"],
    "image_path": "/v1/images/generations",
    "image_model": "gpt-5-image-mini",
    "image_prompt": image_prompt,
    "image_b64": image["data"][0]["b64_json"],
    "credential_nonce": NONCE,
}, sort_keys=True))
'''
    ).strip()


def gemini_api_script(base_url: str) -> str:
    return textwrap.dedent(
        common_result_script_prelude(base_url, "gemini-api")
        + r'''
def parse_sse(body):
    events = []
    for line in body.splitlines():
        if line.startswith("data: ") and line[6:] != "[DONE]":
            events.append(json.loads(line[6:]))
    return events

def post(path, body, *, stream=False):
    headers = {"content-type": "application/json"}
    add_google_auth(headers)
    req = urllib.request.Request(
        BASE_URL + path,
        data=json.dumps(body).encode(),
        headers=headers,
        method="POST",
    )
    with urllib.request.urlopen(req, timeout=60) as response:
        raw = response.read().decode()
    return parse_sse(raw) if stream else json.loads(raw)

stream_path = "/v1beta/models/" + HERMETIC_GEMINI_MODEL + ":streamGenerateContent"
generate_path = "/v1beta/models/" + HERMETIC_GEMINI_MODEL + ":generateContent"
tool_declaration = {
    "functionDeclarations": [
        {
            "name": "write_to_file",
            "description": "Write deterministic fixture text to disk.",
            "parameters": {
                "type": "object",
                "properties": {
                    "TargetFile": {"type": "string"},
                    "Content": {"type": "string"},
                },
                "required": ["TargetFile", "Content"],
            },
        }
    ]
}
first_body = {
    "contents": [{"role": "user", "parts": [{"text": PROMPT}]}],
    "tools": [tool_declaration],
}
first_events = post(stream_path + "?alt=sse", first_body, stream=True)
function_call = next(
    part["functionCall"]
    for event in first_events
    for candidate in event.get("candidates", [])
    for part in candidate.get("content", {}).get("parts", [])
    if "functionCall" in part
)
call_args = function_call["args"]
Path(call_args["TargetFile"]).write_text(call_args["Content"], encoding="utf-8")
call_response = "Process exited with code 0"
second_body = {
    "contents": [
        {"role": "user", "parts": [{"text": PROMPT}]},
        {"role": "model", "parts": [{"functionCall": function_call}]},
        {
            "role": "function",
            "parts": [
                {
                    "functionResponse": {
                        "name": function_call["name"],
                        "response": {"content": call_response},
                    }
                }
            ],
        },
    ],
    "tools": [tool_declaration],
}
second_events = post(stream_path + "?alt=sse", second_body, stream=True)
final_parts = [
    part
    for event in second_events
    for candidate in event.get("candidates", [])
    for part in candidate.get("content", {}).get("parts", [])
]
reasoning = next((part["text"] for part in final_parts if part.get("thought") is True), "")
output = next(part["text"] for part in final_parts if "text" in part and part.get("thought") is not True)
nonstream = post(generate_path, {"contents": [{"role": "user", "parts": [{"text": PROMPT}]}]})
print("IRONBANK_CLIENT_RESULT=" + json.dumps({
    "input": PROMPT,
    "reasoning": reasoning,
    "output": output,
    "tool_call_name": function_call["name"],
    "call_args": call_args,
    "call_response": call_response,
    "provider": "google",
    "credential_provider": "google",
    "domain": BASE_DOMAIN,
    "path": stream_path,
    "model": HERMETIC_GEMINI_MODEL,
    "target": TARGET,
    "filename": FILENAME,
    "nonce": NONCE,
    "file_text": Path(TARGET).read_text(encoding="utf-8"),
    "file_matches": Path(TARGET).read_text(encoding="utf-8") == NONCE + "\n",
    "output_contains_nonce": NONCE in output,
    "dns_qname": DNS_QNAME,
    "dns_ip": DNS_IP,
    "nonstream_path": generate_path,
    "nonstream_text": nonstream["candidates"][0]["content"]["parts"][0]["text"],
    "nonstream_model": nonstream["modelVersion"],
}, sort_keys=True))
'''
    ).strip()


def live_openai_responses_api_script() -> str:
    return textwrap.dedent(
        common_result_script_prelude("https://api.openai.com", "live-openai-api")
        + r'''
from openai import OpenAI

MODEL = os.environ.get("CAPSEM_LIVE_OPENAI_RESPONSE_MODEL", LIVE_OPENAI_RESPONSES_MODEL)
client = OpenAI(api_key=os.environ["OPENAI_API_KEY"])
tools = [{
    "type": "function",
    "name": "exec_command",
    "description": "Write the requested UUID value to the requested file.",
    "parameters": {
        "type": "object",
        "properties": {
            "cmd": {"type": "string"},
        },
        "required": ["cmd"],
    },
}]

first = client.responses.create(
    model=MODEL,
    input=PROMPT,
    tools=tools,
    tool_choice={"type": "function", "name": "exec_command"},
    reasoning={"effort": "minimal"},
    max_output_tokens=1024,
)
tool_item = next(item for item in first.output if getattr(item, "type", None) == "function_call")
call_args = json.loads(tool_item.arguments)
call_response = run_tool(call_args)
second = client.responses.create(
    model=MODEL,
    input=[
        {
            "type": "function_call",
            "call_id": tool_item.call_id,
            "name": tool_item.name,
            "arguments": tool_item.arguments,
        },
        {
            "type": "function_call_output",
            "call_id": tool_item.call_id,
            "output": call_response,
        },
        {
            "role": "user",
            "content": "Return exactly the uuid4 hex value that was written to disk.",
        },
    ],
    tools=tools,
    reasoning={"effort": "minimal"},
    max_output_tokens=1024,
)
output = second.output_text.strip()
if NONCE not in output:
    raise SystemExit("live OpenAI output did not contain nonce: " + output)
reasoning = ""
for item in getattr(second, "output", []) or []:
    if getattr(item, "type", None) == "reasoning":
        summary = getattr(item, "summary", None) or []
        if summary:
            reasoning = " ".join(str(getattr(part, "text", "")) for part in summary).strip()
emit_result("openai", "api.openai.com", "/v1/responses", MODEL, output, reasoning, tool_item.name, call_args, call_response)
'''
    ).strip()


def live_openai_chat_completions_script() -> str:
    return textwrap.dedent(
        common_result_script_prelude("https://api.openai.com", "live-openai-chat")
        + r'''
from openai import OpenAI

MODEL = os.environ.get("CAPSEM_LIVE_OPENAI_CHAT_MODEL", LIVE_OPENAI_RESPONSES_MODEL)
client = OpenAI(api_key=os.environ["OPENAI_API_KEY"])
tools = [{
    "type": "function",
    "function": {
        "name": "exec_command",
        "description": "Write the requested UUID value to the requested file.",
        "parameters": {
            "type": "object",
            "properties": {
                "cmd": {"type": "string"},
            },
            "required": ["cmd"],
        },
    },
}]

first = client.chat.completions.create(
    model=MODEL,
    messages=[{"role": "user", "content": PROMPT}],
    tools=tools,
    tool_choice={"type": "function", "function": {"name": "exec_command"}},
    max_completion_tokens=1024,
)
tool_item = first.choices[0].message.tool_calls[0]
call_args = json.loads(tool_item.function.arguments)
call_response = run_tool(call_args)
second = client.chat.completions.create(
    model=MODEL,
    messages=[
        {"role": "user", "content": PROMPT},
        first.choices[0].message.model_dump(exclude_none=True),
        {
            "role": "tool",
            "tool_call_id": tool_item.id,
            "content": call_response,
        },
        {
            "role": "user",
            "content": "Return exactly the uuid4 hex value that was written to disk.",
        },
    ],
    tools=tools,
    max_completion_tokens=1024,
)
output = (second.choices[0].message.content or "").strip()
if NONCE not in output:
    raise SystemExit("live OpenAI chat output did not contain nonce: " + output)
emit_result("openai", "api.openai.com", "/v1/chat/completions", MODEL, output, "", tool_item.function.name, call_args, call_response)
'''
    ).strip()


def openai_two_tool_calls_script(base_url: str) -> str:
    return textwrap.dedent(
        common_result_script_prelude(base_url, "openai-two")
        + r'''
def parse_sse(body):
    events = []
    for line in body.splitlines():
        if line.startswith("data: ") and line[6:] != "[DONE]":
            events.append(json.loads(line[6:]))
    return events

def post(body):
    headers = {"content-type": "application/json"}
    add_openai_auth(headers)
    req = urllib.request.Request(
        BASE_URL + "/v1/responses",
        data=json.dumps(body).encode(),
        headers=headers,
        method="POST",
    )
    with urllib.request.urlopen(req, timeout=60) as response:
        return response.read().decode()

def run_one(index):
    nonce = uuid.uuid4().hex
    filename = "openai-two-" + uuid.uuid4().hex + ".txt"
    target = "/root/" + filename
    prompt = "Write uuid4 hex value " + nonce + " to " + target + "."
    first_events = parse_sse(post({
        "model": HERMETIC_OPENAI_PRICED_MODEL,
        "stream": True,
        "input": prompt,
        "tools": [{"type": "function", "name": "exec_command"}],
    }))
    tool_item = next(event["item"] for event in first_events if event.get("type") == "response.output_item.done")
    call_args = json.loads(tool_item["arguments"])
    call_response = run_tool(call_args)
    second_events = parse_sse(post({
        "model": HERMETIC_OPENAI_PRICED_MODEL,
        "stream": True,
        "input": [
            {"type": "function_call", "call_id": tool_item["call_id"], "name": tool_item["name"], "arguments": tool_item["arguments"]},
            {"type": "function_call_output", "call_id": tool_item["call_id"], "output": call_response},
            {"role": "user", "content": prompt},
        ],
        "tools": [{"type": "function", "name": "exec_command"}],
    }))
    output = next(event["text"] for event in second_events if event.get("type") == "response.output_text.done")
    reasoning = next(event["delta"] for event in second_events if event.get("type") == "response.reasoning_summary_text.delta")
    file_text = Path(target).read_text(encoding="utf-8")
    return {
        "index": index,
        "input": prompt,
        "reasoning": reasoning,
        "output": output,
        "tool_call_name": tool_item["name"],
        "call_id": tool_item["call_id"],
        "call_args": call_args,
        "call_response": call_response,
        "filename": filename,
        "target": target,
        "nonce": nonce,
        "file_matches": file_text == nonce + "\n",
    }

results = [run_one(1), run_one(2)]
print("IRONBANK_CLIENT_RESULT=" + json.dumps({
    "provider": "openai",
    "domain": BASE_DOMAIN,
    "path": "/v1/responses",
    "model": HERMETIC_OPENAI_PRICED_MODEL,
    "dns_qname": DNS_QNAME,
    "dns_ip": DNS_IP,
    "credential_nonce": NONCE,
    "results": results,
}, sort_keys=True))
'''
    ).strip()


def claude_api_script(base_url: str) -> str:
    return textwrap.dedent(
        common_result_script_prelude(base_url, "claude-api")
        + r'''
def post(body):
    headers = {"content-type": "application/json", "anthropic-version": "2023-06-01"}
    add_anthropic_auth(headers)
    req = urllib.request.Request(
        BASE_URL + "/v1/messages",
        data=json.dumps(body).encode(),
        headers=headers,
        method="POST",
    )
    with urllib.request.urlopen(req, timeout=60) as response:
        return json.loads(response.read().decode())

first = post({
    "model": HERMETIC_ANTHROPIC_MODEL,
    "max_tokens": 128,
    "messages": [{"role": "user", "content": PROMPT}],
    "tools": [{"name": "exec_command", "description": "run a command", "input_schema": {"type": "object", "properties": {"cmd": {"type": "string"}}}}],
})
tool_item = next(part for part in first["content"] if part["type"] == "tool_use")
call_args = tool_item["input"]
call_response = run_tool(call_args)
second = post({
    "model": HERMETIC_ANTHROPIC_MODEL,
    "max_tokens": 128,
    "messages": [
        {"role": "user", "content": PROMPT},
        {"role": "assistant", "content": [tool_item]},
        {"role": "user", "content": [{"type": "tool_result", "tool_use_id": tool_item["id"], "content": call_response}]},
    ],
    "tools": [{"name": "exec_command", "description": "run a command", "input_schema": {"type": "object", "properties": {"cmd": {"type": "string"}}}}],
})
reasoning = next(part["thinking"] for part in second["content"] if part["type"] == "thinking")
output = next(part["text"] for part in second["content"] if part["type"] == "text")
emit_result("anthropic", BASE_DOMAIN, "/v1/messages", HERMETIC_ANTHROPIC_MODEL, output, reasoning, tool_item["name"], call_args, call_response)
'''
    ).strip()


def claude_streaming_api_script(base_url: str) -> str:
    return textwrap.dedent(
        common_result_script_prelude(base_url, "claude-stream")
        + r'''
def parse_sse(body):
    events = []
    for line in body.splitlines():
        if line.startswith("data: "):
            events.append(json.loads(line[6:]))
    return events

def post(body):
    headers = {"content-type": "application/json", "anthropic-version": "2023-06-01"}
    add_anthropic_auth(headers)
    req = urllib.request.Request(
        BASE_URL + "/v1/messages",
        data=json.dumps(body).encode(),
        headers=headers,
        method="POST",
    )
    with urllib.request.urlopen(req, timeout=60) as response:
        return parse_sse(response.read().decode())

tools = [{"name": "exec_command", "description": "run a command", "input_schema": {"type": "object", "properties": {"cmd": {"type": "string"}}}}]
first_events = post({
    "model": HERMETIC_ANTHROPIC_MODEL,
    "max_tokens": 128,
    "stream": True,
    "messages": [{"role": "user", "content": PROMPT}],
    "tools": tools,
})
tool_start = next(event["content_block"] for event in first_events if event.get("type") == "content_block_start" and event["content_block"]["type"] == "tool_use")
arguments = "".join(
    event["delta"]["partial_json"]
    for event in first_events
    if event.get("type") == "content_block_delta" and event.get("delta", {}).get("type") == "input_json_delta"
)
call_args = json.loads(arguments)
call_response = run_tool(call_args)
second_events = post({
    "model": HERMETIC_ANTHROPIC_MODEL,
    "max_tokens": 128,
    "stream": True,
    "messages": [
        {"role": "user", "content": PROMPT},
        {"role": "assistant", "content": [tool_start]},
        {"role": "user", "content": [{"type": "tool_result", "tool_use_id": tool_start["id"], "content": call_response}]},
    ],
    "tools": tools,
})
reasoning = "".join(
    event["delta"]["thinking"]
    for event in second_events
    if event.get("type") == "content_block_delta" and event.get("delta", {}).get("type") == "thinking_delta"
)
output = "".join(
    event["delta"]["text"]
    for event in second_events
    if event.get("type") == "content_block_delta" and event.get("delta", {}).get("type") == "text_delta"
)
emit_result("anthropic", BASE_DOMAIN, "/v1/messages", HERMETIC_ANTHROPIC_MODEL, output, reasoning, tool_start["name"], call_args, call_response)
'''
    ).strip()


def claude_sdk_script(base_url: str) -> str:
    return textwrap.dedent(
        common_result_script_prelude(base_url, "claude-sdk")
        + r'''
import anthropic

client = anthropic.Anthropic(
    base_url=BASE_URL,
    api_key="sk-ant-" + NONCE,
)
tools = [{"name": "exec_command", "description": "run a command", "input_schema": {"type": "object", "properties": {"cmd": {"type": "string"}}}}]
first = client.messages.create(
    model=HERMETIC_ANTHROPIC_MODEL,
    max_tokens=128,
    messages=[{"role": "user", "content": PROMPT}],
    tools=tools,
)
tool_item = next(part for part in first.content if part.type == "tool_use")
call_args = dict(tool_item.input)
call_response = run_tool(call_args)
second = client.messages.create(
    model=HERMETIC_ANTHROPIC_MODEL,
    max_tokens=128,
    messages=[
        {"role": "user", "content": PROMPT},
        {"role": "assistant", "content": [tool_item.model_dump()]},
        {"role": "user", "content": [{"type": "tool_result", "tool_use_id": tool_item.id, "content": call_response}]},
    ],
    tools=tools,
)
reasoning = next(part.thinking for part in second.content if part.type == "thinking")
output = next(part.text for part in second.content if part.type == "text")
emit_result("anthropic", BASE_DOMAIN, "/v1/messages", HERMETIC_ANTHROPIC_MODEL, output, reasoning, tool_item.name, call_args, call_response)
'''
    ).strip()


def codex_cli_script(base_url: str) -> str:
    return textwrap.dedent(
        common_result_script_prelude(base_url, "codex-cli")
        + r'''
codex_config = Path("/root/.codex/config.toml")
codex_text = codex_config.read_text(encoding="utf-8")
codex_text = codex_text.replace('base_url = "http://127.0.0.1:11434/v1"', 'base_url = "' + BASE_URL + '/v1"')
if 'env_key = "OPENAI_API_KEY"' not in codex_text:
    codex_text = codex_text.replace('base_url = "' + BASE_URL + '/v1"', 'base_url = "' + BASE_URL + '/v1"\nenv_key = "OPENAI_API_KEY"')
if "check_for_update_on_startup" not in codex_text:
    codex_text += "\ncheck_for_update_on_startup = false\n[analytics]\nenabled = false\n"
codex_config.write_text(codex_text, encoding="utf-8")
env = os.environ.copy()
env["HOME"] = "/root"
env["NO_COLOR"] = "1"
env["TERM"] = "dumb"
env["OPENAI_API_KEY"] = "sk-" + NONCE
completed = subprocess.run(
    [
        "codex",
        "exec",
        "--dangerously-bypass-approvals-and-sandbox",
        "--skip-git-repo-check",
        "--cd",
        "/root",
        PROMPT,
    ],
    cwd="/root",
    env=env,
    capture_output=True,
    text=True,
    timeout=180,
)
if completed.returncode != 0:
    raise SystemExit((completed.stdout or "") + (completed.stderr or ""))
call_args = {"cmd": "printf '%s\\n' " + NONCE + " > " + TARGET, "yield_time_ms": 1000, "max_output_tokens": 2000}
emit_result("ollama", BASE_DOMAIN, "/v1/responses", HERMETIC_OPENAI_COMPAT_MODEL, NONCE, "ledger reasoning", "exec_command", call_args, "Process exited with code 0", credential_provider="openai")
'''
    ).strip()


def claude_ollama_launch_script(base_url: str) -> str:
    return textwrap.dedent(
        common_result_script_prelude(base_url, "claude-ollama-launch")
        + r'''
env = os.environ.copy()
env["HOME"] = "/root"
env["NO_COLOR"] = "1"
env["TERM"] = "xterm-256color"
env["OLLAMA_HOST"] = BASE_URL
completed = subprocess.run(
    [
        "ollama",
        "launch",
        "claude",
        "-y",
        "--model",
        HERMETIC_OPENAI_COMPAT_MODEL,
        "--",
        "-p",
        PROMPT,
    ],
    cwd="/root",
    env=env,
    capture_output=True,
    text=True,
    timeout=240,
)
if completed.returncode != 0:
    raise SystemExit((completed.stdout or "") + (completed.stderr or ""))
call_args = {"command": "printf '%s\\n' " + NONCE + " > " + TARGET, "description": "write ironbank token"}
emit_result("ollama", "127.0.0.1", "/v1/messages", HERMETIC_OPENAI_COMPAT_MODEL, NONCE, "ledger reasoning", "Bash", call_args, "(Bash completed with no output)")
'''
    ).strip()


def codex_ollama_launch_script(base_url: str) -> str:
    return textwrap.dedent(
        common_result_script_prelude(base_url, "codex-ollama-launch")
        + r'''
env = os.environ.copy()
env["HOME"] = "/root"
env["NO_COLOR"] = "1"
env["TERM"] = "xterm-256color"
env["OLLAMA_HOST"] = BASE_URL
completed = subprocess.run(
    [
        "ollama",
        "launch",
        "codex",
        "-y",
        "--model",
        HERMETIC_OPENAI_COMPAT_MODEL,
        "--",
        "exec",
        "--dangerously-bypass-approvals-and-sandbox",
        "--skip-git-repo-check",
        "--cd",
        "/root",
        PROMPT,
    ],
    cwd="/root",
    env=env,
    capture_output=True,
    text=True,
    timeout=240,
)
if completed.returncode != 0:
    raise SystemExit((completed.stdout or "") + (completed.stderr or ""))
call_args = {"cmd": "printf '%s\\n' " + NONCE + " > " + TARGET, "yield_time_ms": 1000, "max_output_tokens": 2000}
emit_result("ollama", "127.0.0.1", "/v1/responses", HERMETIC_OPENAI_COMPAT_MODEL, NONCE, "ledger reasoning", "exec_command", call_args, "Process exited with code 0")
'''
    ).strip()


def agy_cli_script(_base_url: str) -> str:
    return textwrap.dedent(
        common_result_script_prelude("http://127.0.0.1:3713", "agy-cli")
        + r'''
env = os.environ.copy()
env["HOME"] = "/root"
env["NO_COLOR"] = "1"
env["TERM"] = "xterm-256color"
token_path = Path("/root/.gemini/antigravity-cli/antigravity-oauth-token")
token_path.parent.mkdir(parents=True, exist_ok=True)
token_path.write_text(json.dumps({
    "token": {
        "access_token": "capsem_test_agy_access_" + NONCE,
        "token_type": "Bearer",
        "refresh_token": "capsem_test_agy_refresh_" + NONCE,
        "expiry": "2099-01-01T00:00:00Z"
    },
    "auth_method": "consumer"
}), encoding="utf-8")
token_path.chmod(0o600)
settings_path = Path("/root/.gemini/antigravity-cli/settings.json")
agy_model_settings = {
    "trustedWorkspaces": ["/root"],
    "telemetry": {"enabled": False},
    "autoUpdate": {"enabled": False}
}
settings_path.write_text(json.dumps(agy_model_settings), encoding="utf-8")
completed = subprocess.run(
    [
        "agy",
        "--dangerously-skip-permissions",
        "--model",
        HERMETIC_AGY_MODEL_DISPLAY,
        "-p",
        PROMPT,
        "--print-timeout",
        "90s",
    ],
    cwd="/root",
    env=env,
    capture_output=True,
    text=True,
    timeout=120,
)
if completed.returncode != 0:
    raise SystemExit((completed.stdout or "")[-24000:] + (completed.stderr or "")[-12000:])
if not Path(TARGET).exists():
    raise SystemExit(
        "agy did not create target file\n"
        + "--- stdout ---\n"
        + (completed.stdout or "")[-12000:]
        + "\n--- stderr ---\n"
        + (completed.stderr or "")[-12000:]
    )
call_args = {
    "CommandLine": "printf '%s\\n' " + NONCE + " > " + TARGET,
    "Cwd": "/root",
    "WaitMsBeforeAsync": 1000,
    "toolSummary": "Write proof",
    "toolAction": "Writing file",
}
emit_result("google", "daily-cloudcode-pa.googleapis.com", "/v1internal:streamGenerateContent", HERMETIC_AGY_MODEL, NONCE, "", "run_command", call_args, "The command completed successfully", credential_provider="google", credential_source="http.header.authorization")
'''
    ).strip()
