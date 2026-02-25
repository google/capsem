# Final Implementation Plan: Zero-Trust Capsem Architecture

### Phase 1: Network & Ephemeral Workspace (M5)

**Goal:** Sever the VM from the internet and eliminate persistent guest storage.

* **Networking:**
* Enable the IP stack in the guest kernel (`CONFIG_INET=y`, `CONFIG_IP_NF_IPTABLES=y`).
* Create a `dummy0` interface (`10.0.0.2/24`).
* Configure a fake DNS server on `127.0.0.1:53` that resolves all queries to `10.0.0.1`.
* Use `iptables REDIRECT` to force all ports (8080, 8081, 443) into the host's Vsock proxies.


* **Storage (The Burn-After-Reading Sandbox):**
* **Remove VirtioFS completely.**
* The `/workspace` directory is mounted strictly as a RAM-backed `tmpfs`.
* If the VM is killed, all generated files, logs, and state are instantly and permanently destroyed unless explicitly extracted by the host.



### Phase 2: The RPC Command & Control Channel (M5/M6)

**Goal:** Replace static boot configs with a hyper-fast, real-time MessagePack C2 pipe over `vsock:5000`.

* **The Disjoint Type System:**
* Define two strictly separate Rust enums: `HostToGuest` (Commands) and `GuestToHost` (Telemetry/Responses).


* **The Hardcheck (Host Vsock Loop):**
* The host reads from `vsock:5000` using a static `[0u8; 8192]` stack buffer to completely neutralize OOM DoS attacks.
* The host parses bytes *strictly* into the `GuestToHost` enum using `rmp-serde`.
* **Kill Switch:** If the deserializer fails (e.g., the guest attempts to send a `HostToGuest` command), the host instantly drops the socket and sends `SIGKILL` to the VM.


* **File Telemetry:** The in-guest daemon streams file events (`FileCreated`, `FileEdited`) over this vsock pipe in chunked payloads to prevent buffer overflows.

### Phase 3: The Active API Gateway (M6)

**Goal:** Intercept and evaluate all LLM API traffic at the host level.

* **Host Proxy (`vsock:5004`):** Catches agent API calls, strips dummy keys, and injects real API keys from the macOS Keychain.
* **The 9-Stage Event Lifecycle:**
* The Gateway evaluates every step (`on_model_call`, `on_tool_call`, etc.) against the corporate `policy.toml`.
* **PII Engine:** Scans outgoing prompts and replaces secrets with `[REDACTED]` tokens.
* **Tool Intercept:** Pauses LLM `tool_use` streams. Prompts the user via the Tauri UI. If denied, injects a synthetic failure back to the LLM.



### Phase 4: Hybrid MCP Execution (M7)

**Goal:** Keep local tools in the sandbox; route enterprise tools through the host.

* **Local MCPs:** Tools requiring local binaries (`bash`, `npm`) execute natively inside the VM via `stdio`. The host controls them via the Stage 7 `on_tool_call` intercept.
* **Remote MCPs:** Enterprise tools (Jira, GitHub) are rewritten during boot. The host Gateway (`vsock:5003`) proxies these requests, injecting corporate auth headers and routing them over the host's VPN.

### Phase 5: Telemetry, Audit & Compress State (M8)

**Goal:** Eliminate SQLite locking issues and minimize disk footprint.

* **Per-Session Databases:** Every agent run gets its own isolated directory and SQLite file (e.g., `~/.capsem/sessions/sess_123/audit.db`).
* **Zstd Compression:** Raw MessagePack telemetry and LLM payloads are compressed using the `zstd` crate *before* being inserted into SQLite `BLOB` columns.
* **Observability:** Expose a `127.0.0.1:9090/metrics` endpoint (Prometheus) and support OTLP exporting for corporate SIEM integration.

---

# New Document: `docs/architecture.md`

**Purpose:** This document serves as the "Map of the Territory" for onboarding new developers. It explains the physical layout of the system and how data flows between the host and the guest.

**Required Sections:**

1. **System Overview & Philosophy:** Explain the Zero-Trust, "Burn-After-Reading" architecture. Clarify why the VM is treated as hostile.
2. **Crate Structure:**
* `capsem-core` (Host Daemon): Contains the API Gateway, Vsock Master, Policy Engine, and SQLite managers.
* `capsem-rpc-agent` (Guest Daemon): The PID 1 process inside the VM. Contains the Vsock Slave and `fanotify` filesystem watcher.
* `capsem-shared`: Contains the strictly disjoint `HostToGuest` and `GuestToHost` MessagePack enums.


3. **Data Flow Diagrams (Mermaid.js):**
* *The Boot Sequence:* Host spawning the VM -> Vsock Handshake -> `SetEnv` Injection.
* *The API Intercept:* Agent -> Vsock 5004 -> API Gateway -> PII Scrubber -> LLM.


4. **Vsock Port Map:** A clear table defining ports 5000 (RPC), 5002 (SNI), 5003 (MCP), and 5004 (API).
5. **Memory & Security Bounds:** Document the 8KB static buffer rule and the strict `rmp-serde` hardcheck.

---

# Updates for `docs/security.md`

You must update the Threat Model section to explicitly outline the new mitigations we just built.

**T3: Data Exfiltration (Updated)**

* *Old Mitigation:* API Key isolation.
* *New Mitigation:* Vsock SNI Proxy enforces a strict domain allowlist. Fake DNS and `iptables` drop all outbound traffic not explicitly routed through host-inspected vsock ports. The API Gateway utilizes a real-time PII redaction engine to tokenize secrets before they reach the LLM provider.

**T14 (NEW): Vsock C2 Reversal & Privilege Escalation**

* *Threat:* A compromised guest agent attempts to send commands back over the Vsock RPC channel to execute code on the host Mac.
* *Mitigation:* **Disjoint Type-Level Allowlist.** The RPC protocol uses physically separate enums for Host and Guest vocabularies. The host's deserialization loop will structurally fail to parse guest-originated commands, triggering an immediate `SIGKILL` of the VM.

**T15 (NEW): Vsock Resource Exhaustion (OOM DoS)**

* *Threat:* A compromised guest blasts a continuous, multi-gigabyte stream of garbage data over vsock to crash the host daemon via Out-Of-Memory (OOM).
* *Mitigation:* **Static Stack Buffers.** The host reads vsock streams into a statically allocated `[0u8; 8192]` buffer. MessagePack payloads are length-capped and processed in chunks. The host will throttle or drop connections that exceed defined memory boundaries.

**T16 (NEW): Persistent Malware Infection**

* *Threat:* The AI agent downloads or writes a malicious binary to the filesystem, hoping to execute it on subsequent runs.
* *Mitigation:* **Burn-After-Reading Tmpfs.** VirtioFS has been completely removed for workspace storage. The guest filesystem is exclusively backed by RAM (`tmpfs`). All state is cryptographically erased from memory upon VM termination.

---

Would you like me to draft the `mermaid.js` sequence diagram for the `docs/architecture.md` file so you can drop the code block directly into the new doc?