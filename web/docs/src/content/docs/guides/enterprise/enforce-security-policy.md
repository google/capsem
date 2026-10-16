---
title: Enforce security policy
description: How do I write rules that allow or block access in Capsem?
---

This guide shows how to control which destinations and operations a VM can use:
allow an API, restrict its HTTP methods, block an MCP tool, and apply a corporate
constraint. You need an installed Capsem and permission to change user rules;
user rules cannot override corp constraints.

## Create and load a rule file

Create `~/.capsem/access-rules.toml` with a rule for your API hostname:

```toml
[profiles.rules.allow_api]
name = "allow_api"
action = "allow"
priority = 20
match = 'http.host == "api.example.com" || dns.qname == "api.example.com"'
```

Replace `api.example.com` with your destination. The expression matches either
HTTP traffic to that hostname or its DNS lookup. It does not allow unrelated
destinations.

In `~/.capsem/settings.toml`, add the file to the existing `[rule_files]` section,
or create that section if it is absent:

```toml
[rule_files]
enforcement = "access-rules.toml"
```

If an enforcement file is already configured, add your rules to that file.
Relative paths are resolved from `settings.toml`'s directory.

## Restrict the API to GET and HEAD

Add this rule to the same file:

```toml
[profiles.rules.block_api_writes]
name = "block_api_writes"
action = "block"
priority = 10
match = 'http.host == "api.example.com" && http.method != "GET" && http.method != "HEAD"'
reason = "Only GET and HEAD are permitted for this API."
```

`&&` requires every condition to match. This blocks requests to the API when
their method is neither GET nor HEAD. Its lower priority number places it
before the allow rule; GET and HEAD requests still match the allow rule.

## Block an MCP tool

Add a rule using the configured MCP server and tool names:

```toml
[profiles.rules.block_delete_tool]
name = "block_delete_tool"
action = "block"
priority = 10
match = 'mcp.server.name == "my-server" && mcp.tool_call.name == "delete_file"'
reason = "This VM must not delete files through this MCP tool."
```

This targets one tool on one server. It does not block other tools named
`delete_file` on different servers.

## Check the result

Create a new VM after saving the files, then access it through the
[TUI or Capsem UI](/getting-started/how-to-use-capsem/#access-your-vm).
Try a GET and a POST against your chosen API. GET should be allowed; POST should
be refused with the rule's reason. Check the recorded security decision to
confirm which rule matched. Corp constraints can still refuse the GET.

## Enforce a corporate constraint

After checking the rule on a pilot VM, put administrator constraints in your
corporate enforcement file rather than relying on user-editable rules:

```toml
[corp.rules.block_api_writes]
name = "block_api_writes"
action = "block"
priority = -100
match = 'http.host == "api.example.com" && http.method != "GET" && http.method != "HEAD"'
reason = "Corporate policy permits only GET and HEAD."
```

Select it through `[corp_rule_files].enforcement` in your corporate file,
then validate with `capsem-admin enforcement validate PATH --source corp --json`.
Repeat the allowed and refused request checks on a pilot before distributing
both files through your administrator-owned configuration.
Corporate rules precede user rules. See
[Deploy Capsem in your enterprise](/guides/enterprise/deploy-capsem/).

For all operators, string functions, event fields, and quoting rules, use the
[CEL syntax reference](/reference/cel/). To record detections, see
[Write detection rules](/guides/enterprise/write-detection-rules/). The
[security engine architecture](/architecture/security-engine/) explains evaluation.
