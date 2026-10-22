# Capsem 0.7.1 gateway chain: sandbox boundary assessment

## Reproduction status

On an installed macOS 0.7.1 instance, `ollama_gateway_escape.py` asked Ollama
to select `capsem__expose_port` inside a disposable guest. The guest MCP relay
published its HTTP page on a host loopback port. When that page was opened in a
host browser, it reported `TOKEN_STATUS=200`, `TOKEN_LENGTH=64`, and
`SETTINGS_STATUS=200`. The page did not print the bearer token.

The separate request for `/Users/elie/leakme.txt` through
`/vms/{id}/files/content` returned `HOST_FILE_ROUTE_STATUS=400` with an
outside-workspace error. **This run proved gateway bearer disclosure and an
authenticated settings read; it did not prove an arbitrary home-file read.**
Keep the latter claim open until a payload reads the harmless marker and its
output is captured without printing other secrets. The launcher and guest
script are in this directory.

## Where each boundary applies

| Component | OS restriction | Why it did not stop the observed chain |
| --- | --- | --- |
| Guest and MCP relay | Guest VM isolation; native MCP workers have descriptor-only host authority | `capsem__expose_port` was an allowed product operation. The worker passed a publication request through its granted channel; it did not open a host file or socket directly. |
| Host browser | The browser's own sandbox, outside Capsem's worker policy | The user opened the published loopback page. Its JavaScript sent ordinary HTTP requests to the gateway, and 0.7.1's CORS policy allowed that loopback origin. |
| Gateway on macOS | Seatbelt denies ambient file access and new outbound connections; the gateway has a prebound listener, a token-file grant, and an inherited control channel | `GET /token` returned the token already held in gateway memory. The authenticated settings handler used its existing control channel. No forbidden syscall was needed. |
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
above predates those fixes; it must not be used as proof that the patched build
passes. The final combined branch still needs installed-package qualification.
