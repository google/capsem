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

## alacritty_terminal 0.26.0

Source: [alacritty/alacritty](https://github.com/alacritty/alacritty), commit
`94e7c8874e526b1e67b349d9ba30ddf81669119e`.
License: Apache-2.0; the original license and all upstream reference fixtures
are retained. The application uses the terminal model, not Alacritty's PTY or
process launcher.

The optional `checkpoint` feature captures both logical grids, history,
current/saved cursors, charsets, tabs, palette, modes, selection, title stack,
keyboard stacks and automatic hyperlink identifier high-water mark. Event
proxies, allocation caches and damage caches are not serialized: restore binds
a fresh event proxy and marks the display fully damaged, without dispatching
effects. The existing upstream grid serialization remains unchanged.

Restore requires equality with the caller's current trusted configuration, so
a saved state cannot enable clipboard access. Explicit caller budgets bound
dimensions, history, combined cell count and text/combining data. Model
validation checks row shapes, cursor/selection bounds, mode bits, stack depths,
wide-cell pairing and semantic state before exposing a usable terminal.
Callers must also bound serialized bytes before decoding. Pair this model with
the pinned vte processor checkpoint to retain unfinished escape/UTF-8 input.
The versioned format is internal implementation state, not an external API.

Three upstream passwd fallback matches are consolidated to satisfy the pinned
compiler's warning-denied Clippy checks. The standalone test lockfile resolves
the sibling vte patch and pins its test-only MessagePack dependency cohort.

Run `cargo test --manifest-path third_party/rust/alacritty_terminal/Cargo.toml
--features checkpoint --locked --offline` and corresponding warning-denied
all-target Clippy through the bounded-command wrapper. Checkpoint tests include
named MessagePack round trips, side-effect-free restore, malformed/budget
refusal, tab-cell preservation and continued processing of every upstream
recording. Upstream unit, reference and documentation tests remain intact.
These prove the dependency interface; session suspend and UI behavior require
their separate integration proof.
