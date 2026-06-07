//! Compile-time hash of the protocol enum source bytes. Detects "I added
//! a variant in the middle without bumping PROTOCOL_VERSION" -- silent
//! re-numbering of bincode variants. Hashes protocol type source bytes
//! (FNV-1a 64),
//! emits a `schema_hash.txt` file containing a `u64` literal which
//! `lib.rs` includes via `include!()`.
//!
//! Hand-rolled FNV (no extra crate deps); the cost is negligible because
//! the input is small (a few thousand bytes of enum source), and we get
//! to keep `[build-dependencies]` empty.

use std::path::Path;

fn main() {
    let src_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    // Files whose bytes we hash. Adding a new file that defines protocol
    // types carried by IPC/vsock? Add it here. Comment-only edits trip the
    // hash; we accept that fast-and-loud cost in exchange for not pulling in
    // `syn`.
    let files = ["lib.rs", "ipc.rs", "handshake.rs", "metrics.rs"];

    let mut hash: u64 = 0xcbf29ce484222325; // FNV-1a 64 offset basis
    for f in files {
        let path = src_dir.join(f);
        let bytes = match std::fs::read(&path) {
            Ok(b) => b,
            Err(e) => {
                // handshake.rs may legitimately not exist on first build
                // before the file is created; treat as empty so the
                // bootstrap commit can compile.
                if e.kind() == std::io::ErrorKind::NotFound {
                    Vec::new()
                } else {
                    panic!("schema_hash build script: read {}: {}", path.display(), e);
                }
            }
        };
        for b in &bytes {
            hash ^= *b as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }
        println!("cargo:rerun-if-changed={}", path.display());
    }

    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR not set");
    std::fs::write(
        format!("{}/schema_hash.txt", out_dir),
        format!("{}u64", hash),
    )
    .expect("schema_hash build script: write OUT_DIR/schema_hash.txt");
}
