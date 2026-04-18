---
title: Network Isolation
description: Air-gapped networking, iptables rules, and the MITM proxy.
sidebar:
  order: 20
---

The guest VM has no real network interface. All outbound HTTPS traffic flows through a host-side MITM proxy that enforces domain and HTTP-level policy, terminates TLS, and logs every request to the session database.

## Air-gapped architecture

```mermaid
graph LR
    subgraph "Guest VM"
        APP["Application (curl, pip, npm)"]
        DNS["dnsmasq<br/>all domains -> 10.0.0.1"]
        IPT["iptables REDIRECT<br/>:443 -> :10443"]
        NP["capsem-net-proxy<br/>TCP:10443"]
    end

    subgraph "Host"
        MITM["MITM Proxy<br/>TLS termination + policy"]
        UP["Upstream server"]
    end

    APP -->|DNS query| DNS
    APP -->|HTTPS :443| IPT
    IPT -->|TCP :10443| NP
    NP -->|vsock:5002| MITM
    MITM -->|TLS| UP
```

No packets leave the VM through a NIC. The only path to the internet is vsock port 5002, which the host MITM proxy controls.

## Guest network setup

`capsem-init` builds the air-gapped network stack during boot:

| Step | Command | Purpose |
|------|---------|---------|
| 1. Loopback | `ip link set lo up` | Enable localhost |
| 2. Dummy NIC | `ip link add dummy0 type dummy` | Create fake interface |
| 3. Assign IP | `ip addr add 10.0.0.1/24 dev dummy0` | Give it a local address |
| 4. Default route | `ip route add default dev dummy0` | All traffic routes to dummy0 |
| 5. Fake DNS | `dnsmasq --address=/#/10.0.0.1` | All domains resolve to 10.0.0.1 |
| 6. iptables | `iptables -t nat -A OUTPUT -p tcp --dport 443 -j REDIRECT --to-port 10443` | Redirect HTTPS to proxy |
| 7. Net proxy | `capsem-net-proxy` | TCP:10443 to vsock:5002 bridge |

The result: when an application resolves `github.com`, it gets `10.0.0.1`. When it connects to `10.0.0.1:443`, iptables redirects to `127.0.0.1:10443`. `capsem-net-proxy` bridges the TCP connection to the host over vsock port 5002.

## MITM proxy overview

The host MITM proxy receives each connection on vsock:5002 and runs a full inspection pipeline:

```mermaid
graph TD
    A["vsock:5002 connection"] --> B["TLS ClientHello<br/>extract SNI domain"]
    B --> C{"Domain policy<br/>check"}
    C -->|Denied| D["Return 403<br/>log to session.db"]
    C -->|Allowed| E["Complete TLS handshake<br/>mint leaf cert for domain"]
    E --> F["Parse HTTP request<br/>method + path + headers"]
    F --> G{"HTTP policy<br/>check"}
    G -->|Denied| H["Return 403<br/>log to session.db"]
    G -->|Allowed| I["Forward to upstream<br/>real TLS connection"]
    I --> J["Stream response<br/>to guest"]
    J --> K["Log telemetry<br/>domain, method, path, status, bytes, latency"]
```

The proxy mints per-domain TLS certificates signed by a static Capsem CA (ECDSA P-256, 24-hour validity). The CA is baked into the guest rootfs and trusted by the system certificate store, Python certifi, and Node.js. See [MITM Proxy Architecture](/architecture/mitm-proxy/) for implementation details.

### CA trust chain

| Component | How it trusts the CA |
|-----------|---------------------|
| System store | `/usr/local/share/ca-certificates/capsem-ca.crt` + `update-ca-certificates` |
| Python certifi | Patched bundle includes Capsem CA |
| Node.js | `NODE_EXTRA_CA_CERTS` env var |
| curl/wget | `SSL_CERT_FILE` env var |
| pip/requests | `REQUESTS_CA_BUNDLE` env var |

## Domain policy

The domain policy engine uses block-before-allow semantics with a default-deny fallback.

### Evaluation order

```mermaid
graph TD
    A["Domain received"] --> B{"In block list?"}
    B -->|Yes| C["DENY<br/>'domain in block-list'"]
    B -->|No| D{"In allow list?"}
    D -->|Yes| E["ALLOW<br/>'domain in allow-list'"]
    D -->|No| F["DENY<br/>'domain not in allow-list'"]
```

Block list is checked first. If a domain appears in both lists, block wins.

### Pattern matching

| Pattern | Example | Matches | Does not match |
|---------|---------|---------|----------------|
| Exact | `github.com` | `github.com` | `api.github.com` |
| Wildcard | `*.github.com` | `api.github.com`, `raw.github.com` | `github.com` (base domain) |

Matching is case-insensitive. Wildcard patterns require at least one subdomain label before the suffix.

### Default allow list

| Domain | Purpose |
|--------|---------|
| `github.com`, `*.github.com` | Git hosting, API |
| `*.githubusercontent.com` | GitHub raw content |
| `registry.npmjs.org`, `*.npmjs.org` | npm packages |
| `pypi.org`, `files.pythonhosted.org` | Python packages |
| `crates.io`, `static.crates.io` | Rust packages |
| `deb.debian.org`, `security.debian.org` | Debian packages |
| `*.googleapis.com` | Google APIs |
| `en.wikipedia.org`, `*.wikipedia.org` | Reference |

### Default block list

| Domain | Reason |
|--------|--------|
| `api.anthropic.com` | AI provider -- forced through audit gateway |
| `api.openai.com` | AI provider -- forced through audit gateway |

### User configuration

Users can customize policy in `~/.capsem/user.toml`:

```toml
[network]
custom_allow = ["internal.corp.com", "*.example.org"]
custom_block = ["malware.bad.com"]
```

Corporate policy in `/etc/capsem/corp.toml` overrides user settings entirely per field.

## HTTP policy

For allowed domains, an optional HTTP-level policy checks method and path:

| Field | Description | Example |
|-------|-------------|---------|
| `domain` | Exact domain match | `github.com` |
| `method` | HTTP method or `*` for any | `POST`, `GET`, `*` |
| `path_pattern` | Exact path or prefix wildcard | `/repos/*`, `/api/v1/users` |
| `action` | Allow or Deny | `Deny` |

### Evaluation stages

| Stage | Check | Short-circuit |
|-------|-------|---------------|
| 1. Domain | Block/allow list + default | If denied, return immediately |
| 2. HTTP rules | Method + path matching for the domain | First matching rule wins |
| 3. Fallback | No matching HTTP rule for allowed domain | Allow (backward compat) |

### Path matching

| Pattern | Matches | Does not match |
|---------|---------|----------------|
| `/api/v1/users` | Exact: `/api/v1/users` | `/api/v1/users/123` |
| `/api/v1/*` | `/api/v1/users`, `/api/v1/repos/foo/bar` | `/api/v2/users` |
| `*` | Any path | -- |

## Telemetry

Every proxied request is logged to the per-VM `session.db`:

| Column | Content |
|--------|---------|
| `domain` | Target domain |
| `method` | HTTP method |
| `path` | Request path |
| `status_code` | Upstream response status |
| `decision` | `allowed`, `denied`, or `error` |
| `bytes_sent` | Request body size |
| `bytes_received` | Response body size |
| `duration_ms` | End-to-end latency |
| `request_body_preview` | First 4 KB of request body |
| `response_body_preview` | First 4 KB of response body |
| `matched_rule` | Which policy rule matched |

For AI provider traffic (Anthropic, OpenAI, Google), the proxy also parses SSE streams to extract model calls, token usage, tool calls, and estimated cost. See [Session Telemetry](/architecture/session-telemetry/) for the full schema.

## What gets blocked

| Scenario | Outcome | Why |
|----------|---------|-----|
| HTTPS to unlisted domain (`example.com`) | 403 Forbidden | Default deny; domain not in allow list |
| HTTPS to blocked domain (`api.openai.com`) | 403 Forbidden | Explicit block list |
| HTTP port 80 (`http://google.com`) | Connection refused | Only port 443 is redirected |
| Non-standard port (`https://google.com:8443`) | Connection refused | Only port 443 is redirected |
| Direct IP (`https://1.1.1.1`) | Connection refused | No real NIC; dummy0 has no real route |
| POST to allowed domain with deny rule | 403 Forbidden | HTTP-level rule blocks the method |

## capsem-doctor validation

Network isolation is validated by `test_network.py` across 7 layers. Tests are ordered low-to-high so failures pinpoint the exact broken layer.

| Layer | Tests | What it validates |
|-------|-------|-------------------|
| **L1: Guest plumbing** | `test_dummy0_has_ip`, `test_dnsmasq_responds`, `test_dns_all_resolve_to_local`, `test_iptables_redirect_443_to_10443` | dummy0 has 10.0.0.1, DNS resolves all domains to 10.0.0.1, iptables REDIRECT rule present |
| **L2: Net proxy** | `test_net_proxy_listening`, `test_tcp_443_reaches_proxy`, `test_vsock_bridge_delivers_bytes` | capsem-net-proxy accepts TCP on :10443, iptables redirect works, bytes flow through vsock bridge |
| **L3: TLS handshake** | `test_tls_handshake_completes`, `test_tls_cert_from_capsem_ca` | Full TLS to allowed domain succeeds, MITM proxy presents Capsem CA cert |
| **L4: HTTP over MITM** | `test_curl_https_with_skip_verify`, `test_curl_verbose_diagnostics` | curl -k gets HTTP response, full handshake trace captured |
| **L5: CA trust** | `test_mitm_ca_cert_file_exists`, `test_mitm_ca_in_system_bundle`, `test_certifi_includes_capsem_ca`, `test_curl_allowed_domain_ca_trusted`, `test_python_urllib_https_trusted`, `test_ca_env_var_set` | CA cert file exists, in system bundle, in Python certifi, curl works without -k, Python TLS works, `SSL_CERT_FILE`/`REQUESTS_CA_BUNDLE`/`NODE_EXTRA_CA_CERTS` set |
| **L6: Policy enforcement** | `test_denied_domain_rejected`, `test_post_to_random_domain_denied`, `test_ai_provider_domain_blocked`, `test_http_port_80_not_proxied`, `test_non_standard_port_fails`, `test_direct_ip_no_route` | Denied domains get 403, port 80 fails, non-443 ports fail, direct IP fails |
| **L7: Throughput** | `test_proxy_download_throughput` | 100 MB download through MITM meets minimum speed threshold |

Additional network tests in `test_sandbox.py`:

| Test | Property |
|------|----------|
| `test_dummy_interface_exists` | dummy0 interface present |
| `test_dns_resolves_to_local` | DNS returns 10.0.0.1 |
| `test_iptables_redirect` | REDIRECT rule active |
| `test_net_proxy_running` | capsem-net-proxy process alive |
| `test_no_real_nics` | Only `lo` and `dummy0` in `/sys/class/net/` |
| `test_allowed_domain` | End-to-end HTTPS to allowed domain (5-step diagnostic) |
| `test_denied_domain` | HTTPS to denied domain returns 403 or refused |
