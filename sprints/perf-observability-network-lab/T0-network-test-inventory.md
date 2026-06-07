# T0: Network Test Replacement Inventory

Default release gates must be deterministic. Public network coverage is allowed
only as explicit smoke, not as the benchmark/correctness path.

## Classification

| Classification | Meaning |
| --- | --- |
| Local-lab replacement | Must move to the deterministic debug upstream or local fixture before this sprint closes. |
| Explicit smoke | May remain, but must be opt-in/skipped unless a smoke flag or provider credential is present. |
| Keep local | Already local/deterministic; no replacement required. |
| Obsolete | Remove once covered by a better local-lab or security-event proof. |

## Inventory

| Surface | Current file/test | Public dependency today | Classification | Replacement target |
| --- | --- | --- | --- | --- |
| Guest DNS allowed resolution | `guest/artifacts/diagnostics/test_network.py::test_dns_resolves_via_capsem_proxy` | `elie.net` | Local-lab replacement | Local DNS fixture backed by debug upstream domain, plus one opt-in public DNS smoke. |
| Guest DNS NXDOMAIN | `guest/artifacts/diagnostics/test_network.py::test_dns_nxdomain_propagates_from_upstream` | `.invalid` reserved TLD | Keep local | Keep: deterministic reserved-domain behavior, no internet dependency. |
| Guest TLS handshake | `guest/artifacts/diagnostics/test_network.py::test_tls_handshake_completes` | `google.com` | Local-lab replacement | Local HTTPS upstream or deterministic host-side SNI/cert fixture. |
| Guest MITM cert inspection | `guest/artifacts/diagnostics/test_network.py::test_tls_cert_from_capsem_ca` | `google.com` | Local-lab replacement | Local HTTPS/SNI fixture; still validates Capsem CA chain. |
| Guest HTTPS request | `guest/artifacts/diagnostics/test_network.py::test_curl_https_with_skip_verify` | `google.com` | Local-lab replacement | `GET /tiny` through local debug upstream. |
| Guest verbose curl diagnostic | `guest/artifacts/diagnostics/test_network.py::test_curl_verbose_diagnostics` | `google.com` | Explicit smoke | Keep as opt-in diagnostic only; local-lab handles release proof. |
| Guest trusted CA curl | `guest/artifacts/diagnostics/test_network.py::test_curl_allowed_domain_ca_trusted` | `google.com` | Local-lab replacement | Local HTTPS upstream with Capsem MITM CA trust. |
| Guest trusted Python TLS | `guest/artifacts/diagnostics/test_network.py::test_python_urllib_https_trusted` | `google.com` | Local-lab replacement | Local HTTPS upstream with Python default context. |
| Denied random domain | `guest/artifacts/diagnostics/test_network.py::test_denied_domain_rejected` | `.invalid` reserved TLD | Keep local | Keep with deterministic denied-domain fixture. |
| Denied POST | `guest/artifacts/diagnostics/test_network.py::test_post_to_random_domain_denied` | `example.com` | Local-lab replacement | Local `/deny-target` with block rule. |
| Provider domain policy | `guest/artifacts/diagnostics/test_network.py::test_ai_provider_domain_blocked` | `api.anthropic.com`, `api.openai.com` | Explicit smoke | Keep opt-in/provider-smoke only; default provider-rule proof uses local model fixtures and session DB. |
| Plain HTTP proxy | `guest/artifacts/diagnostics/test_network.py::test_http_port_80_is_proxied` | `google.com` | Local-lab replacement | Local plain HTTP `GET /tiny`. |
| Non-standard HTTPS port | `guest/artifacts/diagnostics/test_network.py::test_non_standard_port_fails` | `google.com:8443` | Local-lab replacement | Local host port fixture or pure route/iptables assertion. |
| Direct IP blocked | `guest/artifacts/diagnostics/test_network.py::test_direct_ip_no_route` | `1.1.1.1` | Local-lab replacement | Local unrouted/private IP fixture or pure route assertion. |
| Proxy throughput | `guest/artifacts/diagnostics/test_network.py::test_proxy_download_throughput` | `cdn.elie.net` PDF | Local-lab replacement | Local `/bytes/10mb` and `/gzip/1mb` benchmark cases. |
| Sandbox network smoke | `guest/artifacts/diagnostics/test_sandbox.py` network section | `elie.net`, `example.com` | Local-lab replacement | Reuse local DNS/TLS/HTTP cases; keep one public smoke opt-in. |
| MCP builtin positive HTTP | `guest/artifacts/diagnostics/test_mcp.py` fetch/grep/header positive tests | `elie.net` | Local-lab replacement | Local debug upstream pages with deterministic body/header text. |
| MCP builtin blocked HTTP | `guest/artifacts/diagnostics/test_mcp.py` blocked-domain tests | fake blocked domains | Keep local | Keep, but align expected rows with security-rule ledger. |
| AI CLI provider smoke | `guest/artifacts/diagnostics/test_ai_cli.py::test_google_ai_domain_allowed` | `generativelanguage.googleapis.com` | Explicit smoke | Skip unless provider credential/smoke flag is set. |
| Integration script network rows | `scripts/integration_test.py` | `google.com`, `example.com`, `cdn.elie.net` | Local-lab replacement | Local `GET /tiny`, denied `/deny-target`, and `/bytes/10mb`; public smoke split out. |
| Session net event test | `tests/capsem-session/test_net_events.py` | `elie.net` | Local-lab replacement | Local-lab curl that asserts `net_events`/`security_rule_events`. |
| Session exhaustive fixture | `tests/capsem-session-exhaustive/conftest.py` | `elie.net` | Local-lab replacement | Local-lab fixture row or recorded deterministic DB fixture. |
| `capsem-bench http` default | `guest/artifacts/capsem_bench/helpers.py`, `http_bench.py` | `https://www.google.com/` | Local-lab replacement | New `capsem-bench mitm-local` tiny/1MB/gzip cases; old `http URL N C` remains manual. |
| `capsem-bench throughput` default | `guest/artifacts/capsem_bench/throughput.py` | `cdn.elie.net` PDF | Local-lab replacement | Local `/bytes/10mb`; public throughput only opt-in. |
| `capsem-bench mitm-load` default | `guest/artifacts/capsem_bench/mitm_load.py` | nonexistent public domain | Obsolete | Replace with local denied/upstream-error targets so DNS/upstream variance disappears. |
| `capsem-bench dns-load` default | `guest/artifacts/capsem_bench/dns_load.py` | `api.openai.com` blocked path by default | Local-lab replacement | Local blocked qname and local allowed qname fixture; public upstream resolver path opt-in. |
| `capsem-bench mcp-load` default | `guest/artifacts/capsem_bench/mcp_load.py` | none; local MCP echo | Keep local | Keep; add security-event/DB queue labels when T2/T3 land. |
| Gateway tests | `tests/capsem-gateway/*` | local test services | Keep local | No replacement required. |
| Install asset download tests | `tests/capsem-install/test_asset_download.py` | local `http.server` | Keep local | Good pattern for T1 debug upstream lifecycle helper. |
| Policy V2 HTTP/DNS MITM tests | `tests/capsem-e2e/test_policy_v2_http_dns_mitm.py` | mostly local fixtures with `example.com` policy names | Keep local | Keep; replace any real upstream calls with debug upstream when found. |
| Model policy MITM tests | `tests/capsem-e2e/test_model_policy_mitm.py` | OpenAI endpoint URL shape | Local-lab replacement | Local model-like SSE/OpenAI-compatible fixture; real provider smoke opt-in. |
| Brokered AI credential E2E | `tests/capsem-e2e/test_brokered_ai_credentials.py` | Anthropic URL shape | Local-lab replacement | Local credential response/capture fixture for default gate; real provider smoke opt-in. |

## Replacement Order

1. Build the local debug upstream and lifecycle helper.
2. Add `capsem-bench mitm-local` using `/tiny`, `/bytes/1mb`, `/gzip/1mb`,
   `/sse/model`, `/deny-target`, `/credential/response`, and WebSocket cases.
3. Move `capsem-bench all` away from public HTTP/throughput defaults.
4. Replace guest diagnostics public HTTP/TLS/DNS proof with local lab.
5. Replace MCP builtin positive fetch/grep/header tests with local deterministic
   pages.
6. Split real provider/public-network tests behind explicit smoke flags.
7. Query `session.db` for every local-lab case and assert security-event rows,
   rule rows, and no raw secret leakage.

## Open Questions For Implementation

- Whether T1 local HTTPS should use a tiny Rust TLS server or terminate TLS in
  the host MITM while upstream stays plain HTTP. Either is acceptable if the
  Capsem guest-facing TLS trust path remains covered.
- Whether local DNS should be served by the debug upstream binary or by a small
  process-side DNS fixture. The requirement is deterministic qname -> response
  with no public resolver in the default path.
