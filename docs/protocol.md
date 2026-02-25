# Capsem Protocol

This document describes the vsock control protocol between the host application and the guest agent.

## Vsock Ports

| Port | Purpose |
|------|---------|
| 5000 | Control messages (resize, heartbeat, exec, boot handshake) |
| 5001 | Terminal data (raw PTY I/O, bidirectional) |
| 5002 | SNI proxy (HTTPS connections bridged to host) |

## Wire Format

All control messages on port 5000 use length-prefixed MessagePack framing:

```
[4-byte BE length][MessagePack payload]
```

- **Length prefix**: unsigned 32-bit big-endian, giving the size of the payload in bytes.
- **Payload**: MessagePack-encoded message using serde's `#[serde(tag = "t", content = "d")]` encoding (internally tagged with a content field).
- **Max frame size**: 8192 bytes (8KB). Frames exceeding this are rejected before reading the payload.

Port 5001 (terminal data) carries raw bytes with no framing -- it's a bidirectional byte stream between the host and the guest PTY master fd.

Port 5002 (SNI proxy) carries raw TLS traffic -- each accepted connection is a TLS session bridged to the real server on the host.

## Message Types

### Host-to-Guest (`HostToGuest`)

| Variant | Fields | Description |
|---------|--------|-------------|
| `BootConfig` | `epoch_secs: u64`, `env_vars: [(String, String)]` | Clock sync + environment injection at boot |
| `Resize` | `cols: u16`, `rows: u16` | Terminal resize request |
| `Exec` | `id: u64`, `command: String` | Execute command in guest PTY |
| `Ping` | -- | Liveness check |
| `FileWrite` | `path: String`, `data: [u8]`, `mode: u32` | Inject file into guest (reserved) |
| `FileRead` | `id: u64`, `path: String` | Request file content (reserved) |
| `FileDelete` | `path: String` | Delete file in guest (reserved) |
| `Shutdown` | -- | Graceful shutdown request |

### Guest-to-Host (`GuestToHost`)

| Variant | Fields | Description |
|---------|--------|-------------|
| `Ready` | `version: String` | Agent alive, waiting for boot config |
| `BootReady` | -- | Boot config applied, terminal ready |
| `ExecDone` | `id: u64`, `exit_code: i32` | Command completed with exit code |
| `Pong` | -- | Heartbeat response |
| `FileCreated` | `path: String`, `size: u64` | Telemetry: file created (reserved) |
| `FileModified` | `path: String`, `size: u64` | Telemetry: file modified (reserved) |
| `FileDeleted` | `path: String` | Telemetry: file deleted (reserved) |
| `FileContent` | `id: u64`, `path: String`, `data: [u8]` | Response to FileRead (reserved) |

## Host State Machine

All state tracking and protocol enforcement lives on the host (zero-trust on guest binaries). The host tracks its lifecycle via `HostState`:

```
                    +---> Error ---> Stopped
                    |
Created ---> Booting ---> VsockConnected ---> Handshaking ---> Running ---> ShuttingDown ---> Stopped
                |              |                   |               |
                +--> Stopped   +--> Stopped        +--> Stopped   +--> Stopped
```

| State | Description |
|-------|-------------|
| `Created` | VM object created, not yet started |
| `Booting` | VM started, serial console active, waiting for vsock connections |
| `VsockConnected` | Both vsock ports (5000, 5001) connected from guest |
| `Handshaking` | Ready received, BootConfig sent, awaiting BootReady |
| `Running` | Boot handshake complete, terminal interactive |
| `ShuttingDown` | Graceful shutdown requested |
| `Stopped` | VM stopped (terminal state) |
| `Error` | Unrecoverable error (can transition to Stopped) |

Every non-terminal state can transition to `Error`. `Error` can only transition to `Stopped`. `Stopped` is terminal -- no transitions out.

## Boot Handshake Sequence

```
Host                                Guest Agent
 |                                      |
 |  [VM starts, serial active]          |
 |                                      |
 |  [vsock:5001 connect] <------------- |  (terminal data port)
 |  [vsock:5000 connect] <------------- |  (control port)
 |                                      |
 |  Host: Booting -> VsockConnected     |
 |                                      |
 |  <--- Ready { version: "0.3.0" } --- |
 |                                      |
 |  Host: VsockConnected -> Handshaking |
 |                                      |
 |  --- BootConfig { epoch_secs, ... }  |
 |                  env_vars } -------> |
 |                                      |  [set clock, apply env vars, fork bash]
 |  <--- BootReady --------------------- |
 |                                      |
 |  Host: Handshaking -> Running        |
 |                                      |
 |  [serial forwarding stopped]         |
 |  [vsock PTY bridge active]           |
```

## Message Validation Rules

The host validates both outbound (host-to-guest) and inbound (guest-to-host) messages against its current state:

### Host-to-Guest validation

| Message | Valid in state |
|---------|---------------|
| `BootConfig` | Handshaking only |
| `Shutdown` | Any state |
| All others | Running only |

### Guest-to-Host validation

| Message | Valid in state |
|---------|---------------|
| `Ready` | VsockConnected only |
| `BootReady` | Handshaking only |
| All others | Running only |

Messages that arrive in an invalid state are logged and dropped.

## Exec Protocol

The host sends `Exec { id, command }` to inject a command into the guest PTY. The guest agent:

1. Disables terminal echo (`ECHO` off)
2. Writes the command + newline to the PTY master
3. Appends a sentinel: `echo $'\\e_CAPSEM_EXIT:{id}:$?\\e\\\\'`
4. Re-enables echo (`ECHO` on)

The agent scans PTY output for the sentinel `ESC _ CAPSEM_EXIT:{id}:{exit_code} ESC \`. When found, it sends `ExecDone { id, exit_code }` to the host and strips the sentinel from the terminal output.

## Security Invariants

1. **Disjoint types (T14)**: `HostToGuest` and `GuestToHost` use different serde tag values. Cross-type decoding fails at deserialization.
2. **Frame cap**: 8KB max frame size prevents memory exhaustion from malicious frames.
3. **State enforcement**: The host rejects messages invalid for the current lifecycle stage. A compromised guest cannot send BootConfig (host-only) or trigger out-of-sequence state changes.
4. **Zero-trust guest**: All validation lives on the host. The guest agent has no state machine, no message validation, and no policy enforcement. See `docs/security.md` for the full rationale.
5. **Type-safe API**: Separate `encode_host_msg`/`decode_host_msg` and `encode_guest_msg`/`decode_guest_msg` functions prevent accidentally using the wrong direction.
