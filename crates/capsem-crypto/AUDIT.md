# Cryptographic ownership and integrity contracts

The audit covers Git-owned Rust workspace sources and manifests, Python build
and guest tooling, SDKs, MCP, shell/image/workflow scripts and web sources.
Dependencies, generated clients and vendored browser code are provider users
owned upstream; they are not first-party primitive implementations.

| Contract | Producer and consumer | Owner and algorithm |
|---|---|---|
| OCI manifests, configs, layers and registry addresses | Registry, assets pull/cache, guest launcher | Rust AWS-LC SHA-256; portable guest OpenSSL SHA-256 |
| Prepared EROFS v2 | Image producer, subject-bound pinned artifact, assets cache, core staging, guest held-descriptor mount | Python SIMD BLAKE3 producer; `capsem-crypto::verify` on host and guest |
| Release packages and VM asset manifests | Admin/build/release tooling, installer/updater/assets | Explicit existing SHA-256 and BLAKE3 fields; owned Rust/Python bindings |
| Build, source and cache keys | Builder/gate and foundation | Existing BLAKE3, SHA-256 and configured BLAKE2 identities retained |
| Archive blocks, deduplicated stored spans, exported bodies | Archive/logger readers and writers | Owned BLAKE3; storage corruption and wrong-span checks stay with storage owners; no event admission commitments |
| Credential fingerprints and session capabilities | Protocol, credentials, service, gateway, core | Owned BLAKE3 derivation, OS entropy, constant-time checks and erasure |
| IDs and UUID-derived private secrets | Archive, logger, core, service | `capsem-crypto::uuid_v4`, including test/build callers |
| OAuth PKCE and ID tokens | Credential owner | AWS-LC SHA-256 and restricted RS256 JWT verification; issuer, audience, expiry, nonce and JWKS admission remain credential policy |
| TLS provider, CA/leaf keys and certificates | Core MITM and mock server | Crypto-owned rustls AWS-LC provider and certificate issuer; SNI/cache/admission policy remains in core |
| WebSocket acceptance | Mock upstream and portable guest diagnostics | Protocol-specific SHA-1 only; no generic SHA-1 content verification |
| SDK/package acceptance | Node tooling, standalone Python SDK proof and guest commands | Owned Node native SHA-256/UUID bindings; independent Python/OpenSSL reference hashes |
| Platform package signing and notarization | Build/release rails | Native codesign/notarytool and packaging tools, under the release contract |

The Rust guard discovers workspace members rather than maintaining a caller
list. It checks normal, target, build and dev dependencies and source aliases,
raw provider paths, FFI and UUID generation. Only the crypto crate selects
cryptographic providers. Hash states and the provider crate are optimized in
development too; the hot-build guard prevents a debug-profile regression.

Python build and image callers use `capsem_builder.crypto`. Its native
bindings avoid a host executable dependency. The one stdlib-only bootstrap
retains SHA-256 until source-keyed re-exec. Installed/guest shell and Python
fragments retain the native SHA tools available in their execution environment;
their upstream and release identities remain explicit. Independent Python test
hashes are references rather than a production provider selection. Node
package/acceptance tools share `sdk/typescript/tools/crypto.mjs`; shipped
browser SDK and MCP source contain no first-party crypto operations. Python
and Node import guards protect these tooling boundaries.

Prepared EROFS deliberately changes its private format to v2 without a v1
compatibility path. Its pinned manifest binds the BLAKE3 annotation and
platform subject. A download checks transport SHA-256 and BLAKE3 in the same
pass. A cache hit checks BLAKE3 and exact size. The launcher opens a regular,
no-follow descriptor, passes that descriptor to `capsem-sysutil verify-blob`,
rewinds it and holds it through mounting. Verification rejects corrupt,
truncated, enlarged and malformed content and reads at most one excess byte.

The normal `capsem-bench` crypto dimension records raw paired samples for
accelerated SHA-256, an independent SHA reference, SIMD BLAKE2b-256,
BLAKE2s-256 and BLAKE3, with a read-only control. The provider's example owns
the algorithm loop; the existing benchmark binary owns statistics and storage.
An optional, pinned `xxhash-rust` dependency (BSL-1.0, no runtime dependencies)
extends this example with XXH3-128 one-shot comparisons for non-security uses.
It is enabled only by `benchmark`, is not exported as a production API, and
is optimized alongside BLAKE3 in debug measurements. Existing body hashes stay
on the approved accelerated BLAKE3 owner. No production algorithm changes are
implied by comparing throughput.
There is no separate benchmark crate. BLAKE3 was the fastest bulk candidate on
the measured host and guest. Warm file hash timings are not VM boot timings.

Focused qualification includes standard vectors, chunk boundaries, reader
errors, corruption/size/subject cases, entropy shape, JWT signature corruption,
certificate trust and adversarial source guards. The native BLAKE3 diagnostic
boots retain 12 GiB/4 CPUs and original deadlines. Complete phase, installed
package and Apple qualification remain separate sprint gates; a partial guest
binary diagnostic is not release proof.
