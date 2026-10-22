# Capsem 0.7.1 gateway chain: sandbox boundary assessment

## Reproduction status

On an installed macOS 0.7.1 instance, `ollama_gateway_escape.py` asked Ollama
to select `capsem__expose_port` inside a disposable guest. The guest MCP relay
published its HTTP page on a host loopback port. When that page was opened in a
host browser, it reported `TOKEN_STATUS=200`, `TOKEN_LENGTH=64`, and
`SETTINGS_STATUS=200`. The page did not print the bearer token.

A second run used two simultaneously running, otherwise unconnected 2 GiB
VMs. A marker written only in VM B's `/root/capsem-peer-marker` was absent in
VM A (`test ! -e` exited 0). Ollama in VM A again selected the guest exposure.
The published page then reported `PEER_VM_FILE_STATUS=200` and
`PEER_VM_FILE_BODY=VM_B_ISOLATION_MARKER` for VM B's file route. This proves
the gateway bearer chain crossed the VM ownership boundary after the browser
opened VM A's page; it does not imply that the VMs shared a private network or
that VM A directly opened VM B's filesystem.

The separate request for `/Users/elie/leakme.txt` through
`/vms/{id}/files/content` returned `HOST_FILE_ROUTE_STATUS=400` with an
outside-workspace error. **These runs proved gateway bearer disclosure, an
authenticated settings read, and a cross-VM workspace read; they did not prove
an arbitrary home-file read.**
Keep the latter claim open until a payload reads the harmless marker and its
output is captured without printing other secrets. The launcher and guest
script are in this directory.

## Patched gateway regression

The same Ollama-driven guest page was pointed at an isolated gateway built
from this worktree on an OS-assigned port. In the browser it reported
`STOPPED=Failed to fetch`, before a token or peer file request could complete.
An unauthenticated direct `GET /token` returned 401, and a preflight request
with the guest page's `Origin` received no `Access-Control-Allow-Origin`.
This verifies the patched gateway blocks the demonstrated browser chain. The
isolated patched service had no VMs of its own; the two live VMs belonged to
the installed 0.7.1 service, and the script's `--gateway-port` selected which
gateway the guest page tested.

## Where each boundary applies

| Component | OS restriction | Why it did not stop the observed chain |
| --- | --- | --- |
| Guest and MCP relay | Guest VM isolation; native MCP workers have descriptor-only host authority | `capsem__expose_port` was an allowed product operation. The worker passed a publication request through its granted channel; it did not open a host file or socket directly. |
| Host browser | The browser's own sandbox, outside Capsem's worker policy | The user opened the published loopback page. Its JavaScript sent ordinary HTTP requests to the gateway, and 0.7.1's CORS policy allowed that loopback origin. |
| Gateway on macOS | Seatbelt denies ambient file access and new outbound connections; the gateway has a prebound listener, a token-file grant, and an inherited control channel | `GET /token` returned the token already held in gateway memory. The authenticated settings and VM B file handlers used the gateway's existing control channel. No forbidden syscall was needed. |
| Gateway on Linux | Landlock limits filesystem paths; seccomp limits syscalls | Those controls likewise cannot decide whether an allowed HTTP handler should disclose its token or accept a particular browser origin. This reproduction ran on macOS; the Linux conclusion is from the shared request path and Linux worker policy, not a Linux live exploit run. |

The gateway's startup attestation checks that ambient `/etc/passwd` reads,
direct TCP/UDS connections, and process execution are denied. That is useful
confinement, but it does not revoke the gateway's in-memory token or distinguish a
trusted Tauri request from a guest-published browser origin. The credential
broker issue in the review has the same shape: a permitted broker operation
mapped an OpenAI key to an Ollama endpoint. Landlock, seccomp, and Seatbelt
limit process capabilities; the provider binding and endpoint authorization
must be enforced in application code.

## Application controls in this branch

The fixes remove `/token`, restrict browser origins to the app and fixed
gateway origin, allocate host ports for guest exposures, and refuse OpenAI
credential redemption for Ollama endpoints. The installed 0.7.1 reproduction
above predates those fixes. The isolated patched gateway regression covers the
browser chain; the final combined branch still needs installed-package
qualification, including the separate Linux KVM cross-VM vsock probe.
