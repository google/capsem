---
title: Writing detection rules
description: How do I write Sigma rules to detect activity in Capsem?
---

Use Sigma rules to record activity that matches particular security-event
fields. This guide detects requests to a destination and calls to selected
MCP tools. You need an installed Capsem and permission to change user rules.

## Create a detection

Create `~/.capsem/detection.yaml`:

```yaml
title: Example API request
description: Record requests to the example API.
level: informational
logsource:
  product: capsem
  service: security_event
detection:
  destination:
    http.host: api.example.com
  condition: destination
capsem:
  action: postprocess
```

Replace the hostname with your destination. `destination` is a named selection;
`condition` chooses which selection must match. `level` sets the detection's
severity. `action: postprocess` records the match without selecting an allow,
ask, or block decision; your access rules still decide whether the request proceeds.

In `~/.capsem/settings.toml`, add the detection file to the existing
`[rule_files]` section, or create that section if it is absent:

```toml
[rule_files]
sigma = "detection.yaml"
```

Keep any existing `enforcement` entry. If a Sigma file is already configured,
add the detection there. Separate multiple YAML rules with `---`.

## Detect selected MCP tools

Append another rule to the file:

```yaml
---
title: Sensitive MCP tool call
level: high
logsource:
  product: capsem
  service: security_event
detection:
  sensitive_tool:
    mcp.server.name: my-server
    mcp.tool_call.name:
      - delete_file
      - run_command
  condition: sensitive_tool
capsem:
  action: postprocess
```

Both fields must match: the server must be `my-server`, and the tool must be
either `delete_file` or `run_command`. A list means any listed value matches.

## Check the result

Create a new VM after saving the files. Make a request to the configured API,
then check its security records for the rule name and `informational` detection.
Call one of the configured MCP tools and check for the `high` detection.
An unrelated destination or tool should not match these rules.

To refuse a matching operation instead, set `capsem.action: block` on the
appropriate rule. This can stop an operation at an enforcing boundary;
detecting a guest process audit record cannot undo a command that already ran.

Use the [Sigma syntax reference](/reference/sigma/) for the complete supported
format, conditions, fields, and actions. See [Writing access rules](/guides/writing-access-rules/)
for CEL examples and the [security engine architecture](/architecture/security-engine/)
for how detections are evaluated and logged.
