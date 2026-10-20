---
title: Analyze a suspicious file with an agent
description: How do I have my agent inspect a suspicious file in Capsem and return a report?
---

Give an agent a suspicious file, have it inspect the bytes inside a temporary
Capsem VM, and retrieve a report on your computer. This walkthrough uses a
harmless text fixture containing suspicious-looking strings. It performs
static triage: it does not execute the sample or contact its URLs.

## Connect your agent to Capsem

[Install Capsem](/getting-started/installation/) and start it on your computer.
You also need Node.js 20 or later to run the host MCP server. In your agent's
MCP settings, register a **stdio** server with these fields:

| Field | Value |
| --- | --- |
| Executable | `npx` |
| Arguments | `--yes`, `@capsem/mcp@0.7.0`, `--gateway-url`, `http://127.0.0.1:19222`, `--token-file`, `/absolute/path/to/.capsem/run/gateway.token` |

Replace the token-file path with the absolute path under your home directory.
If Capsem uses another port, read it from `~/.capsem/run/gateway.port` and
replace `19222`. The server reads the token file on the host; do not paste
its contents into the agent's prompt or upload it to the VM. The package was
tested against the native Capsem 0.7.1 service.

Reconnect the agent and check that it lists `capsem_create`, `capsem_exec`,
`capsem_write_file`, `capsem_read_file`, and `capsem_delete`. If they are missing,
check MCP startup diagnostics before trying the task. Exact schemas are in the
[MCP reference](/reference/mcp/).

## Install the agent skill

In your agent's project directory, run:

```sh
npx skills add google/capsem --skill use-capsem
```

Select your agent when prompted. The `use-capsem` skill teaches VM creation,
file transfer, bounded execution, result checks, and cleanup. It includes the
training sample and the Python inspection helper used below. The
[skills CLI](https://github.com/vercel-labs/skills/blob/main/docs/commands.md)
also supports selecting an agent with `--agent`.

## Ask for a static report

Save the [harmless sample](/examples/triage-sample.txt) and
[inspection helper](/examples/triage.py) into your project directory. Give your
agent this task:

```text
Use the use-capsem skill and Capsem MCP to inspect triage-sample.txt.
Create a new temporary VM without an image. Upload the sample and triage.py
using capsem_write_file. Run only:
python3 /root/triage.py /root/triage-sample.txt /root/report.json

Do not execute the sample or follow its URLs. Do not attach a network,
publish a port, or provide credentials. Check the command's exit code.
Retrieve report.json through capsem_read_file and save it as report.json
in this project. Retrieve all bytes if the result is truncated.
Delete only the temporary VM you created after retrieving the report.
Summarize the observed hash, file size, strings, and URL indicators.
Distinguish observations from conclusions about whether the file is malware.
```

The helper reads the sample as data using Python's standard library. It writes
the SHA-256 hash, size, a file-signature hint, printable strings, and extracted
URLs. For a real PE/ELF sample, use a format-specific static parser to inspect
imports and sections; this helper does not claim that depth of analysis.

## Check the returned report

Open the local `report.json`. It should contain the sample's SHA-256 hash,
`executed_sample: false`, `network_requests: false`, and this URL indicator:

```text
https://updates.example.invalid/payload
```

The fixture also contains a PowerShell command string. Finding those strings
is an observation; it does not prove execution, malicious intent, or safety.
The flags describe this helper's operations, not a universal record of every
process in the VM.

Ask the agent to confirm successful `capsem_delete` for its returned VM ID.
Keep the retrieved report before deleting the VM. To inspect recorded activity
before cleanup, use [Investigate a VM](/guides/enterprise/investigate-a-vm/).
