# capsem-crypto

This crate owns first-party Rust cryptographic providers. Domain crates retain
their protocol identities, admission policy, certificate cache and trust rules.
They call this crate instead of selecting a hash or TLS backend themselves.

| Feature | API | Contract |
|---|---|---|
| always | `blake3::{hash, Hash, Hasher}`, `verify` | BLAKE3 identities, constant-time hash equality and bounded reader verification |
| always | `xxh3::{hash, Hasher}` | Allocation-free one-shot and incremental XXH3-128 for non-security integrity, deduplication and fingerprints |
| `sha256` (default) | `Sha256` | Streaming and one-shot SHA-256 through CPU-dispatched AWS-LC |
| `sha256` | `websocket_accept_digest` | SHA-1 only for the RFC 6455 handshake |
| `random` | `random`, `uuid_v4` | OS-seeded tokens, UUID identifiers, ranges and fallible entropy |
| `secrets` | `secrets` | Audited constant-time comparison and secret erasure |
| `tls` | `tls` | Rustls provider, private key ownership and certificate minting |
| `jwt` | `jwt` | RS256 public verification; signing and arbitrary key conversion refused |
| `test-fixtures` | `jwt::sign_rs256_fixture` | Test-only signature construction from checked-in fixture keys |
| `benchmark` | `compare_hashes` example | Optional SIMD BLAKE2 and independent SHA reference, compared with the production BLAKE3 and XXH3 owners |

Foundation, protocol and archive callers disable default features to retain
their lightweight BLAKE3 and XXH3 dependency surface. The optional BLAKE2 and
independent SHA benchmark providers do not enter production binaries. The crypto crate and hot hash backends stay optimized in
dev builds, enforced by Citadel.

Hashing requires a documented contract; do not add fingerprints by default.
The incremental XXH3 state implements `std::io::Write`, so persisted metadata
can stream its actual codec bytes into the owner without a temporary document.
Choose crypto-owned XXH3-128 for non-security fingerprints, quick corruption
checks and deduplication. Use BLAKE3 when integrity depends on a trusted digest
against adversarial modification. Keep SHA-256 where compliance or an external
protocol (OCI, OAuth, published manifests) requires it. XXH3 fingerprints are
probabilistic identities; use byte equality when exact comparison is required.
Changing a stored algorithm advances its explicit format; obsolete formats are
refused and owned fixtures regenerated. Add legacy support only when requested.
Do not truncate BLAKE2b-512 to pretend
it is BLAKE2b-256: the benchmark configures a 32-byte output directly.

Build the optional `compare_hashes` example with `--features benchmark`, then
set `CAPSEM_CRYPTO_BENCH_BINARY` and `CAPSEM_CRYPTO_BENCH_FILES` (a JSON array
of immutable EROFS paths) for the `crypto` benchmark collector. It prints raw
paired wall-time samples for 32-byte through 256-MiB memory inputs and cached
file reads. It rotates algorithm order, uses one thread and bounded memory,
and includes a read-only control. Streaming timings include state creation;
the large BLAKE3 state is boxed. Separate paired `oneshot` metrics compare
allocation-free BLAKE3-256 and XXH3-128 on 32 B, 128 B, 1 KiB, 64 KiB, 1 MiB
and 10 MiB inputs. Each records eight raw batch samples with alternating order;
divide batch timing by the iterations named in its metric for per-call cost.
These are hot-memory microbenchmarks, not end-to-end frame latency or per-call
tail distributions. Digest widths are explicit; XXH3's 128 bits are padded only
in the example's uniform result container. No payload is copied in the timed
one-shot calls. `capsem-bench-rs run crypto` owns statistics
and storage. `CAPSEM_CRYPTO_BENCH_SAMPLES` imports retained raw samples without
repeating the measurement.

The Rust boundary covers normal, build, dev, renamed, inherited and target
dependencies, including SDK and future workspace members. It also rejects
UUID entropy generated outside the owner. Guest EROFS verification streams
the launcher-held descriptor into the existing `capsem-sysutil verify-blob`
command; its lightweight build enables BLAKE3 without host TLS dependencies.

Python tooling uses `capsem_builder.crypto`: OpenSSL hashlib for protocol
hashes, SIMD BLAKE3 for content, native constant-time comparison and OS-seeded
entropy. Node SDK/package acceptance tools use
`sdk/typescript/tools/crypto.mjs`, backed by native Node crypto. Neither
binding changes existing SHA or BLAKE2 identities. Source guards reject raw
provider imports in these production/tooling surfaces.

The exact stdlib-only source-key bootstrap in `gatelaunch.py` keeps hashlib
until its re-exec. Portable installed and guest script fragments use native
OpenSSL-backed hashlib/SHA tools where the builder package is unavailable;
the guest launcher uses SHA only for OCI metadata. Independent Python test
references and standalone SDK acceptance tests also retain native bindings.
These are execution-environment bindings, with format identities fixed by
their contracts; none permits a Rust caller to bypass this crate.

Prepared root artifacts use `application/vnd.capsem.rootfs.erofs.v2`. Their
pinned manifest authenticates `org.capsem.rootfs.blake3=blake3:<64 lowercase
hex characters>` and the platform subject. Transport addresses remain OCI
SHA-256. Download verifies both hashes in one pass; cache reuse and mounting
verify BLAKE3 and exact size. Version 1 artifacts are not admitted.
