# Pinned terminal dependencies

Elie approved adopting Alacritty with narrow checkpoint patches on 2026-10-22.
The Rust session remains the terminal owner. These dependencies expose state;
they do not acquire host clipboard, process, filesystem or network authority.

Each library retains its upstream licenses and source. `UPSTREAM.json` records
the registry archive SHA-256, source commit and original file hashes.
`CAPSEM-PATCH.json` accounts for every modified or added file. The Citadel
provenance guard rejects unrecorded edits, extra files and redirected sources.
To update a pin, verify the new registry archive, review the local delta against
it, reapply only the necessary patch and rerun checkpoint and upstream tests.
Refresh the provenance records only after reviewing that exact source change.

## vte 0.15.0

Source: [alacritty/vte](https://github.com/alacritty/vte), commit
`3b3da71c34cc1256c7e20981cf03f8eb95e08ffc`.
License: Apache-2.0 OR MIT; original texts are retained in the crate directory.

The optional `checkpoint` feature adds opaque serializable parser/processor
continuations. Restoration validates indices, parameter groups, OSC offsets,
UTF-8 continuation, pending-byte bounds and synchronized-output duration before
exposing a usable parser. It invokes no performer or terminal handler. The
caller bounds serialized input before decoding and owns the pending-byte
budget. The format is versioned implementation state, not a stable external
wire format. Ordinary builds without the feature keep the upstream API.

The patch also removes one redundant debug-format borrow flagged by the pinned
Rust compiler. No upstream parsing behavior is changed. The standalone lockfile
pins the test-only dependency cohort; the application workspace owns production
dependency resolution independently.

Run `cargo test --manifest-path third_party/rust/vte/Cargo.toml --features
checkpoint --locked --offline` through the repository bounded-command wrapper.
Run the corresponding all-target clippy command with warnings denied. Tests
include all upstream parser regressions, every split of representative input,
unprocessed synchronized output, malformed state and deterministic hostile
continuation cases. Session lifecycle persistence requires separate VM proof.
