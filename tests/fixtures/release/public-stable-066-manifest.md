# Original public 0.6.6 metadata

`public-stable-066-manifest.json` contains the exact 804,737 response bytes from
`https://release.capsem.org/assets/stable/manifest.json`, captured 2026-10-24
under Sprinty S08-404. BLAKE3:
`2b04fb53657ee3146c66054c7bd4177248eec6ba21bb0e51f33482ebb4b966c8`.

The actual public Mac descriptor agrees with the GitHub `v0.6.6` release API:
source `f9c4ff3af881d86b50a518a7da3d77825ac4ffd0`, published
2026-10-06T17:25:20Z, package 60,150,446 bytes, SHA-256
`bfb0ae4eb5c13bfdbf768476113f113a304f915c8add3813a74fbc1ca511d300`.
No package payload is in this fixture. Unit archive payloads are explicitly
synthetic and do not claim installation, public package verification or VM proof.

The scoped native upgrade owner must use the real public package/SBOM and this
original profile graph, including original absolute artifact URLs. Its initial
install and explicit original-client feed selection may download those original
profile inputs through the existing supported setup network boundary. This is
untimed setup, not Apply latency. A network refusal/missing original artifact
must fail setup; do not rewrite the graph or substitute modern runtime bytes.
