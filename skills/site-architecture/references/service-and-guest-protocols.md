# Service and Guest Protocols

Read this reference before changing the service/process topology, CLI or MCP
execution paths, gateway or service routes, host IPC, vsock framing or ports,
or any guest agent binary.

## Service architecture

**All VM operations go through a single path.** There is no direct VM boot -- every entry point routes through capsem-service to capsem-process.

```
AI Agent  -> capsem-mcp (stdio)  -> HTTP/UDS -> capsem-service
User      -> capsem CLI          -> HTTP/UDS -> capsem-service
Frontend  -> capsem-gateway (TCP)-> HTTP/UDS -> capsem-service
Tray app  -> capsem-gateway (TCP)-> HTTP/UDS -> capsem-service
                                                     |
                                        capsem-process (per-VM, UDS IPC)
                                                     |
                                         +-----------+-----------+
                                         |           |           |
                                    vsock:5000  vsock:5001  vsock:5005
                                    (control)  (terminal)  (exec output)
                                         |           |           |
                                         +-----guest agent------+
```

**Entry points for exec:**
- `capsem exec <id> "cmd"` -> service HTTP `/exec/{id}` -> process IPC -> vsock
- `capsem run "cmd"` -> service HTTP `/run` -> provision + exec + destroy
- MCP `capsem_exec` / `capsem_run` -> service HTTP -> same path

**Entry point for interactive shell:**
- `capsem shell [id]` -> UDS IPC directly to capsem-process -> `StartTerminalStream` -> vsock:5001

### IPC protocols

| Layer | Protocol | Socket |
|-------|----------|--------|
| Frontend/Tray -> gateway | HTTP/1.1 over TCP | `127.0.0.1:19222` (Bearer token auth) |
| Gateway -> service | HTTP/1.1 over UDS | `~/.capsem/run/service.sock` |
| CLI/MCP -> service | HTTP/1.1 over UDS | `~/.capsem/run/service.sock` |
| Service -> process | MessagePack over UDS | `~/.capsem/run/instances/{id}.sock` |
| Process -> guest agent | Binary frames over vsock | ports 5000 (control), 5001 (terminal), 5004 (lifecycle), 5005 (exec) |

### Service HTTP API

The service and gateway expose one explicit route table. Unknown routes must
return 404; do not add compatibility aliases or generic gateway forwarding. The full
contract lives in `docs/src/content/docs/architecture/service-api.md`; the
common session routes are:

Every method/path pair is also locked in `config/public-surface.toml`.
`tests/test_public_surface_contract.py` derives the Axum route table and fails
on any unapproved addition, removal, rename, method change, or count drift.
Do not update that ledger merely to make a test green; changing the HTTP
surface requires explicit product/API approval.

| Method | Path | Purpose |
|--------|------|---------|
| POST | `/vms/create` | Create a session from a profile |
| GET | `/vms/list` | List sessions and profile/status metadata |
| GET | `/vms/{id}/info` | Session identity, profile, config, and diagnostics |
| GET | `/vms/{id}/status` | Hot in-memory runtime state and counters |
| POST | `/vms/{id}/exec` | Execute command, return stdout/stderr/exit_code |
| POST | `/run` | One-shot create + exec + destroy through the same service path |
| POST | `/vms/{id}/stop` | Stop a running session |
| POST | `/vms/{id}/pause` | Pause/suspend a running session |
| POST | `/vms/{id}/start` | Start a stopped session |
| POST | `/vms/{id}/resume` | Resume a paused/stopped session |
| POST | `/vms/{id}/save` | Save current session state |
| POST | `/vms/{id}/fork` | Fork a session into reusable state |
| DELETE | `/vms/{id}/delete` | Destroy session and wipe state |
| POST | `/purge` | Delete defunct/incompatible service state |
| POST | `/vms/{id}/files/write` | Write file to guest |
| POST | `/vms/{id}/files/read` | Read file from guest |
| GET | `/vms/{id}/files/list` | List guest files |
| GET | `/vms/{id}/files/content` | Download file content |
| POST | `/vms/{id}/files/content` | Upload file content |
| GET | `/vms/{id}/logs` | Serial/boot logs |

### MCP tools (capsem-mcp)

MCP tools include `capsem_create`, `capsem_list`, `capsem_info`, `capsem_exec`,
`capsem_run`, lifecycle tools, file read/write, logs, timeline, triage,
version, fork, and profile MCP tools. Raw SQL inspection tools are not part of
the product surface; telemetry access must use typed routes.

## Host-guest communication

All host-guest communication flows through capsem-process via vsock. There is no direct vsock access from any other host binary.

```
Interactive shell:  capsem-process -> vsock:5001 <-> Guest PTY (bash)
Exec command:       capsem-process -> vsock:5000 (Exec cmd) -> Guest agent
                    capsem-process <- vsock:5005 (stdout)    <- Guest child process
                    capsem-process <- vsock:5000 (ExecDone)  <- Guest agent
File I/O:           capsem-process -> vsock:5000 (FileWrite/FileRead) <-> Guest agent
```

Terminal I/O flows through vsock port 5001 (raw PTY bytes). Exec output flows on a dedicated port 5005 connection -- completely separated from the interactive terminal. File I/O uses port 5000 (control channel).

Serial console stays active for kernel boot logs. Terminal I/O switches to vsock once the guest agent sends `Ready`.

### Vsock ports

| Port | Purpose |
|------|---------|
| 5000 | Control messages (resize, heartbeat, exec commands, file I/O) |
| 5001 | Terminal data (PTY I/O) |
| 5002 | MITM proxy and framed guest MCP endpoint |
| 5004 | Lifecycle commands (suspend; deprecated shutdown frames ignored, capsem-sysutil) |
| 5005 | Exec output (direct child process stdout, on demand) |

## Guest agent architecture

All guest binaries live in `crates/capsem-agent/` and are cross-compiled for `aarch64-unknown-linux-musl` (and `x86_64-unknown-linux-musl`). Deployed chmod 555 (read-only) into the initrd at `/run/`.

### capsem-pty-agent (main agent)

Single-threaded, sync Rust binary (no tokio). Launched by capsem-init after filesystems are mounted.

**Boot sequence:**
1. Connect to host on vsock:5001 (terminal) and vsock:5000 (control)
2. Send `GuestToHost::Ready` with agent version
3. Boot handshake: receive `BootConfig` (clock sync), then `SetEnv`/`FileWrite` messages, then `BootConfigDone`
4. Apply env vars, write files, set hostname from `CAPSEM_VM_NAME`
5. Open PTY pair, fork bash on the slave side
6. Send `GuestToHost::BootReady` + `BootTiming` (parsed from capsem-init's JSONL)
7. Enter bridge loop

**Runtime -- two loops running concurrently:**
- **bridge_loop** (main thread): polls master PTY, forwards output to vsock:5001. Spawns a dedicated thread for the reverse direction (vsock -> PTY). Pure bidirectional byte bridge with no scanning or filtering.
- **control_loop** (background thread): reads vsock:5000, handles `Resize` (set winsize + SIGWINCH), `Ping`/`Pong` heartbeat, `Exec` (spawns background thread for direct child process), `FileWrite`/`FileRead`/`FileDelete`, and `Shutdown`.

**Exec mechanism:** spawns `bash -c '<cmd> 2>&1'` as a direct child process (not via PTY). Connects to host on vsock:5005, sends `ExecStarted { id }` handshake, then streams child stdout to the exec port. Exit code comes from `waitpid`, sent as `ExecDone { id, exit_code }` on vsock:5000. Runs in a background thread so control_loop stays responsive to heartbeats during long commands.

**Shutdown handler:** `sync()` -> `SIGTERM` bash -> wait `SHUTDOWN_GRACE_SECS` (defined in `capsem-proto`) -> `SIGKILL` (interactive bash ignores SIGTERM) -> break. The bridge loop cleanup then sends SIGHUP + waitpid to reap the child.

### capsem-sysutil (guest suspend helper)

Busybox-pattern binary dispatching on `argv[0]`. Symlinked by capsem-init:
- `/usr/local/bin/suspend` -> `/run/capsem-sysutil`

Opens its own vsock:5004 connection (independent of capsem-pty-agent) and sends `GuestToHost::SuspendRequest`. Shows a countdown (`SHUTDOWN_GRACE_SECS + 1` seconds) before sending. `shutdown`, `halt`, and `poweroff` return an error; `reboot` remains unsupported. The host ignores old `GuestToHost::ShutdownRequest` frames for wire compatibility.

**Suspend flow (end-to-end):**
```
Guest: suspend -> capsem-sysutil -> vsock:5004 -> capsem-process
  capsem-process: reads SuspendRequest -> sends ProcessToService::SuspendRequested to service
  capsem-process: saves VM state and exits cleanly
  capsem-service: marks persistent VM suspended for resume
```

### capsem-net-proxy

Listens on localhost:10443 inside the guest. iptables redirects all port 443 traffic here. Each connection is bridged to host vsock:5002 where the network intercept handles TLS termination, protocol parsing, and handoff to the security engine.

### capsem-mcp-server

Guest MCP relay. Reads MCP JSON-RPC on stdin/stdout and carries it to the host MITM MCP endpoint as framed records over vsock:5002.
