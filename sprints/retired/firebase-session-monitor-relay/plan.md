# Sprint: Firebase Session Monitor Relay

## What we are building

Remote Capsem access through Firebase Realtime Database as a dumb conduit. A
remote browser writes allowlisted request and terminal relay messages into
Firebase. A local Rust bridge reads those messages, talks to the local
`capsem-gateway`, and writes responses or terminal output back to Firebase.

The first product path is not "create a VM from a phone". It is:

1. Open remote UI on a phone or laptop.
2. Sign in with the same Google/Firebase account.
3. Pick the paired host.
4. See existing Capsem sessions.
5. Attach to a running session.
6. Monitor the session and talk to the already-running agent.

## Core product decision: how remote users talk to a session

Use a new remote session view, not the existing local `TerminalFrame.svelte` as
the primary mobile surface.

- The remote view is chat/transcript-first on mobile.
- The existing xterm-style terminal remains the local desktop default.
- The remote view is still terminal-backed: it talks to the same PTY stream as
  xterm by attaching through `capsem-gateway`'s `/terminal/{id}` WebSocket.
- The composer sends bytes to the existing PTY. It does not call `/exec/{id}`,
  because `/exec` starts a separate process and cannot talk to the agent that is
  already running in the session.
- Remote raw terminal mode is available as a fallback. It uses xterm.js over the
  Firebase terminal byte stream, but it is not the mobile default.

This means "chat" is a mobile-friendly view over the terminal stream, not a new
agent protocol and not a second execution path.

## Why

The current UI is local-gateway-first. `frontend/src/lib/api.ts` fetches a
localhost gateway token and `TerminalFrame.svelte` opens
`ws://127.0.0.1:19222/terminal/{id}` directly. That is correct for the desktop
app, but remote browsers cannot and must not receive the local gateway token or
reach the user's loopback interface.

Firebase gives us a NAT-free, realtime relay without making the gateway public.
The local Rust bridge is the only process that touches `capsem-gateway`.

## Architecture

```
Remote browser / phone
  |
  | Firebase Auth + Realtime Database
  v
users/{uid}/devices/{deviceId}/...
  ^
  | Firebase Auth + RTDB stream
  |
capsem-remote (Rust companion, local host)
  |
  | local HTTP/WS + gateway bearer token
  v
capsem-gateway (127.0.0.1:19222)
  |
  v
capsem-service -> capsem-process -> VM PTY
```

`capsem-remote` is a companion process like the gateway and tray: it is spawned
by `capsem-service`, parent-watched, singleton-locked, and tied to the service
lifecycle. It should not be a guest component.

## Key decisions and trade-offs

### Firebase is a conduit only

Firebase stores relay mailboxes and presence, not Capsem domain state. The
remote UI asks the local gateway for truth through the relay.

Allowed Firebase paths:

```
users/{uid}/devices/{deviceId}/presence
users/{uid}/devices/{deviceId}/requests/{requestId}
users/{uid}/devices/{deviceId}/responses/{requestId}
users/{uid}/devices/{deviceId}/terminal/{vmId}/control/{clientId}
users/{uid}/devices/{deviceId}/terminal/{vmId}/input/{messageId}
users/{uid}/devices/{deviceId}/terminal/{vmId}/output/{chunkId}
```

No VM summaries, transcripts, settings, gateway tokens, local URLs, UDS paths,
or host filesystem paths are stored as first-class Firebase objects.

### HTTP relay is allowlist-only

Remote request envelopes are not arbitrary HTTP proxy requests. They must match
an allowlist for method, path, and headers. Any non-allowlisted method, path, or
header is rejected with a relay error. The bridge does not silently strip and
forward a modified request.

Allowed methods:

- `GET`
- `POST`
- `DELETE`

Allowed remote headers:

- `accept`
- `content-type`
- `x-capsem-request-id`

The local bridge constructs the actual gateway request and injects the gateway
bearer token locally. `authorization` is not an allowed remote header.

Initial path allowlist, focused on existing-session monitoring:

- `/status`
- `/list`
- `/info/{id}`
- `/logs/{id}`
- `/inspect/{id}`
- `/stats`
- `/files/{id}`
- `/files/{id}/content`
- `/stop/{id}`
- `/resume/{name}`
- `/delete/{id}`
- `/persist/{id}`
- `/fork/{id}`
- `/settings`
- `/settings/*`
- `/setup/state`
- `/update/check`

Creating new sessions through `/provision` can be added after the monitor/talk
path is solid.

### Terminal relay is narrow and explicit

Terminal relay is not a generic WebSocket proxy. It only supports attach,
detach, resize, input chunks, and output chunks.

Attach flow:

1. Remote UI writes `terminal/{vmId}/control/{clientId}` with
   `{ "attached": true, "mode": "chat", "cols": 100, "rows": 30 }`.
2. `capsem-remote` opens the local gateway WebSocket for that VM if no bridge
   connection is active.
3. `capsem-remote` sends a terminal resize to stabilize output width.
4. Local PTY bytes are written to `output/{chunkId}`.
5. Composer messages written to `input/{messageId}` are forwarded as UTF-8
   bytes plus `\r` to the PTY.
6. When the last remote client detaches or goes stale, the bridge closes the
   local WebSocket.

Raw mode uses viewport-derived `cols`/`rows` and renders xterm. Chat mode uses a
stable default width so mobile transcript output is predictable.

### Remote UI uses two renderers

1. **Chat/transcript renderer**: the default mobile session view. It shows a
   composer, user-sent messages, terminal output blocks, status, and quick
   session actions.
2. **Raw terminal renderer**: an explicit toggle/drawer for cases where full
   terminal behavior matters. It uses xterm.js over the Firebase terminal
   chunks.

The remote chat renderer may use xterm.js internally to interpret ANSI and PTY
state, but the user-facing mobile default is not a full xterm grid.

## Files and modules to create or modify

### Rust

- Add `crates/capsem-remote/` for the Firebase bridge companion.
- Update workspace `Cargo.toml` to include `capsem-remote`.
- Update `crates/capsem-service/src/main.rs` companion spawning to start
  `capsem-remote` when remote access is enabled and credentials exist.
- Add remote configuration settings under `config/defaults.toml`.
- Add service/gateway-proxied endpoints for remote auth/status:
  - `GET /remote/status`
  - `POST /remote/login/start`
  - `POST /remote/logout`

`capsem-app` should stay thin. The desktop UI can start login through the
gateway/service endpoint and use existing `open_url` to open the system browser.

### Frontend

- Add remote-mode API transport in `frontend/src/lib/api.ts` or a small
  companion module called by `api.ts`.
- Add remote stores under `frontend/src/lib/stores/remote*.svelte.ts`.
- Add remote UI components under `frontend/src/lib/components/remote/`:
  - device picker
  - existing session list
  - session chat/transcript view
  - raw terminal fallback drawer
- Add a remote page entry point suitable for Firebase Hosting.

### Firebase project artifacts

- Add Realtime Database security rules for the relay paths.
- Add emulator configuration or test fixtures for rules tests.
- Document required Firebase web config and Google OAuth client settings in the
  sprint tracker as they are discovered.

## Relay wire shapes

### HTTP request

```json
{
  "method": "GET",
  "path": "/status",
  "headers": {
    "accept": "application/json",
    "x-capsem-request-id": "client-generated-id"
  },
  "body_b64": null,
  "created_at_ms": 1778240000000,
  "expires_at_ms": 1778240030000
}
```

### HTTP response

```json
{
  "status": 200,
  "headers": {
    "content-type": "application/json"
  },
  "body_b64": "eyJ2bXMiOltdfQ==",
  "error": null,
  "completed_at_ms": 1778240001200
}
```

### Terminal control

```json
{
  "attached": true,
  "mode": "chat",
  "cols": 100,
  "rows": 30,
  "updated_at_ms": 1778240000000
}
```

### Terminal input

```json
{
  "text": "continue",
  "append_enter": true,
  "created_at_ms": 1778240000000
}
```

### Terminal output

```json
{
  "seq": 42,
  "data_b64": "Li4u",
  "byte_len": 3,
  "created_at_ms": 1778240000100
}
```

## Dependencies and ordering

1. **Sprint docs and threat model**: finalize relay paths, allowlists, and leak
   rules before code.
2. **Rust bridge skeleton**: new crate, config parsing, parent-watch,
   singleton, credential loading stub, mock Firebase client.
3. **HTTP relay**: request validation, gateway forwarding, response writing,
   timeout behavior.
4. **Terminal relay**: attach/control handling, local gateway WS connection,
   PTY input/output chunking, cleanup.
5. **Firebase auth and rules**: login flow, secure credential storage,
   Realtime Database rules, emulator tests.
6. **Remote frontend transport**: Firebase-backed API transport and device
   selection.
7. **Remote session view**: existing session list, chat/transcript surface, raw
   terminal fallback.
8. **Verification and hardening**: leak tests, mobile screenshots, full test
   gate, changelog, commits.

## Done means

- A user can sign in locally, pair the host, and sign in remotely from a phone
  or remote laptop.
- The remote UI shows existing sessions without exposing gateway credentials.
- The user can attach to a running session, see PTY output, and send text to
  the agent already running in that session.
- The default phone view is chat/transcript-first.
- Raw terminal fallback works for the same attached session.
- Non-allowlisted relay requests are rejected.
- Firebase contains no gateway token, local gateway URL, UDS path, or host
  filesystem internals.
- Tests cover unit/contract, functional, adversarial, E2E/VM, telemetry, and
  performance categories or record named follow-up debt.

## Testing proof matrix

| Slice | Unit/contract | Functional | Adversarial | E2E/VM | Telemetry | Performance |
| --- | --- | --- | --- | --- | --- | --- |
| HTTP relay validation | method/path/header allowlist tests | mock gateway forwarding | bad method/path/header/body/timeout | remote `/status` through bridge | relay logs redact secrets | request latency under budget |
| Terminal relay | input/output chunk tests | mock WS bridge | stale attach, oversized chunk, bad vm id | attach to real VM and send text to shell/agent | terminal relay events redact bytes by default | output burst does not freeze bridge |
| Firebase rules | emulator rule tests | signed-in own-device access | cross-user and malformed path denial | phone/browser emulator smoke | security-rule denial logged locally | rule evaluation acceptable |
| Remote UI | store/transport tests | session list and chat view | timeout/offline/rejected requests | mobile attach to real session | UI logs no secrets | mobile render remains responsive |

## Commit strategy

Use functional milestone commits, each with a `CHANGELOG.md` entry:

1. `feat(remote): add Firebase relay bridge skeleton`
2. `feat(remote): relay allowlisted gateway HTTP requests`
3. `feat(remote): relay terminal sessions through Firebase`
4. `feat(remote-ui): add mobile session monitor view`
5. `test(remote): add emulator and end-to-end relay coverage`

## References

- Firebase Google Sign-In:
  https://firebase.google.com/docs/auth/web/google-signin
- Firebase ID token verification:
  https://firebase.google.com/docs/auth/admin/verify-id-tokens
- Realtime Database REST authentication:
  https://firebase.google.com/docs/database/rest/auth
