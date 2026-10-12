# capsem-crypto

This crate owns first-party Rust cryptographic providers. Domain crates retain
their protocol identities, admission policy, certificate cache and trust rules.
They call this crate instead of selecting a hash or TLS backend themselves.

| Feature | API | Contract |
|---|---|---|
| always | `blake3::{hash, Hash, Hasher}` | Existing 256-bit BLAKE3 identities and constant-time hash equality |
| `sha256` (default) | `Sha256` | Streaming and one-shot SHA-256 through CPU-dispatched AWS-LC |
| `sha256` | `websocket_accept_digest` | SHA-1 only for the RFC 6455 handshake |
| `random` | `random` | OS-seeded tokens, identifiers, ranges and fallible entropy |
| `secrets` | `secrets` | Audited constant-time comparison and secret erasure |
| `tls` | `tls` | Rustls provider, private key ownership and certificate minting |
| `jwt` | `jwt` | RS256 public verification; signing and arbitrary key conversion refused |
| `test-fixtures` | `jwt::sign_rs256_fixture` | Test-only signature construction from checked-in fixture keys |
| `benchmark` | `compare_hashes` example | Optional SIMD BLAKE2 candidates and independent SHA reference |

Foundation, protocol and archive callers disable default features to retain
their lightweight BLAKE3 dependency surface. Benchmark candidates do not enter
production binaries. The crypto crate and hot hash backends stay optimized in
dev builds, enforced by Citadel.

Use BLAKE3 for new internal fingerprints. OCI, OAuth and release identities
already requiring SHA-256 keep SHA-256. Changing a stored algorithm identity
requires a deliberate format migration. Do not truncate BLAKE2b-512 to pretend
it is BLAKE2b-256: the benchmark configures a 32-byte output directly.

Build the optional `compare_hashes` example with `--features benchmark`, then
set `CAPSEM_CRYPTO_BENCH_BINARY` and `CAPSEM_CRYPTO_BENCH_FILES` (a JSON array
of immutable EROFS paths) for the `crypto` benchmark collector. It prints raw
paired wall-time samples for 32-byte through 256-MiB memory inputs and cached
file reads. It rotates algorithm order, uses one thread and bounded memory,
and includes a read-only control. Small-input timings include state creation;
the large BLAKE3 state is boxed. `capsem-bench-rs run crypto` owns statistics
and storage. `CAPSEM_CRYPTO_BENCH_SAMPLES` imports retained raw samples without
repeating the measurement.

The Rust boundary covers normal, build, dev, renamed, inherited and target
dependencies, including SDK and future workspace members. Python build tools
and the portable Python guest launcher retain their existing hashlib/OpenSSL
bindings; this crate does not make them depend on a host Rust executable.
