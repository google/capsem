---
title: MCP Tools
description: Reference for the capsem MCP tools exposed to AI agents (Claude Code, Gemini CLI, Cursor, etc.).
sidebar:
  order: 2
---

When the `capsem-mcp` stdio server is registered with your AI CLI, the agent gains 22 tools for creating and driving VM sessions, running commands, reading and writing files inside the guest, querying telemetry, and calling into the in-VM MCP gateway.

All tools use **camelCase** parameter names on the wire (e.g. `ramMb`, `cpuCount`). The source of truth is `crates/capsem-mcp/src/main.rs`.

## Configuration

Register the server in your AI CLI settings. For Claude Code:

```json
{
  "mcpServers": {
    "capsem": { "command": "capsem-mcp" }
  }
}
```

The binary is installed to `~/.capsem/bin/capsem-mcp` by `capsem setup`.

## Session lifecycle

| Tool | Parameters | Description |
|------|-----------|-------------|
| `capsem_create` | `name?`, `ramMb?`, `cpuCount?`, `env?`, `image?` | Create and boot a new session. Named sessions are persistent. RAM/CPU fall back to the user's configured defaults. Returns session ID. |
| `capsem_run` | `command`, `timeout?` | Run a command in a fresh temporary session. Auto-provisions and destroys the VM. Returns stdout, stderr, exit_code. |
| `capsem_list` | -- | List all sessions (running and stopped persistent) with ID, name, status, RAM, CPUs, uptime, and telemetry. |
| `capsem_info` | `id` | Session details: ID, name, status, persistent, RAM, CPUs, version, telemetry. |
| `capsem_resume` | `name` | Resume a stopped persistent session (or get ID of a running one). Returns session ID. |
| `capsem_suspend` | `id` | Suspend a session to disk (saves RAM + CPU state). Persistent sessions only. |
| `capsem_stop` | `id` | Stop a session. Persistent sessions preserve state; ephemeral sessions are destroyed. |
| `capsem_delete` | `id` | Delete a session permanently. Destroys all state including persistent data. |
| `capsem_persist` | `id`, `name` | Convert a running ephemeral session to a persistent named session. |
| `capsem_fork` | `id`, `name`, `description?` | Fork a running or stopped session into a new stopped persistent session. Works as a reusable template. |
| `capsem_purge` | `all?` | Kill all temporary sessions. Set `all=true` to also destroy persistent sessions. |

## Exec and file access

| Tool | Parameters | Description |
|------|-----------|-------------|
| `capsem_exec` | `id`, `command`, `timeout?` | Run a shell command inside a running session. Returns stdout, stderr, exit_code. Default 30s timeout. |
| `capsem_read_file` | `id`, `path` | Read a file from the guest filesystem. Returns text content. |
| `capsem_write_file` | `id`, `path`, `content` | Write a file into the guest filesystem. |

## Telemetry and logs

| Tool | Parameters | Description |
|------|-----------|-------------|
| `capsem_inspect_schema` | -- | Get CREATE TABLE statements for all session telemetry tables. Call before `capsem_inspect` to know what columns are available. |
| `capsem_inspect` | `id`, `sql` | Run a read-only SQL query against a session's telemetry database. Returns columns and rows. |
| `capsem_vm_logs` | `id`, `grep?`, `tail?` | Serial + process logs for a session. `grep` filters lines, `tail` limits to last N lines. |
| `capsem_service_logs` | `grep?`, `tail?` | Latest `capsem-service` logs (last ~100 KB). `grep` + `tail` filters. |

## MCP aggregator (in-guest gateway)

These tools let the agent exercise the full in-VM MCP gateway (policy + telemetry) without having to drive `capsem_exec` by hand.

| Tool | Parameters | Description |
|------|-----------|-------------|
| `capsem_mcp_servers` | -- | List configured MCP servers with connection status and tool counts. |
| `capsem_mcp_tools` | `server?` | List discovered MCP tools across all connected servers. Filter by `server` name to scope to one. |
| `capsem_mcp_call` | `name`, `arguments?` | Call an MCP tool by namespaced name (e.g. `github__search_repos`) with JSON arguments. |

## Diagnostics

| Tool | Parameters | Description |
|------|-----------|-------------|
| `capsem_version` | -- | MCP server version and service connectivity status. |

## Example workflows

**One-shot command in a disposable VM:**

```json
{ "tool": "capsem_run", "arguments": { "command": "curl -s https://api.github.com/zen" } }
```

**Iterative debugging in a long-lived VM:**

```json
{ "tool": "capsem_create", "arguments": { "name": "dev" } }
{ "tool": "capsem_exec",   "arguments": { "id": "<id>", "command": "capsem-doctor -k net" } }
{ "tool": "capsem_inspect", "arguments": { "id": "<id>", "sql": "SELECT domain, action, status_code FROM net_events ORDER BY timestamp DESC LIMIT 10" } }
```

**Fork a template and boot from it:**

```json
{ "tool": "capsem_fork",   "arguments": { "id": "<id>", "name": "python-ready" } }
{ "tool": "capsem_create", "arguments": { "image": "python-ready" } }
```

For CLI equivalents of these commands see the [CLI reference](/usage/cli/).
