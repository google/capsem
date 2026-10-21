"""Read a sample as data and write a static triage report; never execute it."""

import hashlib
import json
import re
import sys
from pathlib import Path

sample, report = map(Path, sys.argv[1:])
data = sample.read_bytes()
strings = [match.decode("ascii") for match in re.findall(rb"[\x20-\x7e]{4,}", data)]
if data.startswith(b"MZ"):
    format_hint = "PE/DOS signature"
elif data.startswith(b"\x7fELF"):
    format_hint = "ELF signature"
else:
    format_hint = "unidentified; inspect with a format-specific tool"
result = {
    "file": sample.name,
    "size_bytes": len(data),
    "sha256": hashlib.sha256(data).hexdigest(),
    "format_hint": format_hint,
    "strings": strings,
    "urls": sorted(set(re.findall(r"https?://[^\s\"'<>]+", "\n".join(strings)))),
    "executed_sample": False,
    "network_requests": False,
    "conclusion": "Static observations only; strings and signatures do not establish intent or safety.",
}
report.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
print(f"Wrote {report}")
