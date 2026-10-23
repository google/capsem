---
title: Proxy reference
description: What does proxy mode expose, and how do I start, renew, and stop it?
---

Use proxy mode to send model API requests through Capsem's security engine
without creating a VM. This reference covers its command, HTTP control API,
authentication, routing, and lifetime. Follow [Use model APIs through the proxy](/guides/users/use-the-proxy/)
for a working client configuration and a tested response-blocking rule.

## Command

```sh
capsem proxy [--provider PROVIDER] [--bind IP] [--port PORT]
```

| Option | Default | Contract |
| --- | --- | --- |
| `--provider` | `openai` | Provider key in effective built-in, settings, and corp policy. Selects the upstream URL, base path, and protocol. |
| `--bind` | `127.0.0.1` | IP address for the unauthenticated HTTP data listener; hostnames are not accepted. |
| `--port` | `0` | TCP port; `0` selects an available port. |

The command prints `Proxy session:`, `Base URL:`, and `Press Ctrl-C to stop.`
Use the printed base URL, which includes the provider's configured path prefix.
The CLI keeps running and renews the lease every five seconds. Ctrl-C or
SIGTERM requests cleanup; a heartbeat failure also ends the command.

## Control API

These routes use the authenticated Capsem gateway. The model data listener is
a separate endpoint; a model client's API key is not a gateway or lease token.

| Method and route | JSON request | Successful response |
| --- | --- | --- |
| `POST /proxies` | `provider` (string, 1–128 bytes), `bind` (IP string), `port` (unsigned 16-bit integer). All fields required. | `CreateProxyResponse`; HTTP 200. |
| `POST /proxies/{id}/heartbeat` | `lease_token` (string). | `session_id`, `lease_expires_unix_ms`; HTTP 200. |
| `POST /proxies/{id}/stop` | `lease_token` (string). | `session_id`, `stopped: true`; HTTP 200. |

`CreateProxyResponse` contains `session_id`, `provider`, `bind`, selected `port`,
`base_url`, `lease_token`, and `lease_expires_unix_ms`. Store the returned lease
token privately and use it only with that proxy's heartbeat and stop routes.
Unknown proxy IDs return 404; a wrong lease token returns 403. Invalid bind or
provider-length values return 400; startup and cleanup failures return 500.

Each lease lasts 15 seconds from creation or its last successful heartbeat.
Renew before that deadline; use the returned Unix-millisecond expiry when
implementing a controller. Stopping requires both gateway authentication and
the matching lease token. The CLI handles these steps for interactive use.

The generated [TypeScript operations](/sdk/typescript/#operationscreateproxy)
and [Rust operations](/sdk/rust/) describe the low-level client types.
The SDKs do not currently provide a high-level proxy lease manager, and
`@capsem/mcp` does not expose proxy lifecycle tools.

## Data endpoint and routing

| Built-in provider key | Upstream base URL | Native format |
| --- | --- | --- |
| `openai` | `https://api.openai.com/v1` | OpenAI. |
| `anthropic` | `https://api.anthropic.com/v1` | Anthropic Messages (Claude). |
| `google` | `https://generativelanguage.googleapis.com/v1beta` | Gemini API. |
| `ollama` | `http://127.0.0.1:11434` | Ollama, including its OpenAI-compatible routes. |

Use the matching client and API path for the selected provider. The proxy
preserves the request path; it does not add a missing prefix or remove a
duplicated one. OpenAI clients need `/v1` in their base URL. Clients that append
their own `/v1` or `/v1beta` need the listener's origin instead. Settings and
corp configuration can change the built-in upstream URLs.

| Property | Contract |
| --- | --- |
| Client transport | Plain HTTP on the selected bind address and port. No listener authentication. |
| Upstream | Fixed by `--provider` and effective policy. Client `Host` headers do not select another upstream. |
| Request format | The selected provider's native model API format. No translation between OpenAI, Anthropic, and other protocols. |
| OpenAI clients | Chat Completions and Responses requests, including streaming, use the configured OpenAI base path. An OpenAI-compatible upstream must support the selected API. |
| General forward proxy | HTTP CONNECT is refused. Do not configure this as `HTTP_PROXY` or `HTTPS_PROXY`. |
| Policy | The existing security-engine rules, plugins, credential processing, and model logging apply to requests sent here. |
| Host isolation | No VM is created. Host files, processes, and traffic sent elsewhere are outside this endpoint's enforcement. |

Keep the default loopback bind for local clients. Binding another interface
exposes an unauthenticated, unencrypted model endpoint to every client that can
reach it; neither the gateway token nor a model-provider API key restricts
access to this listener.

## Cleanup and records

Stop or lease expiry revokes capabilities, closes the listener, shuts down the
proxy and ledger workers, and removes the proxy's temporary directory and
local records. A worker failure also triggers cleanup. Proxy leases do not
appear in the VM inventory and cannot be retained by naming a VM.

Do not rely on this mode for retained local history: the command has no record
export or keep-history option. See [Proxy architecture](/architecture/overview/#run-model-requests-without-a-vm)
for process ownership and [Security engine](/architecture/security-engine/)
for inspection and logging behavior.
