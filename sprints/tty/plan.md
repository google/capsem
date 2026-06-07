# Sprint: Host-Side Command Recording & PTY History

Record all bash commands inside VMs from the host to prevent spoofing. Restore terminal history on VM reconnect. Expose history through CLI, API, and frontend.

## Context

Every observable is recorded today (network, AI calls, MCP tools, file changes, snapshots) -- except shell commands. Commands are the primary way agents and humans interact with VMs, and enterprises need an auditable record. Recording must happen from the host: guest cannot tamper with, suppress, or fabricate records.

## Architecture

Three recording layers, each independent and complementary:

```
                    capsem-service (gateway)
                         |
          +--------------+--------------+
          |              |              |
     /history      /inspect        /terminal/{id}
     (dedicated)   (SQL query)     (reconnect+replay)
          |              |              |
          +--------------+--------------+
                         |
                  capsem-process (per-VM)
                    |    |    |
         +----------+    |    +----------+
         |               |               |
   Layer 1:         Layer 2:        Layer 3:
   exec_events      PTY transcript   audit_events
   (session.db)     (pty.log file)   (session.db)
         |               |               |
   vsock 5000       vsock 5001      vsock 5006 (new)
   control chan     terminal I/O    audit stream
         |               |               |
         +-------+-------+-------+-------+
                 |               |
           capsem-agent     auditd/dispatcher
              (guest)          (guest kernel)
```

---

## Layer 1: Structured Exec Recording

**What**: Every `Exec` command sent via the structured API path (AI agents use this via MCP `capsem_exec` -> service -> process -> guest). The host has the full command string before it reaches the guest.

**Tamper-proof**: Logged in `capsem-process` at the moment the `ServiceToProcess::Exec { id, command }` IPC message arrives -- before forwarding to guest via vsock. Guest never touches this record.

### Schema: `exec_events`

```sql
CREATE TABLE IF NOT EXISTS exec_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    timestamp TEXT NOT NULL,
    exec_id INTEGER NOT NULL,
    command TEXT NOT NULL,
    exit_code INTEGER,
    duration_ms INTEGER,
    stdout_preview TEXT,
    stderr_preview TEXT,
    stdout_bytes INTEGER DEFAULT 0,
    stderr_bytes INTEGER DEFAULT 0,
    -- Process attribution
    source TEXT NOT NULL DEFAULT 'api',  -- 'mcp', 'cli', 'api', 'frontend'
    mcp_call_id INTEGER,                 -- FK to mcp_calls if MCP-triggered
    trace_id TEXT,                       -- links to model_calls.trace_id
    process_name TEXT,                   -- requesting process ("claude", "codex")
    pid INTEGER                          -- guest PID (from ExecStarted)
);
CREATE INDEX IF NOT EXISTS idx_exec_events_timestamp ON exec_events(timestamp);
CREATE INDEX IF NOT EXISTS idx_exec_events_exec_id ON exec_events(exec_id);
CREATE INDEX IF NOT EXISTS idx_exec_events_trace_id ON exec_events(trace_id);
CREATE INDEX IF NOT EXISTS idx_exec_events_source ON exec_events(source);
```

Attribution: `source` tracks request origin. `mcp_call_id` links to the MCP gateway log. `trace_id` links to the model call that triggered it. Full chain: which AI model -> which turn -> which tool call -> which command.

### Recording points

**Start** (`capsem-process/src/vsock.rs` ~line 478, command handler thread):
```rust
ServiceToProcess::Exec { id, command } => {
    db.write(WriteOp::ExecEvent(ExecEvent {
        timestamp: SystemTime::now(),
        exec_id: id,
        command: command.clone(),
        exit_code: None,
        ..Default::default()
    }));
    ctrl_cmd_tx.send(HostToGuest::Exec { id, command });
}
```

**Complete** (control reader, ~line 376 on `GuestToHost::ExecDone`):
```rust
db.write(WriteOp::ExecEventComplete(ExecEventComplete {
    exec_id: id,
    exit_code,
    duration_ms,
    stdout_preview: truncated_stdout,
    stderr_preview: truncated_stderr,
    stdout_bytes: total_stdout,
    stderr_bytes: total_stderr,
}));
```

### Plumbing

`Arc<DbWriter>` exists in `capsem-process/src/main.rs` (line 121). Thread it through `VsockOptions` to the command handler thread. Main structural change.

---

## Layer 2: Raw PTY Transcript

**What**: Every byte flowing through vsock port 5001 (terminal channel), both directions, with timestamps and direction tags. Complete, byte-perfect record of the interactive terminal session.

**Tamper-proof**: Recording at the vsock boundary in `capsem-process`. Guest cannot suppress bytes already sent through the hypervisor-mediated vsock, inject fake packets, or alter the host-side file.

### File format

Append-only binary at `{session_dir}/pty.log`:

```
[1 byte: direction (0x00=input, 0x01=output)]
[8 bytes: timestamp (microseconds since epoch, LE u64)]
[4 bytes: length (LE u32)]
[{length} bytes: raw terminal data]
```

Simple, no compression -- trivially parseable by external audit tools, CLI, and VTE replayer.

**Why a file, not session.db**: Terminal I/O is high-volume, bursty, append-only. SQLite WAL with thousands of BLOBs/sec would bloat the DB and contend with telemetry writes. A dedicated file gives zero contention, efficient sequential reads, and easy external tool consumption.

### Recording points

- **Output** (guest->host): `capsem-process/src/vsock.rs` ~line 224, terminal output coalesce flush callback. After `serial.log` + `term_out`, append to `pty.log`.
- **Input** (host->guest): `ServiceToProcess::TerminalInput` handler ~line 476. Before writing to `term_f_write`, append to `pty.log`.

### Coalescing

Output already uses `CoalesceBuffer` (5ms/1MB). Input needs similar (~50ms) to avoid per-keystroke writes.

### Storage

Heavy 8h interactive: ~50MB. Typical AI agent sessions: ~1-5MB. Persistent VMs accumulate.

### Rotation

Cap `pty.log` at 10-50MB (configurable, default 20MB) and rotate. When the file hits the cap:
1. Rename `pty.log` -> `pty.log.1` (keep one rotated file for archival)
2. Take a final VTE snapshot at the rotation boundary
3. Start fresh `pty.log`

This aligns with VTE snapshot-based replay: even after rotation, reconnect loads the latest snapshot + delta from the current `pty.log`. Replaying a 100MB+ file is infeasible regardless of snapshots.

### Concurrency

`capsem-process` appends to `pty.log` continuously while `capsem-service` reads from it to serve `/history/{id}/transcript` API requests. On Unix, `OpenOptions::new().read(true)` does not block the writer, but ensure no exclusive locks are accidentally acquired (no `flock`, no `O_EXCL`). Both sides open independently: writer with append, reader with read-only.

---

## Layer 3: Kernel Audit (execve)

**What**: Every `execve` syscall in the guest -- shell commands, script-spawned processes, subshells, command substitutions, background jobs, cron, direct `exec()` from Python/Node. Captures everything the other layers miss.

**Tamper-proof**:
- `CONFIG_AUDIT` + `CONFIG_AUDITSYSCALL` compiled into guest kernel
- `auditctl -e 2` (immutable mode) set during capsem-init before any user code
- Immutable mode locks audit config until reboot -- even root cannot disable
- Records streamed to host via dedicated vsock in real-time
- capsem-agent (PID 1, read-only binary) does the streaming
- If streaming dies, kernel buffers records; agent can reconnect and drain

### Kernel changes

Add to `guest/config/kernel/defconfig.arm64` and `defconfig.x86_64`:

```
CONFIG_AUDIT=y
CONFIG_AUDITSYSCALL=y
CONFIG_AUDIT_ARCH_COMPAT_GENERIC=y
```

Minimal size increase (~50KB). Standard in all enterprise Linux distros.

### Guest-side setup

In `guest/artifacts/capsem-init` (before user code):

```bash
# Start auditd (generates /var/log/audit/audit.log)
auditd &
# Wait for auditd socket
sleep 0.1

auditctl -a always,exit -F arch=b64 -S execve -k capsem_exec
auditctl -e 2  # immutable until reboot
```

Guest image must include `auditd` binary (add to rootfs build).

### Streaming architecture

New vsock port 5006 (`VSOCK_PORT_AUDIT`):

```
Guest kernel audit -> auditd -> /var/log/audit/audit.log
    -> capsem-agent tail reader thread
        -> filter for capsem_exec key
        -> correlate multi-line records (SYSCALL + EXECVE + PROCTITLE share audit ID)
        -> serialize to MessagePack
        -> write to vsock 5006
            -> capsem-process audit receiver thread
                -> deserialize MessagePack records
                -> WriteOp::AuditEvent -> session.db
```

**Why auditd, not raw netlink**: Linux audit netlink messages are notoriously difficult to parse natively. They are not clean binary structs -- they are stringy, space-delimited key-value pairs that span multiple correlated netlink messages (e.g., SYSCALL, EXECVE, and PROCTITLE messages all share the same audit ID and must be joined). `auditd` handles this correlation reliably. Since guest filesystems are typically memory-backed (tmpfs/overlay), the "disk I/O" of tailing a log file is negligible.

### Schema: `audit_events`

```sql
CREATE TABLE IF NOT EXISTS audit_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    timestamp TEXT NOT NULL,
    -- Process tree context
    pid INTEGER NOT NULL,
    ppid INTEGER NOT NULL,
    uid INTEGER NOT NULL,
    exe TEXT NOT NULL,                -- /usr/bin/python3
    comm TEXT,                        -- python3
    argv TEXT NOT NULL,               -- full command line
    cwd TEXT,                         -- working directory at exec time
    exit_code INTEGER,
    -- Process lineage
    session_id INTEGER,               -- kernel session ID (tty grouping)
    tty TEXT,                         -- pts/0 or none
    -- Correlation
    audit_id TEXT,                    -- kernel audit event ID
    exec_event_id INTEGER,            -- FK to exec_events if spawned by structured exec
    parent_exe TEXT                   -- parent exe (quick "bash spawned python" queries)
);
CREATE INDEX IF NOT EXISTS idx_audit_events_timestamp ON audit_events(timestamp);
CREATE INDEX IF NOT EXISTS idx_audit_events_exe ON audit_events(exe);
CREATE INDEX IF NOT EXISTS idx_audit_events_pid ON audit_events(pid);
CREATE INDEX IF NOT EXISTS idx_audit_events_ppid ON audit_events(ppid);
```

- `tty`: distinguishes interactive (pts/0) from background (none)
- `parent_exe`: quick queries without self-joining on ppid
- `exec_event_id`: correlates audit records back to structured exec (e.g., `bash -c "make"` triggers dozens of child execve calls, all linked to the original exec_event)

### Proto changes

```rust
// capsem-proto/src/lib.rs
pub const VSOCK_PORT_AUDIT: u32 = 5006;

pub struct AuditRecord {
    pub timestamp_us: u64,
    pub pid: u32,
    pub ppid: u32,
    pub uid: u32,
    pub exe: String,
    pub comm: Option<String>,
    pub argv: String,
    pub cwd: Option<String>,
    pub tty: Option<String>,
    pub session_id: Option<u32>,
    pub parent_exe: Option<String>,
}
```

### Volume management

`make` can spawn hundreds of processes. Mitigations:
- Kernel-level filter: only `execve` with `capsem_exec` key
- DbWriter batching (existing infrastructure)
- Rate limiting/sampling for extreme cases (configurable)
- **No pruning by default.** SQLite handles millions of rows fine with proper indexes. For corporate compliance, dropping records is worse than a large session.db. Pruning can be added as an opt-in config later if needed.

---

## PTY History Restoration

### Problem

Persistent VM stopped and resumed, or user disconnects and reconnects: terminal shows blank. Previous output lost.

### Solution

Use `pty.log` as source of truth. On reconnect:
1. Read full output-direction bytes from `pty.log`
2. Feed through host-side VTE state machine (`vte` crate) to reconstruct terminal state
3. Extract last N lines of scrollback (default 500, configurable)
4. Send reconstructed state to client before live-streaming begins

### VTE replay module

New: `crates/capsem-process/src/pty_replay.rs`

```rust
pub struct PtyReplayer {
    parser: vte::Parser,
    performer: ScreenState,
}

impl PtyReplayer {
    pub fn feed(&mut self, data: &[u8]);
    pub fn tail_lines(&self, n: usize) -> Vec<u8>;  // includes ANSI escapes
    pub fn screen_state(&self) -> ScreenSnapshot;     // pixel-perfect restore
}
```

### VTE snapshots (performance)

Replaying the entire `pty.log` through a VTE state machine on every reconnect becomes a bottleneck for persistent VMs. Terminal state is highly stateful -- correct colors and cursor positions for the last 500 lines require parsing every ANSI escape from byte 0. A 50MB `pty.log` would cause a noticeable latency spike and CPU burn.

**Solution**: Periodically snapshot the VTE `ScreenState` (every 1MB of output) and store alongside `pty.log`:

```
{session_dir}/pty.log             # append-only raw transcript
{session_dir}/pty_snapshots/      # periodic VTE state snapshots
  snapshot_00000000.bin           # at byte offset 0
  snapshot_01048576.bin           # at byte offset 1MB
  snapshot_02097152.bin           # at byte offset 2MB
  ...
```

Each snapshot contains serialized `ScreenState` + the `pty.log` byte offset it corresponds to. On reconnect, load the latest snapshot and replay only the delta from that offset forward.

```rust
pub struct PtyReplayer {
    parser: vte::Parser,
    performer: ScreenState,
    snapshot_interval: usize,       // default 1MB
    bytes_since_snapshot: usize,
}

impl PtyReplayer {
    pub fn feed(&mut self, data: &[u8]);
    pub fn tail_lines(&self, n: usize) -> Vec<u8>;
    pub fn screen_state(&self) -> ScreenSnapshot;
    pub fn maybe_snapshot(&mut self, offset: u64) -> Option<ScreenSnapshot>;
}
```

Snapshotting is driven by the same code path that appends to `pty.log` -- after each write, check if `bytes_since_snapshot >= snapshot_interval` and persist if so.

### Reconnect flow

```
Client connects to /terminal/{id} WebSocket
  -> capsem-process checks pty.log exists
  -> Find latest VTE snapshot (if any)
  -> Load snapshot, seek pty.log to snapshot offset
  -> Replay only the delta bytes through PtyReplayer
  -> Send tail_lines(500) as initial WebSocket message
  -> Switch to live streaming (existing terminal bridge)
```

Without snapshots (new VM, no snapshots yet): falls back to full replay from byte 0. This is fine for small logs.

---

## API Surface

### `GET /history/{id}`

Command history for a VM. Merges `exec_events` and `audit_events`, sorted by timestamp desc.

Query params:
- `limit` (default 500), `offset` (pagination)
- `search` (substring, SQL LIKE across command text)
- `layer` -- `all`, `exec`, `audit`
- `process` -- filter by process name or exe path
- `trace_id` -- filter by AI agent trace (all commands from one turn)

Response:
```json
{
  "ok": true,
  "data": {
    "commands": [
      {
        "timestamp": "2026-04-14T10:23:45Z",
        "layer": "exec",
        "command": "pip install numpy",
        "exit_code": 0,
        "duration_ms": 3200,
        "stdout_preview": "Successfully installed numpy-2.0.0",
        "process": {
          "source": "mcp",
          "process_name": "claude",
          "trace_id": "tr_abc123",
          "mcp_call_id": 42
        }
      },
      {
        "timestamp": "2026-04-14T10:22:10Z",
        "layer": "audit",
        "command": "/usr/bin/git status",
        "exit_code": 0,
        "process": {
          "pid": 1234,
          "ppid": 1100,
          "exe": "/usr/bin/git",
          "parent_exe": "/bin/bash",
          "tty": "pts/0",
          "cwd": "/root/project"
        }
      }
    ],
    "total": 847,
    "has_more": true
  }
}
```

### `GET /history/{id}/transcript?tail_lines=500`

PTY transcript tail as reconstructed terminal output (with ANSI escapes). Base64-encoded, suitable for xterm.js.

```json
{
  "ok": true,
  "data": {
    "lines": 500,
    "total_lines": 12847,
    "bytes": 48230,
    "content": "<base64-encoded terminal output>"
  }
}
```

### `GET /history/{id}/processes`

Process-centric view: group commands by executable.

```json
{
  "ok": true,
  "data": {
    "processes": [
      {
        "exe": "/bin/bash",
        "command_count": 847,
        "first_seen": "2026-04-14T09:01:00Z",
        "last_seen": "2026-04-14T18:29:00Z",
        "children_count": 312
      },
      {
        "exe": "/usr/bin/python3",
        "command_count": 156,
        "first_seen": "2026-04-14T09:15:00Z",
        "last_seen": "2026-04-14T18:20:00Z",
        "triggered_by": ["exec", "audit"]
      }
    ]
  }
}
```

### `GET /history/{id}/export`

Full audit export for corporate SIEM/compliance:

```json
{
  "ok": true,
  "data": {
    "vm_id": "abc123",
    "vm_name": "dev-sandbox",
    "session_start": "2026-04-14T09:00:00Z",
    "session_end": "2026-04-14T18:30:00Z",
    "exec_events": [...],
    "audit_events": [...],
    "process_tree": {
      "1": { "exe": "/run/capsem-pty-agent", "children": [100] },
      "100": { "exe": "/bin/bash", "tty": "pts/0", "children": [1234, 1300] },
      "1234": { "exe": "/usr/bin/pip", "children": [] }
    },
    "summary": {
      "total_exec_events": 142,
      "total_audit_events": 3847,
      "unique_executables": 28,
      "top_executables": ["/bin/bash", "/usr/bin/python3", "/usr/bin/git"]
    },
    "transcript_size_bytes": 5242880,
    "transcript_blake3": "a1b2c3..."
  }
}
```

`transcript_blake3` provides integrity verification. `process_tree` reconstructs parent-child relationships from audit records.

### Existing endpoints

- `/inspect/{id}` -- no changes. `exec_events` and `audit_events` in session.db, `query_raw(sql)` already works.
- `/info/{id}` -- add `exec_count` and `audit_event_count` to telemetry enrichment.
- Gateway -- no changes. Already proxies all unmatched paths to service via UDS.

---

## CLI: `capsem history`

```
capsem history <session>                     # last 500 commands
capsem history <session> --tail 100          # last 100
capsem history <session> --all               # full history
capsem history <session> --search "pip install"
capsem history <session> --layer exec        # only structured exec
capsem history <session> --layer audit       # only kernel audit
capsem history <session> --process python3   # filter by process
capsem history <session> --trace tr_abc123   # all commands from one AI turn
capsem history <session> --tree              # process tree view
capsem history <session> --json              # machine-readable
capsem history <session> --export            # full audit export
```

Default output:
```
 TIMESTAMP            LAYER   EXIT  PROCESS       COMMAND
 2026-04-14 10:23:45  exec      0   claude/mcp    pip install numpy
 2026-04-14 10:22:10  audit     0   bash>git      git status
 2026-04-14 10:22:08  audit     0   bash          git status
 2026-04-14 10:21:55  exec      1   claude/mcp    python train.py
 ...
 Showing 500 of 847 commands. Use --all for full history.
```

Process tree view (`--tree`):
```
 capsem-pty-agent (PID 1)
   bash (PID 100, pts/0)
     pip install numpy        exit=0  3.2s  [exec: claude/mcp tr_abc123]
       python3 setup.py       exit=0  2.8s  [audit]
     git status               exit=0  0.1s  [exec: claude/mcp tr_abc124]
     python train.py          exit=1  45.2s [exec: claude/mcp tr_abc125]
       python3 train.py       exit=1  45.0s [audit]
         python3 -c "import torch"  exit=0  0.3s [audit]
```

Uses existing `UdsClient` pattern in `crates/capsem/src/client.rs`.

---

## Frontend UI

### History view

New `HistoryView.svelte` in VM detail panel sidebar (alongside Logs, Stats, Inspector):

- Table: timestamp, layer (exec/audit), command, exit code, duration, process
- Search bar: real-time filtering via `search` query param
- Layer filter: dropdown for all/exec/audit
- Process filter: dropdown populated from `/history/{id}/processes`
- Click to expand: stdout/stderr preview (exec), full argv + cwd + process tree context (audit)
- Auto-refresh: poll when VM is running

### Terminal reconnect

On terminal view connect to running/resumed VM:
1. Fetch `/history/{id}/transcript?tail_lines=500`
2. Write base64-decoded bytes to xterm.js terminal
3. Attach live WebSocket stream

Seamless continuity -- recent history followed by live output.

---

## Cross-Session Index

Add to `main.db` (`capsem-core/src/session/index.rs`):

```sql
ALTER TABLE session_records ADD COLUMN exec_count INTEGER DEFAULT 0;
ALTER TABLE session_records ADD COLUMN audit_event_count INTEGER DEFAULT 0;
```

Enables: "which sessions ran `rm -rf`" from `/stats` or `/inspect/_main`.

---

## Security Analysis

### Threat matrix

| Threat | Layer 1 (exec) | Layer 2 (PTY) | Layer 3 (audit) |
|--------|:-:|:-:|:-:|
| Guest suppresses record | Host logs before guest sees it | Host records at vsock boundary | auditctl -e 2 immutable |
| Guest fabricates commands | Host is sole writer | Host is sole writer | Kernel generates, not userspace |
| Guest alters command text | Host has original string | Raw bytes are ground truth | Kernel captures actual execve argv |
| Guest hides background procs | N/A (structured exec only) | Not visible in PTY | Captured (all execve) |
| Guest disables recording | Recording in host process | Recording in host process | Immutable mode until reboot |
| Guest modifies audit binary | Read-only rootfs, chmod 555 | N/A | Kernel audit, not userspace |
| Root bypasses audit | N/A | N/A | auditctl -e 2 prevents even root |
| Guest spoofs attribution | source/trace_id set by host | N/A | pid/ppid/exe from kernel |
| Guest fakes parent process | N/A | N/A | ppid kernel-tracked, unforgeable |

### What remains unrecorded

- Pure computation (no exec): Python that never spawns subprocess. Needs VM introspection (out of scope).
- Kernel modules: mitigated by `CONFIG_MODULES=n` in guest kernel.

### Corporate audit guarantees

- **Completeness**: Three independent layers, no command unrecorded
- **Integrity**: PTY transcript verified with BLAKE3 hash; audit records kernel-generated
- **Non-repudiation**: All records timestamped, stored on host; guest has no write access
- **Attribution**: Every command traceable to process (pid/exe from kernel), AI turn (trace_id), or MCP call (mcp_call_id). Process tree reconstruction shows full lineage
- **Searchability**: SQL via `/inspect/{id}`, dedicated `/history/{id}` with filtering, `capsem history` CLI
- **Exportability**: `/history/{id}/export` with process tree for SIEM ingestion. Raw `pty.log` downloadable with BLAKE3 verification

---

## Files to modify

### Kernel/Guest
- `guest/config/kernel/defconfig.arm64` -- CONFIG_AUDIT, CONFIG_AUDITSYSCALL
- `guest/config/kernel/defconfig.x86_64` -- same
- `guest/artifacts/capsem-init` -- start auditd, auditctl rules + immutable mode
- `guest/config/` -- add auditd to rootfs build (Dockerfile or guest config)
- `crates/capsem-agent/src/main.rs` -- audit log tail reader thread, stream to vsock 5006

### Protocol
- `crates/capsem-proto/src/lib.rs` -- VSOCK_PORT_AUDIT, AuditRecord type

### Logger
- `crates/capsem-logger/src/schema.rs` -- exec_events + audit_events tables, migrations
- `crates/capsem-logger/src/events.rs` -- ExecEvent, AuditEvent structs
- `crates/capsem-logger/src/writer.rs` -- WriteOp::ExecEvent, WriteOp::AuditEvent
- `crates/capsem-logger/src/reader.rs` -- history query methods

### Per-VM Process
- `crates/capsem-process/src/vsock.rs` -- exec recording, audit port listener, PTY transcript recording
- `crates/capsem-process/src/main.rs` -- thread DbWriter through, PTY log setup
- `crates/capsem-process/src/pty_replay.rs` -- new: VTE replay with periodic snapshots for reconnect
- `crates/capsem-process/src/pty_log.rs` -- new: PTY transcript file I/O, rotation, concurrency

### Service
- `crates/capsem-service/src/main.rs` -- /history endpoints, terminal reconnect replay
- `crates/capsem-service/src/api.rs` -- HistoryResponse, TranscriptResponse, ProcessesResponse, ExportResponse

### CLI
- `crates/capsem/src/main.rs` -- `capsem history` subcommand

### Frontend
- `frontend/src/lib/api.ts` -- history API functions
- `frontend/src/lib/components/views/HistoryView.svelte` -- new: command history UI
- Terminal view -- transcript replay on connect

---

## Resolved decisions

1. **Audit record format on vsock 5006**: MessagePack (via `rmp-serde`). Same encoding as the control channel on vsock 5000. Already a dependency in `capsem-proto` and `capsem-agent`.

2. **PTY transcript rotation**: Cap at 10-50MB (default 20MB) and rotate. VTE snapshot-based replay makes replaying 100MB+ infeasible anyway.

3. **Audit event pruning**: No pruning by default. SQLite handles millions of rows with proper indexes. Corporate compliance requires full retention -- dropping records is worse than a large session.db.

4. **Audit reader in capsem-agent**: Tail `/var/log/audit/audit.log` generated by auditd. Netlink audit messages are stringy key-value pairs spanning multiple correlated messages that require complex joining. auditd handles correlation reliably. tmpfs-backed guest filesystem makes log file I/O negligible.

---

## Implementation Status (2026-04-15)

All recording infrastructure, API, and CLI are implemented. 632 tests pass across all modified crates. Remaining work is VTE replay and frontend UI.

### Done

| Area | What | Files |
|------|------|-------|
| Protocol | `VSOCK_PORT_AUDIT=5006`, `AuditRecord` struct, encode/decode, 4 tests | `capsem-proto/src/lib.rs` |
| Logger schema | `exec_events` + `audit_events` tables, migrations, indexes | `capsem-logger/src/schema.rs` |
| Logger events | `ExecEvent`, `ExecEventComplete`, `AuditEvent` structs | `capsem-logger/src/events.rs` |
| Logger writer | `WriteOp::ExecEvent`, `ExecEventComplete`, `AuditEvent` + insert/update functions | `capsem-logger/src/writer.rs` |
| Logger reader | `history()`, `history_processes()`, `history_counts()`, `recent_exec_events()`, `recent_audit_events()` | `capsem-logger/src/reader.rs` |
| PTY recording | `PtyLog` with direction/timestamp/length framing, 20MB rotation, 5 tests | `capsem-process/src/pty_log.rs` (new) |
| PTY wiring | Output recording in coalesce flush, input recording in TerminalInput handler | `capsem-process/src/vsock.rs` |
| Exec recording | `ExecEvent` on Exec dispatch, `ExecEventComplete` on ExecDone with timing | `capsem-process/src/vsock.rs` |
| Audit listener | vsock 5006 handler in connection loop, deserializes MessagePack, writes AuditEvent | `capsem-process/src/vsock.rs` |
| Guest agent | `audit_reader_loop()` tails auditd log, correlates multi-line records, streams MessagePack | `capsem-agent/src/main.rs` |
| Kernel config | `CONFIG_AUDIT=y`, `CONFIG_AUDITSYSCALL=y` | `guest/config/kernel/defconfig.{arm64,x86_64}` |
| Guest init | auditd startup, execve rules, immutable lock | `guest/artifacts/capsem-init` |
| Rootfs | `auditd` added to apt packages | `guest/config/packages/apt.toml` |
| Service API | `GET /history/{id}`, `/processes`, `/counts`, `/transcript` | `capsem-service/src/{main,api}.rs` |
| CLI | `capsem history <session>` with `--tail/--all/--search/--layer/--json` | `capsem/src/{main,client}.rs` |
| Session index | `exec_count`, `audit_event_count` columns, v6->v7 migration | `capsem-core/src/session/{index,types}.rs` |
| Changelog | Entry added | `CHANGELOG.md` |

### Remaining -- Implementation Order

```
7. Session rollup          (small, standalone)
6. Exec source attribution (small, standalone)
5. Export endpoint          (small, standalone)
1. VTE replay              (medium, unblocks 2+4)
2. Terminal reconnect       (small, needs 1)
3. Frontend HistoryView     (medium, standalone)
4. Frontend terminal reconnect (small, needs 2)
```

---

### Item 7: Session Rollup

**Goal**: Update `exec_count`/`audit_event_count` in main.db when a VM stops.

**Problem**: `handle_stop` does zero rollup. `handle_run` does network/MCP/file rollup but not exec/audit counts. The v6->v7 migration added the columns, but nothing populates them.

**Existing infrastructure**:
- `reader.history_counts()` returns `HistoryCounts { exec_count, audit_count }` (`capsem-logger/src/reader.rs:1195`)
- `SessionIndex` has `update_request_counts()` and `update_session_summary()` but no method for exec counts
- `state.main_db_path()` returns `~/.capsem/sessions/main.db` (`capsem-service/src/main.rs:236`)

**Changes**:

1. **`crates/capsem-core/src/session/index.rs`** -- add method after `update_request_counts` (line 234):

```rust
pub fn update_exec_counts(
    &self,
    id: &str,
    exec_count: u64,
    audit_event_count: u64,
) -> rusqlite::Result<()> {
    self.conn.execute(
        "UPDATE sessions SET exec_count = ?1, audit_event_count = ?2 WHERE id = ?3",
        params![exec_count as i64, audit_event_count as i64, id],
    )?;
    Ok(())
}
```

2. **`crates/capsem-service/src/main.rs`** -- add to `handle_run` rollup block (after line 1977, inside the `if let Ok(reader)` block):

```rust
if let Ok(hc) = reader.history_counts() {
    let _ = idx.update_exec_counts(&id, hc.exec_count, hc.audit_count);
}
```

3. **`crates/capsem-service/src/main.rs`** -- add rollup to `handle_stop` (line 1872, after process exit wait, before `if !persistent`):

```rust
// Roll up session counters for persistent VMs before cleanup.
{
    let session_db = session_dir.join("session.db");
    if session_db.exists() {
        let db_path = state.main_db_path();
        if let Ok(idx) = capsem_core::session::SessionIndex::open(&db_path) {
            if let Ok(reader) = capsem_logger::DbReader::open(&session_db) {
                if let Ok(counts) = reader.net_event_counts() {
                    let _ = idx.update_request_counts(
                        &id, counts.total as u64, counts.allowed as u64, counts.denied as u64,
                    );
                }
                if let Ok(hc) = reader.history_counts() {
                    let _ = idx.update_exec_counts(&id, hc.exec_count, hc.audit_count);
                }
                let file_events = reader.file_event_count().unwrap_or(0);
                let mcp_calls = reader.mcp_call_stats().map(|s| s.total).unwrap_or(0);
                let _ = idx.update_session_summary(&id, 0, 0, 0.0, 0, mcp_calls, file_events);
            }
            let _ = idx.update_status(&id, "stopped", Some(&capsem_core::session::now_iso()));
        }
    }
}
```

**Key detail**: `handle_stop` currently only has `(session_dir, persistent, pid)` from `shutdown_vm_process`. It also needs the `id` string, which it already has from the path param. The `session_dir` comes from `shutdown_vm_process` return. The `state.main_db_path()` is available via the handler's `State(state)`.

**Tests**: Unit test for `update_exec_counts` in `index.rs` tests. Integration: stop a persistent VM, verify main.db counts.

---

### Item 6: Exec Source Attribution

**Goal**: Thread `source`, `mcp_call_id`, `trace_id`, `process_name` through the IPC path so ExecEvent records who triggered each command.

**Problem**: In `capsem-process/src/vsock.rs:637`, ExecEvent is created with hardcoded `source: "api"` and all attribution fields as `None`.

**Current IPC message** (`capsem-proto/src/ipc.rs:14-15`):
```rust
Exec { id: u64, command: String },
```

**Current ExecEvent struct** (`capsem-logger/src/events.rs:216-228`):
```rust
pub struct ExecEvent {
    pub timestamp: SystemTime,
    pub exec_id: u64,
    pub command: String,
    pub source: String,           // already exists
    pub mcp_call_id: Option<u64>, // already exists
    pub trace_id: Option<String>, // already exists
    pub process_name: Option<String>, // already exists
}
```

**Current ExecRequest** (`capsem-service/src/api.rs:170-174`):
```rust
pub struct ExecRequest {
    pub command: String,
    pub timeout_secs: u64,
}
```

**Changes**:

1. **`crates/capsem-proto/src/ipc.rs:14-15`** -- extend Exec variant:

```rust
Exec {
    id: u64,
    command: String,
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    mcp_call_id: Option<u64>,
    #[serde(default)]
    trace_id: Option<String>,
    #[serde(default)]
    process_name: Option<String>,
},
```

`#[serde(default)]` ensures backward compatibility -- old messages without these fields deserialize as `None`.

2. **`crates/capsem-service/src/api.rs:170-174`** -- extend ExecRequest:

```rust
pub struct ExecRequest {
    pub command: String,
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mcp_call_id: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process_name: Option<String>,
}
```

3. **`crates/capsem-service/src/main.rs:1039`** -- `handle_exec`: thread attribution from payload:

```rust
let res = send_ipc_command(&uds_path, ServiceToProcess::Exec {
    id: id_val,
    command: payload.command,
    source: payload.source,
    mcp_call_id: payload.mcp_call_id,
    trace_id: payload.trace_id,
    process_name: payload.process_name,
}, payload.timeout_secs).await
```

4. **`crates/capsem-service/src/main.rs:2164`** -- `handle_run`: add `None`s:

```rust
ServiceToProcess::Exec {
    id: job_id,
    command: payload.command,
    source: None,
    mcp_call_id: None,
    trace_id: None,
    process_name: None,
},
```

5. **`crates/capsem-service/src/main.rs:701`** -- fsfreeze during fork: set internal source:

```rust
ServiceToProcess::Exec {
    id: freeze_id,
    command: "fsfreeze -f / 2>/dev/null; sync; fsfreeze -u / 2>/dev/null; true".to_string(),
    source: Some("internal".to_string()),
    mcp_call_id: None,
    trace_id: None,
    process_name: None,
},
```

6. **`crates/capsem-process/src/vsock.rs:629-644`** -- use IPC fields:

```rust
ServiceToProcess::Exec { id, command, source, mcp_call_id, trace_id, process_name } => {
    exec_times_cmd.lock().unwrap().insert(id, std::time::Instant::now());
    db_for_cmd.try_write(capsem_logger::WriteOp::ExecEvent(
        capsem_logger::ExecEvent {
            timestamp: std::time::SystemTime::now(),
            exec_id: id,
            command: command.clone(),
            source: source.unwrap_or_else(|| "api".to_string()),
            mcp_call_id,
            trace_id,
            process_name,
        },
    ));
    let _ = ctrl_cmd_tx.send(HostToGuest::Exec { id, command });
}
```

7. **`crates/capsem-mcp/src/main.rs:35-39`** -- add source to exec body:

```rust
fn build_exec_body(params: &ExecParams) -> Value {
    json!({
        "command": params.command,
        "timeout_secs": params.timeout.unwrap_or(30),
        "source": "mcp",
    })
}
```

**Note**: Full `mcp_call_id`/`trace_id` threading from MCP tool context is a follow-up. The MCP server doesn't currently have access to the MCP request context in tool handler functions. For now, `source: "mcp"` is the pragmatic first step.

**Tests**: Existing test `exec_params_roundtrip` in `capsem-mcp` will need `source` field check. Add test for `ExecRequest` deserialization with/without optional fields in `capsem-service/src/api.rs`. Verify `ServiceToProcess::Exec` serde roundtrip in `capsem-proto`.

---

### Item 5: `/history/{id}/export` Endpoint

**Goal**: Full audit export with process tree + BLAKE3 hash for corporate SIEM/compliance ingestion.

**Existing infrastructure**:
- `reader.history(limit, offset, search, layer)` returns `(Vec<HistoryEntry>, u64)` -- supports "exec" or "audit" layer filter
- `reader.history_processes(limit)` returns `Vec<ProcessEntry>`
- `reader.history_counts()` returns `HistoryCounts`
- `reader.recent_audit_events(limit)` returns `Vec<AuditEvent>` with full `pid`, `ppid`, `exe`, `parent_exe` fields
- `blake3` is already a dep of `capsem-core` (`blake3 = "1"`)
- Existing handlers follow pattern: `resolve_session_dir(&state, &id)` -> open `DbReader` -> query -> return typed response

**Changes**:

1. **`crates/capsem-service/src/api.rs`** -- add response types:

```rust
#[derive(Serialize, Debug)]
pub struct HistoryExportResponse {
    pub vm_id: String,
    pub export_timestamp: String,
    pub exec_events: Vec<capsem_logger::HistoryEntry>,
    pub audit_events: Vec<capsem_logger::HistoryEntry>,
    pub process_tree: serde_json::Value,
    pub summary: HistoryExportSummary,
    pub transcript_size_bytes: u64,
    pub transcript_blake3: Option<String>,
}

#[derive(Serialize, Debug)]
pub struct HistoryExportSummary {
    pub total_exec_events: u64,
    pub total_audit_events: u64,
    pub unique_executables: u64,
    pub top_executables: Vec<String>,
}
```

2. **`crates/capsem-service/src/main.rs`** -- add handler:

```rust
async fn handle_history_export(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
) -> Result<Json<api::HistoryExportResponse>, AppError> {
    let session_dir = resolve_session_dir(&state, &id)?;
    let db_path = session_dir.join("session.db");

    let reader = capsem_logger::DbReader::open(&db_path)
        .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("failed to open DB: {e}")))?;

    // Fetch all events (no pagination for export)
    let (exec_events, _) = reader.history(usize::MAX, 0, None, "exec")
        .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("exec query: {e}")))?;
    let (audit_events, _) = reader.history(usize::MAX, 0, None, "audit")
        .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("audit query: {e}")))?;

    // Build process tree from audit events
    let audit_raw = reader.recent_audit_events(usize::MAX)
        .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("audit raw: {e}")))?;
    let process_tree = build_process_tree(&audit_raw);

    // Summary
    let counts = reader.history_counts()
        .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("counts: {e}")))?;
    let processes = reader.history_processes(10)
        .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("processes: {e}")))?;
    let unique_exes = reader.history_processes(usize::MAX)
        .map(|p| p.len() as u64).unwrap_or(0);

    // PTY transcript hash
    let pty_log_path = session_dir.join("pty.log");
    let (transcript_size, transcript_hash) = if pty_log_path.exists() {
        let data = std::fs::read(&pty_log_path).unwrap_or_default();
        let hash = blake3::hash(&data);
        (data.len() as u64, Some(hash.to_hex().to_string()))
    } else {
        (0, None)
    };

    Ok(Json(api::HistoryExportResponse {
        vm_id: id,
        export_timestamp: capsem_core::session::now_iso(),
        exec_events,
        audit_events,
        process_tree,
        summary: api::HistoryExportSummary {
            total_exec_events: counts.exec_count,
            total_audit_events: counts.audit_count,
            unique_executables: unique_exes,
            top_executables: processes.iter().map(|p| p.exe.clone()).collect(),
        },
        transcript_size_bytes: transcript_size,
        transcript_blake3: transcript_hash,
    }))
}
```

**Process tree construction** (`build_process_tree` helper):

```rust
fn build_process_tree(events: &[capsem_logger::AuditEvent]) -> serde_json::Value {
    use std::collections::{HashMap, BTreeSet};
    let mut nodes: HashMap<u32, serde_json::Value> = HashMap::new();
    let mut children: HashMap<u32, BTreeSet<u32>> = HashMap::new();

    for event in events {
        nodes.entry(event.pid).or_insert_with(|| {
            json!({ "exe": event.exe, "ppid": event.ppid })
        });
        children.entry(event.ppid).or_default().insert(event.pid);
    }

    let mut tree = serde_json::Map::new();
    for (pid, node) in &nodes {
        let kids: Vec<u32> = children.get(pid).map(|s| s.iter().copied().collect()).unwrap_or_default();
        let mut entry = node.clone();
        entry["children"] = json!(kids);
        tree.insert(pid.to_string(), entry);
    }
    serde_json::Value::Object(tree)
}
```

3. **`crates/capsem-service/src/main.rs:2373`** -- add route alongside existing history routes:

```rust
.route("/history/{id}/export", get(handle_history_export))
```

4. **`crates/capsem-service/Cargo.toml`** -- add `blake3 = "1"` if not already present (it's in capsem-core but may need to be in capsem-service directly for the hash call).

**Tests**: Unit test for `build_process_tree` with sample audit events. Integration: verify export endpoint returns valid JSON with all fields populated.

---

### Item 1: VTE Replay Module

**Goal**: Reconstruct terminal screen state from PTY log bytes. Used by terminal reconnect (capsem-process) and transcript endpoint (capsem-service).

**Decision**: Use `vte` crate (parser only, 900 lines, zero deps) + hand-rolled screen buffer. NOT `alacritty_terminal` (15k+ lines, pulls in windowing/font deps).

**Location**: `capsem-core` (shared library), so both capsem-process and capsem-service can use it.

**Existing infrastructure**:
- `pty_log.rs` in capsem-process records frames as `[1b direction][8b timestamp_us LE][4b length LE][data]`
- `read_output_bytes(path)` extracts output-only data from pty.log
- PTY log rotates at 20MB, so worst-case replay is ~20MB of output
- `read_pty_log(path)` and `parse_pty_log(data)` parse the frame format -- these are `pub(crate)` in capsem-process, need equivalent in capsem-core

**Changes**:

1. **`crates/capsem-core/Cargo.toml`** -- add:
```toml
vte = "0.13"
```

2. **`crates/capsem-core/src/pty_log.rs`** (new) -- PTY log frame parser (shared version):

```rust
//! PTY log frame parser. Format:
//! [1 byte: direction (0x00=input, 0x01=output)]
//! [8 bytes: timestamp (microseconds since epoch, LE u64)]
//! [4 bytes: length (LE u32)]
//! [{length} bytes: raw terminal data]

pub const DIR_INPUT: u8 = 0x00;
pub const DIR_OUTPUT: u8 = 0x01;

/// Parse PTY log bytes into (direction, timestamp_us, data) entries.
pub fn parse_pty_log(data: &[u8]) -> Vec<(u8, u64, Vec<u8>)> {
    let mut entries = Vec::new();
    let mut pos = 0;
    while pos + 13 <= data.len() {
        let direction = data[pos];
        let timestamp_us = u64::from_le_bytes(data[pos + 1..pos + 9].try_into().unwrap());
        let len = u32::from_le_bytes(data[pos + 9..pos + 13].try_into().unwrap()) as usize;
        if pos + 13 + len > data.len() {
            break;
        }
        let payload = data[pos + 13..pos + 13 + len].to_vec();
        entries.push((direction, timestamp_us, payload));
        pos += 13 + len;
    }
    entries
}

/// Extract only output-direction bytes from PTY log data.
pub fn extract_output_bytes(data: &[u8]) -> Vec<u8> {
    let mut output = Vec::new();
    for (dir, _, payload) in parse_pty_log(data) {
        if dir == DIR_OUTPUT {
            output.extend_from_slice(&payload);
        }
    }
    output
}
```

3. **`crates/capsem-core/src/pty_replay.rs`** (new) -- VTE replayer:

**Screen buffer design**:
```rust
use std::collections::VecDeque;

struct Cell {
    ch: char,
    bold: bool,
    fg: Option<u8>,   // 0-255 (256-color)
    bg: Option<u8>,
}

impl Default for Cell {
    fn default() -> Self { Cell { ch: ' ', bold: false, fg: None, bg: None } }
}

pub struct PtyReplayer {
    grid: Vec<Vec<Cell>>,
    scrollback: VecDeque<Vec<Cell>>,
    cursor_row: usize,
    cursor_col: usize,
    cols: usize,
    rows: usize,
    // Current SGR attrs
    bold: bool,
    fg: Option<u8>,
    bg: Option<u8>,
    // Scroll region (1-indexed in VT100, 0-indexed here)
    scroll_top: usize,
    scroll_bottom: usize,
    // Saved cursor
    saved_cursor: Option<(usize, usize)>,
    // VTE parser
    parser: vte::Parser,
}
```

**VTE `Perform` implementation** (handle these sequences):
- `print(char)` -- write at cursor, advance, autowrap at EOL (push line to scrollback)
- `execute(byte)`:
  - `0x0D` CR -- cursor to column 0
  - `0x0A` LF -- cursor down, scroll if at bottom of scroll region
  - `0x08` BS -- cursor left (clamp at 0)
  - `0x09` HT -- advance to next tab stop (every 8 columns)
  - `0x07` BEL -- ignore
- `csi_dispatch(params, intermediates, ignore, action)`:
  - `'A'` CUU -- cursor up N
  - `'B'` CUD -- cursor down N
  - `'C'` CUF -- cursor right N
  - `'D'` CUB -- cursor left N
  - `'H'`/`'f'` CUP -- cursor position (row;col, 1-indexed)
  - `'J'` ED -- erase display (0=below, 1=above, 2=all)
  - `'K'` EL -- erase line (0=right, 1=left, 2=all)
  - `'m'` SGR -- set graphic rendition (0=reset, 1=bold, 22=unbold, 30-37/90-97=fg, 38;5;N=fg256, 39=default fg, 40-47/100-107=bg, 48;5;N=bg256, 49=default bg)
  - `'r'` DECSTBM -- set scroll region (top;bottom, 1-indexed)
  - `'h'`/`'l'` SM/RM -- mode set/reset (ignore most, but track cursor visibility for completeness)
  - `'L'` IL -- insert N blank lines at cursor
  - `'M'` DL -- delete N lines at cursor
- `esc_dispatch`:
  - `'M'` RI -- reverse index (cursor up, scroll down if at top of scroll region)
  - `'7'` DECSC -- save cursor position
  - `'8'` DECRC -- restore cursor position
- `osc_dispatch` -- ignore (title changes, color definitions)

**Public API**:
```rust
impl PtyReplayer {
    pub fn new(cols: usize, rows: usize) -> Self;

    /// Feed raw terminal output bytes through the VTE parser.
    pub fn feed(&mut self, data: &[u8]);

    /// Render the last N lines (scrollback + visible grid) as ANSI bytes.
    /// Output is raw bytes with SGR sequences that xterm.js can render directly.
    pub fn tail_bytes(&self, n_lines: usize) -> Vec<u8>;

    /// Resize the terminal grid. Existing content reflows.
    pub fn resize(&mut self, cols: usize, rows: usize);
}
```

**`tail_bytes()` implementation**:
- Collect the last N lines from scrollback + grid
- For each line, emit SGR sequences for each cell's attributes (only when they change from the previous cell)
- Emit the character
- At end of line, emit `\r\n`
- Reset SGR at the very end: `\x1b[0m`

**Note on `vte` crate API**: The `vte` crate uses a `Perform` trait. You create a `Parser`, then call `parser.advance(&mut performer, byte)` for each byte. The `PtyReplayer` struct owns the `Parser` and implements `Perform` on a separate helper struct (since `advance` takes `&mut self` on both parser and performer, you can't have them in the same struct). Pattern:

```rust
impl PtyReplayer {
    pub fn feed(&mut self, data: &[u8]) {
        let mut performer = Performer { screen: &mut self.screen_state };
        for byte in data {
            self.parser.advance(&mut performer, *byte);
        }
    }
}
```

4. **`crates/capsem-core/src/lib.rs`** -- add modules:
```rust
pub mod pty_log;
pub mod pty_replay;
```

**Tests** (in `pty_replay.rs`):
- `plain_text` -- feed "hello\r\nworld", verify grid content
- `cursor_movement` -- CUP, CUU/CUD/CUF/CUB sequences
- `screen_clear` -- ED(2), verify all cells are spaces
- `line_erase` -- EL(0), EL(1), EL(2)
- `sgr_color` -- feed colored text, verify `tail_bytes` emits correct SGR
- `scrolling` -- fill screen, add more lines, verify scrollback
- `scroll_region` -- DECSTBM, verify scrolling within region
- `tail_bytes_roundtrip` -- feed known ANSI, check `tail_bytes` output is valid

**Performance note**: Full replay of 20MB pty.log (worst case after rotation) at typical terminal throughput is <100ms on modern hardware. No snapshot optimization needed yet.

---

### Item 2: Terminal Reconnect Wiring

**Goal**: When a WebSocket client connects to a running VM's terminal, send reconstructed screen before live data.

**Current flow** (`capsem-process/src/terminal.rs:5-44`):
```rust
pub(crate) async fn handle_terminal_socket(
    ws: WebSocket,
    ctrl_tx: mpsc::Sender<ServiceToProcess>,
    mut term_rx: broadcast::Receiver<Vec<u8>>,
) {
    let (mut client_write, mut client_read) = ws.split();
    // rx_task: broadcast -> ws (guest output to client)
    // tx_task: ws -> ctrl_tx (client input to guest)
    // ... immediately starts forwarding
}
```

**Callsite** (`capsem-process/src/main.rs:416-425`):
```rust
let ws_app = axum::Router::new()
    .route("/terminal", axum::routing::get(
        move |ws: axum::extract::ws::WebSocketUpgrade| {
            let ctrl_tx = ctrl_tx_ws.clone();
            let term_rx = term_bcast_tx_app.subscribe();
            async move {
                ws.on_upgrade(move |socket| terminal::handle_terminal_socket(socket, ctrl_tx, term_rx))
            }
        }
    ));
```

`session_dir` is moved into the vsock spawn at line 368, but `args.session_dir` is still available at line 331. Need to clone it before the move.

**Changes**:

1. **`crates/capsem-process/src/main.rs`** -- clone session_dir for WS handler (before line 358):

```rust
let session_dir_ws = args.session_dir.clone();
```

Thread into the WS route closure:

```rust
let ws_app = axum::Router::new()
    .route("/terminal", axum::routing::get(
        move |ws: axum::extract::ws::WebSocketUpgrade| {
            let ctrl_tx = ctrl_tx_ws.clone();
            let term_rx = term_bcast_tx_app.subscribe();
            let sd = session_dir_ws.clone();
            async move {
                ws.on_upgrade(move |socket| terminal::handle_terminal_socket(socket, ctrl_tx, term_rx, sd))
            }
        }
    ));
```

2. **`crates/capsem-process/src/terminal.rs`** -- add replay before live:

```rust
pub(crate) async fn handle_terminal_socket(
    ws: axum::extract::ws::WebSocket,
    ctrl_tx: mpsc::Sender<ServiceToProcess>,
    mut term_rx: broadcast::Receiver<Vec<u8>>,
    session_dir: std::path::PathBuf,
) {
    let (mut client_write, mut client_read) = ws.split();

    // Replay: send reconstructed screen before live data
    let pty_log_path = session_dir.join("pty.log");
    if pty_log_path.exists() {
        if let Ok(raw) = std::fs::read(&pty_log_path) {
            let output = capsem_core::pty_log::extract_output_bytes(&raw);
            if !output.is_empty() {
                let mut replayer = capsem_core::pty_replay::PtyReplayer::new(80, 24);
                replayer.feed(&output);
                let replay_data = replayer.tail_bytes(500);
                if !replay_data.is_empty() {
                    let _ = client_write.send(
                        axum::extract::ws::Message::Binary(replay_data.into())
                    ).await;
                }
            }
        }
    }

    // Live forwarding (existing code)
    let mut rx_task = tokio::spawn(async move { ... });
    let mut tx_task = tokio::spawn(async move { ... });
    tokio::select! { ... }
}
```

**Edge cases**:
- No pty.log: skip replay (fresh VM, first connection)
- Empty pty.log: `output` will be empty, skip
- Rotation happened: only replay current `pty.log` (last 20MB max). Rotated `pty.log.1` is older data -- skip it.
- Default 80x24 terminal size: the first resize message from the client may not have arrived yet. The replay uses 80x24 as default. If the client later sends a different size, the live data will reflow naturally. This is acceptable for reconnect UX.

---

### Item 3: Frontend HistoryView

**Goal**: Command history UI as a new view tab alongside Terminal, Stats, Files.

**Existing patterns to follow**:
- `LogsView.svelte` -- polling, filtering, table layout
- `Toolbar.svelte:31-35` -- view button array
- `App.svelte:17` -- vmViews array, `App.svelte:108-121` -- view rendering
- `tabs.svelte.ts:1` -- TabView type union

**API endpoints** (all implemented):
- `GET /history/{id}?limit=500&offset=0&search=&layer=all` -> `{ commands: HistoryEntry[], total, has_more }`
- `GET /history/{id}/processes` -> `{ processes: ProcessEntry[] }`
- `GET /history/{id}/counts` -> `{ exec_count, audit_count }`

**HistoryEntry shape** (from `capsem-logger/src/reader.rs:160-171`):
```typescript
interface HistoryEntry {
  timestamp: string;
  layer: 'exec' | 'audit';
  command: string;
  exit_code: number | null;
  duration_ms: number | null;
  stdout_preview: string | null;
  stderr_preview: string | null;
  details: Record<string, any>;
  // For exec: details.source, details.process_name, details.trace_id, details.mcp_call_id
  // For audit: details.pid, details.ppid, details.exe, details.parent_exe, details.tty, details.cwd
}
```

**Changes**:

1. **`frontend/src/lib/stores/tabs.svelte.ts:1`** -- add 'history' to TabView:
```typescript
export type TabView = 'new-tab' | 'overview' | 'terminal' | 'stats' | 'files' | 'logs' | 'history' | 'inspector' | 'settings';
```

2. **`frontend/src/lib/components/shell/App.svelte:17`** -- add to vmViews:
```typescript
const vmViews = ['terminal', 'stats', 'logs', 'files', 'history', 'inspector'] as const;
```

3. **`frontend/src/lib/components/shell/App.svelte:~120`** -- add rendering branch:
```svelte
{:else if tab.view === 'history'}
  <div class="absolute inset-0"><HistoryView vmId={tab.vmId} /></div>
```

And import at top:
```typescript
import HistoryView from '../views/HistoryView.svelte';
```

4. **`frontend/src/lib/components/shell/Toolbar.svelte:31-35`** -- add history button:
```typescript
import ClockCounterClockwise from 'phosphor-svelte/lib/ClockCounterClockwise';

const vmViewButtons: { view: TabView; label: string; icon: typeof Terminal }[] = [
  { view: 'terminal', label: 'Terminal', icon: Terminal },
  { view: 'stats', label: 'Stats', icon: ChartBar },
  { view: 'files', label: 'Files', icon: FolderSimple },
  { view: 'history', label: 'History', icon: ClockCounterClockwise },
];
```

5. **`frontend/src/lib/api.ts`** -- add API functions:
```typescript
export interface HistoryEntry {
  timestamp: string;
  layer: string;
  command: string;
  exit_code: number | null;
  duration_ms: number | null;
  stdout_preview: string | null;
  stderr_preview: string | null;
  details: Record<string, any>;
}

export interface HistoryResponse {
  commands: HistoryEntry[];
  total: number;
  has_more: boolean;
}

export interface ProcessEntry {
  exe: string;
  command_count: number;
  first_seen: string;
  last_seen: string;
}

export interface HistoryCountsResponse {
  exec_count: number;
  audit_count: number;
}

export async function getHistory(id: string, params?: {
  limit?: number; offset?: number; search?: string; layer?: string;
}): Promise<HistoryResponse> {
  const qs = new URLSearchParams();
  if (params?.limit) qs.set('limit', String(params.limit));
  if (params?.offset) qs.set('offset', String(params.offset));
  if (params?.search) qs.set('search', params.search);
  if (params?.layer) qs.set('layer', params.layer);
  const resp = await _get(`/history/${id}?${qs}`);
  return resp.json();
}

export async function getHistoryProcesses(id: string): Promise<{ processes: ProcessEntry[] }> {
  const resp = await _get(`/history/${id}/processes`);
  return resp.json();
}

export async function getHistoryCounts(id: string): Promise<HistoryCountsResponse> {
  const resp = await _get(`/history/${id}/counts`);
  return resp.json();
}
```

6. **`frontend/src/lib/components/views/HistoryView.svelte`** (new) -- follow LogsView pattern:

```svelte
<script lang="ts">
  import { onMount, onDestroy } from 'svelte';
  import * as api from '../../api';
  import type { HistoryEntry, ProcessEntry } from '../../api';

  let { vmId }: { vmId: string } = $props();

  let entries = $state<HistoryEntry[]>([]);
  let total = $state(0);
  let processes = $state<ProcessEntry[]>([]);
  let pollInterval: ReturnType<typeof setInterval> | null = null;

  // Filters
  let layerFilter = $state<'all' | 'exec' | 'audit'>('all');
  let searchText = $state('');
  let processFilter = $state('');

  onMount(async () => {
    await Promise.all([fetchHistory(), fetchProcesses()]);
    pollInterval = setInterval(fetchHistory, 5000);
  });

  onDestroy(() => {
    if (pollInterval) clearInterval(pollInterval);
  });

  async function fetchHistory() {
    if (!api.isConnected()) return;
    try {
      const resp = await api.getHistory(vmId, {
        limit: 500,
        layer: layerFilter,
        search: searchText || undefined,
      });
      entries = resp.commands;
      total = resp.total;
    } catch { /* keep existing data */ }
  }

  async function fetchProcesses() {
    if (!api.isConnected()) return;
    try {
      const resp = await api.getHistoryProcesses(vmId);
      processes = resp.processes;
    } catch { /* keep existing data */ }
  }

  // Re-fetch when filters change
  $effect(() => {
    layerFilter; searchText;
    fetchHistory();
  });

  const filtered = $derived.by(() => {
    if (!processFilter) return entries;
    return entries.filter(e => {
      const exe = e.details?.exe ?? e.details?.process_name ?? '';
      return exe.includes(processFilter);
    });
  });

  function formatTimestamp(iso: string): string {
    if (!iso) return '';
    return new Date(iso).toLocaleTimeString(undefined, {
      hour12: false, hour: '2-digit', minute: '2-digit', second: '2-digit',
    });
  }

  function formatDuration(ms: number | null): string {
    if (ms == null) return '';
    if (ms < 1000) return `${ms}ms`;
    return `${(ms / 1000).toFixed(1)}s`;
  }
</script>

<!-- Filter bar: layer toggle, search, process filter, count -->
<!-- Table: timestamp, layer badge, command, exit code, duration, process -->
<!-- Empty state: "No command history" / "No commands match filters" -->
```

UI follows LogsView layout:
- Filter bar: `flex items-center gap-x-3 border-b border-line-2 bg-layer px-4 py-2`
- Layer toggle: `bg-background-1 rounded-lg p-0.5` with All/Exec/Audit buttons
- Search: `flex-1 text-sm border border-line-2 rounded-lg bg-layer text-foreground px-3 py-1`
- Process dropdown: `select` populated from `/history/{id}/processes`
- Table rows: timestamp (w-24), layer badge (w-16, exec=`bg-primary/10 text-primary`, audit=`bg-surface text-muted-foreground`), command, exit code, duration, process
- Count: `text-xs text-muted-foreground` showing `{filtered.length} of {total}`

---

### Item 4: Frontend Terminal Reconnect

**Goal**: Show reconstructed terminal history before live WebSocket attach, so reconnecting to a running VM shows recent output.

**Current flow**:
1. VMFrame.svelte gets iframe 'ready' message
2. Sends 'ws-ticket' with WebSocket URL to iframe
3. TerminalFrame.svelte connects WebSocket, clears terminal, sends resize+nudge

**Problem**: On reconnect, the terminal starts blank. The user loses all context from before the disconnection.

**Changes**:

1. **`crates/capsem-service/src/main.rs:1680-1704`** -- fix transcript endpoint to use VTE replay:

Current code reads raw binary pty.log and base64-encodes it (useless for xterm.js). Replace with:

```rust
async fn handle_history_transcript(
    State(state): State<Arc<ServiceState>>,
    Path(id): Path<String>,
    Query(params): Query<api::TranscriptQuery>,
) -> Result<Json<api::TranscriptResponse>, AppError> {
    use base64::Engine;
    let session_dir = resolve_session_dir(&state, &id)?;
    let pty_log_path = session_dir.join("pty.log");

    if !pty_log_path.exists() {
        return Ok(Json(api::TranscriptResponse {
            content: String::new(),
            bytes: 0,
        }));
    }

    let raw = std::fs::read(&pty_log_path)
        .map_err(|e| AppError(StatusCode::INTERNAL_SERVER_ERROR, format!("read pty.log: {e}")))?;

    let output = capsem_core::pty_log::extract_output_bytes(&raw);
    if output.is_empty() {
        return Ok(Json(api::TranscriptResponse { content: String::new(), bytes: 0 }));
    }

    let mut replayer = capsem_core::pty_replay::PtyReplayer::new(80, 24);
    replayer.feed(&output);
    let rendered = replayer.tail_bytes(params.tail_lines);

    let encoded = base64::engine::general_purpose::STANDARD.encode(&rendered);
    Ok(Json(api::TranscriptResponse {
        bytes: rendered.len(),
        content: encoded,
    }))
}
```

2. **`frontend/src/lib/terminal/postmessage.ts`** -- add transcript-replay message:

```typescript
export interface MsgTranscriptReplay {
  type: 'transcript-replay';
  data: string; // base64-encoded ANSI bytes
}

export type ParentToIframeMsg =
  | MsgVmId
  | MsgThemeChange
  | MsgFocus
  | MsgWsTicket
  | MsgClipboardPaste
  | MsgTranscriptReplay;
```

Add to `parseParentMessage`:
```typescript
case 'transcript-replay': {
  if (typeof msg.data !== 'string') return null;
  if (msg.data.length > 10_000_000) return null; // 10MB max
  return { type: 'transcript-replay', data: msg.data };
}
```

3. **`frontend/src/lib/api.ts`** -- add transcript fetch:

```typescript
export async function getTranscript(id: string, tailLines?: number): Promise<{ content: string; bytes: number }> {
  const qs = tailLines ? `?tail_lines=${tailLines}` : '';
  const resp = await _get(`/history/${id}/transcript${qs}`);
  return resp.json();
}
```

4. **`frontend/src/lib/components/shell/VMFrame.svelte`** -- fetch transcript before sending ws-ticket:

In the `case 'ready':` handler (line 51-66), after sending vm-id and theme, before sending ws-ticket:

```typescript
case 'ready':
  sendToIframe({ type: 'vm-id', vmId });
  sendToIframe({
    type: 'theme-change',
    mode: themeStore.mode,
    terminalTheme: themeStore.resolvedTerminalTheme,
    fontSize: themeStore.fontSize,
    fontFamily: themeStore.fontFamily,
  });
  // Fetch and send transcript replay before WebSocket
  if (gatewayStore.connected) {
    try {
      const transcript = await api.getTranscript(vmId, 500);
      if (transcript.content && transcript.bytes > 0) {
        sendToIframe({ type: 'transcript-replay', data: transcript.content });
      }
    } catch { /* proceed without replay */ }
    const wsUrl = api.getTerminalWsUrl(vmId);
    const base = api.getBaseUrl();
    sendToIframe({ type: 'ws-ticket', ticket: wsUrl, gatewayUrl: base });
  }
  break;
```

5. **`frontend/src/lib/components/terminal/TerminalFrame.svelte`** -- handle transcript-replay:

In `onMessage` switch (line 49-65):

```typescript
case 'transcript-replay': {
  if (terminal && msg.data) {
    // Decode base64 and write to terminal
    const binary = atob(msg.data);
    const bytes = new Uint8Array(binary.length);
    for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
    terminal.write(bytes);
  }
  break;
}
```

**Flow on reconnect**:
1. VMFrame sends 'vm-id' + 'theme-change' to iframe
2. VMFrame fetches `/history/{vmId}/transcript?tail_lines=500` (VTE-rendered ANSI bytes, base64)
3. VMFrame sends 'transcript-replay' with base64 data to iframe
4. TerminalFrame decodes and writes to xterm.js (shows reconstructed screen)
5. VMFrame sends 'ws-ticket' to iframe
6. TerminalFrame connects WebSocket, sends resize (does NOT clear terminal this time -- the replay data should remain)

**Important**: In `connectWebSocket` (TerminalFrame.svelte:98-112), the `terminal.clear()` call at line 102 needs to be conditional -- skip it if transcript replay was already written. Add a flag:

```typescript
let hasReplayData = false;

// In transcript-replay handler:
hasReplayData = true;

// In connectWebSocket socket.onopen:
socket.onopen = () => {
  wsConnected = true;
  reconnectAttempt = 0;
  if (terminal) {
    if (!hasReplayData) {
      terminal.clear();
    }
    hasReplayData = false; // reset for next reconnect cycle
    // ... existing resize + nudge
  }
};
```

---

### Pre-existing issues found (not this sprint)

- `capsem-service/src/main.rs`: `supervise_companion` signature changed but callers had stale arg count (fixed by user during sprint)
- `capsem/src/main.rs`: `generate_systemd_unit` test has stale arg count (blocks `cargo test -p capsem` test binary compilation, non-test binary compiles fine)
- `capsem-core/src/mcp/gateway.rs`: 9 pre-existing test failures in MCP gateway tests (unrelated to this sprint)

---

## Verification

### Per-item testing
- **Items 7, 6, 5**: `cargo test -p capsem-core -p capsem-service -p capsem-proto -p capsem-mcp-server`
- **Item 1**: `cargo test -p capsem-core` (VTE replay unit tests)
- **Item 2**: Manual: `capsem shell`, run commands, disconnect, reconnect -- should see prior output
- **Items 3, 4**: `pnpm run check` in `frontend/`, visual verification via dev server

### Integration
- `just smoke` -- full integration test suite
- Boot VM, run commands, check `/history/{id}` API returns data, verify `capsem history` CLI
- Stop persistent VM, verify main.db has `exec_count`/`audit_event_count` populated
- Verify `/history/{id}/export` returns process tree + BLAKE3 hash
- Open frontend, switch to History tab, verify table with search/filters works
- Disconnect terminal, reconnect, verify prior output appears before live data
