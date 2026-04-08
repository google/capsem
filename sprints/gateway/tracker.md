# TCP Gateway Sprint

New `capsem-gateway` crate: standalone process that bridges TCP to capsem-service UDS with shared-secret auth. Serves the frontend web app and system tray.

Worktree: worktrees/capsem-gateway (branch: tcp-gateway)
Crate: crates/capsem-gateway/

## Architecture

capsem-gateway is a low-privilege reverse proxy. It has no VM access, no vsock, no elevated privileges. Follows the Chrome multi-process isolation model already used for capsem-process.

```
Browser / Tauri / System Tray
  |
  |-- GET/POST http://127.0.0.1:19222/...
  |-- Authorization: Bearer <token>
  |
  v
capsem-gateway (TCP listener, auth middleware, CORS)
  |
  |-- HTTP/1.1 over UDS (~/.capsem/run/service.sock)
  |-- No auth (filesystem permissions sufficient for UDS)
  |
  v
capsem-service (high privilege, VM lifecycle)
  |
  v
capsem-process (per-VM) -> Guest VM (vsock 5000-5003)
```

### Why separate process (not baked into capsem-service)

- capsem-service is high-privilege (spawns VMs, manages vsock). Adding TCP + network exposure increases its attack surface.
- Gateway is low-privilege: reads UDS, writes TCP. Properly sandboxed.
- Tray team can modify/extend the gateway without touching the service.
- Clean separation of concerns: service manages VMs, gateway manages access.

### Token Flow

1. Gateway starts, generates 64-char random token
2. Writes token to `~/.capsem/run/gateway.token` (chmod 600)
3. Writes TCP port to `~/.capsem/run/gateway.port`
4. All TCP requests require `Authorization: Bearer <token>` header
5. Token regenerated on each restart (not persistent across restarts)
6. Clean shutdown deletes token + port + pid files

### Endpoints

All capsem-service endpoints are proxied through, plus gateway-native additions:

| Endpoint | Source | Purpose |
|----------|--------|---------|
| `GET /` | Gateway-native | Health check (no auth required, for liveness probes) |
| `GET /status` | Gateway-native | Aggregated system health for tray |
| `GET /list` | Proxied | VM list |
| `POST /provision` | Proxied | Create VM |
| `GET /info/{id}` | Proxied | VM details |
| `POST /exec/{id}` | Proxied | Run command |
| `POST /read_file/{id}` | Proxied | Read file |
| `POST /write_file/{id}` | Proxied | Write file |
| `POST /stop/{id}` | Proxied | Stop VM |
| `DELETE /delete/{id}` | Proxied | Delete VM |
| `GET /logs/{id}` | Proxied | VM logs |
| `POST /inspect/{id}` | Proxied | SQL query |
| `POST /resume/{name}` | Proxied | Resume persistent VM |
| `POST /persist/{id}` | Proxied | Make VM persistent |
| `POST /purge` | Proxied | Clean up VMs |
| `POST /run` | Proxied | Provision + wait for ready |
| `POST /fork/{id}` | Proxied | Snapshot VM to image |
| `GET /images` | Proxied | List images |
| `GET /images/{name}` | Proxied | Image details |
| `DELETE /images/{name}` | Proxied | Delete image |
| `POST /reload-config` | Proxied | Reload network/MCP config |
| `WS /terminal/{id}` | Gateway-native (SS7) | WebSocket terminal bridge via per-process UDS |

## Sub-sprints

### SS1: Scaffold

Status: Done

- [x] Create crate: `crates/capsem-gateway/Cargo.toml`
- [x] Dependencies: axum, hyper, hyper-util, tokio, tower, tower-http (trace, cors), rand, serde, serde_json, clap, tracing, tracing-subscriber, anyhow
- [x] CLI args: `--port` (default 19222), `--uds-path` (default `~/.capsem/run/service.sock`), `--foreground`
- [x] Basic main.rs: parse args, init tracing, placeholder server that returns 200 on `/`
- [x] Add `capsem-gateway` to workspace `Cargo.toml` members list
- [x] Verify: `cargo build -p capsem-gateway` succeeds, binary starts and responds to curl

### SS2: Auth System

Status: Done

- [x] `auth.rs`: `generate_token()` -- 64-char alphanumeric random string using `rand`
- [x] Write token to `~/.capsem/run/gateway.token` with chmod 600
- [x] Write port to `~/.capsem/run/gateway.port`
- [x] Bearer token validation as Axum middleware layer
- [x] Reject requests without valid token -> 401 Unauthorized JSON response
- [x] Exempt `/` health endpoint from auth (liveness probe)
- [x] Token file cleanup on SIGTERM/SIGINT (tokio::signal handler)
- [x] Verify: `curl http://127.0.0.1:19222/list` -> 401; `curl -H "Authorization: Bearer $(cat ~/.capsem/run/gateway.token)" http://127.0.0.1:19222/list` -> proxied response (or 502 if service not running)

```rust
// auth.rs sketch
pub fn generate_token() -> String {
    use rand::Rng;
    rand::rng()
        .sample_iter(&rand::distr::Alphanumeric)
        .take(64)
        .map(char::from)
        .collect()
}

pub async fn auth_middleware(
    State(token): State<Arc<String>>,
    req: Request,
    next: Next,
) -> Response {
    // Skip auth for health check
    if req.uri().path() == "/" {
        return next.run(req).await;
    }

    let valid = req.headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .is_some_and(|t| t == token.as_str());

    if valid {
        next.run(req).await
    } else {
        (StatusCode::UNAUTHORIZED, Json(json!({"error": "unauthorized"}))).into_response()
    }
}
```

### SS3: UDS Proxy

Status: Done

Forward all HTTP requests to capsem-service over UDS. Use the same `hyper` HTTP/1.1-over-UnixStream pattern the CLI uses.

- [x] `proxy.rs`: `forward()` function that sends an HTTP request over UDS
- [x] Use `hyper::client::conn::http1::handshake` over `tokio::net::UnixStream` (same as `crates/capsem/src/main.rs:422-467`)
- [x] Catch-all Axum fallback route: any method, any path -> forward to UDS
- [x] Preserve: HTTP method, URI path, query string, Content-Type header, request body
- [x] Return: service response status code, Content-Type header, response body
- [x] Timeout: 30s per request (prevent hung connections)
- [x] Error handling: if UDS connect fails -> 502 Bad Gateway `{"error": "service unavailable"}`
- [x] Verify: `GET /list` through gateway returns same JSON as `curl --unix-socket ~/.capsem/run/service.sock http://localhost/list`

```rust
// proxy.rs sketch -- same UDS pattern as CLI (crates/capsem/src/main.rs:422-467)
pub async fn forward(
    uds_path: &Path,
    method: Method,
    uri: &str,
    body: Option<Bytes>,
) -> Result<Response> {
    let stream = UnixStream::connect(uds_path).await?;
    let io = TokioIo::new(stream);
    let (mut sender, conn) = hyper::client::conn::http1::handshake(io).await?;
    tokio::spawn(conn);

    let mut builder = Request::builder()
        .method(method)
        .uri(format!("http://localhost{}", uri));
    if body.is_some() {
        builder = builder.header("content-type", "application/json");
    }
    let req = builder.body(Full::new(body.unwrap_or_default()))?;
    let res = sender.send_request(req).await?;
    Ok(res.into_response())
}
```

### SS4: /status Endpoint

Status: Done

Aggregated system health designed for the tray to poll efficiently.

- [x] `status.rs`: handler for `GET /status`
- [x] Calls `GET /list` on service over UDS
- [x] Enriches with `GET /info/{id}` for each VM
- [ ] Response shape:

```json
{
  "service": "running",
  "gateway_version": "0.1.0",
  "vm_count": 3,
  "vms": [
    { "id": "abc123", "name": "dev", "status": "running", "persistent": true },
    { "id": "def456", "name": null, "status": "running", "persistent": false }
  ],
  "resource_summary": {
    "total_ram_mb": 6144,
    "total_cpus": 6,
    "running_count": 2,
    "stopped_count": 1
  }
}
```

- [x] Cache: 2-second TTL (`tokio::sync::RwLock` + `Instant`) to avoid hammering service
- [x] Graceful degradation: if service UDS unreachable, return `{ "service": "unavailable", "vm_count": 0, "vms": [], "resource_summary": null }`
- [x] Verify: `GET /status` returns valid JSON with VM summary

### SS5: CORS + Browser Support

Status: Done

Browser fetch needs CORS headers or requests fail.

- [x] `CorsLayer::permissive()` from `tower-http` on the TCP router
- [x] OPTIONS preflight requests handled automatically by the layer
- [x] All error responses as JSON `{ "error": "message" }` (not plain text)
- [x] Content-Type: application/json on all responses
- [x] Verify: browser `fetch()` from `localhost:5173` to `gateway:19222` succeeds (no CORS error in devtools)

### SS6: Lifecycle + Integration

Status: Done

Clean startup, shutdown, and discoverability.

- [x] Startup: check if `service.sock` exists, log warning if not (don't block -- service may start later)
- [x] Log at startup: gateway version, bound port, token file path, UDS target path
- [x] Graceful shutdown on SIGTERM/SIGINT: close TCP listener, delete `gateway.token`, `gateway.port`, `gateway.pid`
- [x] PID file: `~/.capsem/run/gateway.pid` (write on start, delete on shutdown)
- [x] Health check: `GET /` returns `{ "ok": true, "version": "0.1.0" }` with no auth required (for liveness probes and service discovery)
- [x] Verify: start gateway, curl `/status`, kill gateway with SIGTERM, confirm token file deleted

### SS7: WebSocket Terminal Streaming

Status: Not started

Browser-accessible terminal via WebSocket, using Option C architecture: per-VM process exposes its own HTTP/WS endpoint on a dedicated UDS, gateway proxies the WebSocket upgrade to it. This keeps the gateway dumb (no capsem-proto, no IPC knowledge) and limits blast radius to one VM per process.

**Architecture (Option C -- chosen for security):**

```
Browser (xterm.js)
  |
  |-- WS ws://127.0.0.1:19222/terminal/{id}
  |-- Authorization: Bearer <token>
  |
  v
capsem-gateway (WebSocket proxy, auth only)
  |
  |-- WS over UDS (~/.capsem/run/instances/{id}-ws.sock)
  |-- No auth (filesystem permissions sufficient for UDS)
  |
  v
capsem-process (per-VM, WS-to-PTY bridge)
  |
  |-- Already has vsock:5001 terminal stream
  |-- Already has vsock:5000 control channel for resize
  |
  v
capsem-agent -> PTY -> bash
```

**Why Option C (process-level WS) over alternatives:**
- A) Gateway connects to process IPC directly -- gateway needs capsem-proto, gains VM access, larger blast radius
- B) Service adds WS endpoint -- adds network surface to high-privilege service
- C) Process exposes WS -- gateway stays dumb proxy, blast radius = 1 VM, service stays isolated

**Gateway side:**

- [ ] `terminal.rs`: WebSocket proxy handler for `WS /terminal/{id}`
- [ ] Accept WebSocket upgrade using axum's `WebSocketUpgrade` extractor
- [ ] Look up per-VM WS socket path: `~/.capsem/run/instances/{id}-ws.sock`
- [ ] Connect to process WS UDS, perform HTTP upgrade over UDS
- [ ] Bidirectional frame forwarding: browser WS <-> process WS
- [ ] Spawn two tasks: client-to-process and process-to-client
- [ ] Forward binary frames (terminal data) and text frames (resize JSON)
- [ ] Clean shutdown: close both sides when either disconnects
- [ ] Verify: connect from browser, type commands, see output

**Process side (capsem-process changes):**

- [ ] Add HTTP/WS listener on `~/.capsem/run/instances/{id}-ws.sock`
- [ ] Accept WebSocket upgrades on `WS /terminal`
- [ ] Bridge WS frames to existing terminal infrastructure:
  - Binary frames from client -> write to vsock:5001 (same as `ServiceToProcess::TerminalInput`)
  - Terminal output from `TerminalOutputQueue` -> binary frames to client
  - Text frame `{"type":"resize","cols":N,"rows":N}` -> `ioctl(TIOCSWINSZ)` via vsock:5000
- [ ] Send `StartTerminalStream` equivalent internally when WS connects
- [ ] Handle multiple concurrent WS connections (multiple browser tabs)
- [ ] Dependency additions: axum (for WS), tokio-tungstenite or axum built-in WS

**Protocol:**

| Direction | Frame Type | Content |
|-----------|-----------|---------|
| Client -> Process | Binary | Raw terminal input bytes |
| Client -> Process | Text | `{"type":"resize","cols":80,"rows":24}` |
| Process -> Client | Binary | Raw terminal output bytes |

**Dependencies:**
- Gateway: no new deps (axum already has WebSocket support)
- Process: needs axum or lightweight WS library for the per-process HTTP server

## Crate Structure

```
crates/capsem-gateway/
  Cargo.toml
  src/
    main.rs          # CLI args (clap), startup, signal handling, dual concerns
    auth.rs          # Token generation, file write/cleanup, Bearer middleware
    proxy.rs         # UDS forwarding (hyper HTTP/1.1 over UnixStream)
    status.rs        # GET /status aggregation + caching
    terminal.rs      # WS /terminal/{id} proxy to per-process WS UDS
```

## Acceptance Criteria (Sprint Gate)

- [x] `capsem-gateway` binary builds cleanly (`cargo build -p capsem-gateway`)
- [x] Starts in <100ms, logs port and token path
- [x] Token written to `gateway.token` with 0o600 permissions
- [x] Unauthenticated requests -> 401 JSON
- [x] `GET /` -> 200 (no auth required, health check)
- [x] All service endpoints reachable through gateway with valid token
- [x] `GET /status` returns aggregated VM health
- [x] CORS headers present on all responses
- [x] Clean shutdown (SIGTERM) deletes token + port + pid files
- [x] Integration test: provision, list, exec, stop cycle through gateway with a running capsem-service
- [ ] `WS /terminal/{id}` connects browser to guest PTY through gateway -> process WS UDS
- [ ] Terminal resize from browser propagates to guest PTY
- [ ] Terminal session closes cleanly on browser disconnect

## Test Coverage (SS1-SS6)

| Tier | Count | Scope |
|------|-------|-------|
| Rust unit | 58 | auth (22), proxy (21), status (15), health (2) |
| Python integration (mock UDS) | 32 | health (3), auth (7), proxy (8), status (4), runtime (6), CORS (3) |
| Python E2E (real service + VM) | 5 | lifecycle, /status, 404, race regression, health |
| **Total** | **95** | |

Security: 10MB body limit (413), auth bypass edge cases (8 vectors), path traversal, header filtering.

## Depends On

- capsem-service running (for integration testing only; gateway starts without it)
- Service API types reference: `crates/capsem-service/src/api.rs`
- CLI UDS client pattern reference: `crates/capsem/src/main.rs:422-467`
- Terminal data path: CLI shell handler `crates/capsem/src/main.rs:516-642`
- Per-VM process IPC: `crates/capsem-process/src/main.rs:999-1140` (handle_ipc_connection)
- Terminal output queue: `crates/capsem-core/src/vm/terminal.rs:16-112`
- IPC protocol: `crates/capsem-proto/src/ipc.rs` (ServiceToProcess/ProcessToService)
- Guest agent bridge: `crates/capsem-agent/src/main.rs:401-603` (bridge_loop)

## Blocks

- System tray sprint `sprints/tray/tracker.md` (needs gateway for HTTP access to service)
- UI wiring sprint (needs gateway for frontend-to-service communication)
- `WS /terminal/{id}` enables browser-based terminal (SS7, required by tray and frontend)

## Requirements from Tray Team

- Tray polls `GET /status` every 5s (SS4 must return VM list with status)
- Tray reads `gateway.token` + `gateway.port` from `~/.capsem/run/` (SS2)
- Tray hot-reloads token on gateway restart (re-reads files on 401/connection refused)

## Reference

- Service router: `crates/capsem-service/src/main.rs:1370-1398`
- CLI UDS client: `crates/capsem/src/main.rs:422-467`
- vsock ports: `crates/capsem-core/src/vm/vsock.rs` (5000=control, 5001=terminal, 5002=MITM, 5003=MCP)
- Rust patterns: `skills/dev-rust-patterns/SKILL.md`
- Testing policy: `skills/dev-testing/SKILL.md`
