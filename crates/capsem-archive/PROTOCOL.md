# Archive generations and atomic ledger publication

Status: implementation specification for PR #227; **not implemented yet**.
Designed against `c8da3c1272c77c1ba07784de6176da13fae4e493` on 2026-09-21.
The associated Sprinty ledger is worktree-local at
`/Users/elie/.codex/worktrees/pr227-review/capsem/.sprinty`.

## Historical decision: snapshot retirement is outside this sprint

The active sprint and Sol handoff cover PR #227 only. The retirement decision
below is preserved as future context, not an implementation assignment.
S07-001 and the counters, IPC and confinement subsprints are deprecated from
this execution queue. SDK cleanup and network/proxy work are outside its scope.

The user explicitly chose **Remove all snapshot features**, and explicitly
included SDK cleanup. Deprecated S07-001 records the former #221/#216 plan's
replacement. A separate future issue should remove scheduling and all snapshot
create/list/status/change/history/revert/compact product surfaces. Remove their
MCP tools, server routes, IPC, configuration, UI, documentation and tests.
Remove snapshot methods, request/response types, exported symbols, generated
schemas, examples and tests from both SDKs; regenerate from their actual source
of truth. Do not keep deprecated aliases or methods returning unsupported.
This is explicit authorization to remove the corresponding public-surface
entries, not a request to disable the feature behind a setting.

Preserve existing saved snapshots and session data; no cleanup migration is
authorized. Do not implement a replacement scheduler or merge #216 to expand
snapshots. Remove snapshot-only tests and add only focused removal/regression
checks using the existing gates.

`auto_snapshot` currently also houses clone and disk-usage primitives used by
ordinary VM creation and fork. Move still-needed primitives to their proper
owner before deleting that module. Do not remove normal persistence, file I/O,
or unrelated SQL/state snapshots by keyword. The coherent ledger-copy protocol
below still protects archive evidence and any retained cloning operations; it
does not require retaining a user-facing snapshot feature. Do not expand
snapshot functionality as part of #227. No later feature is assigned here.

## Decision

Give every physical archive generation its own unique filename. SQLite is
the sole authority selecting the current generation. Retention writes and
durably installs a new generation **before** atomically committing the new
generation identity and every remapped index row in SQLite. It never replaces
the file named by the old generation and never reverses offsets following a
filesystem error. Old files become garbage only after durable publication.

A reader captures one SQLite snapshot and opens its selected generation under
a short shared acquisition lock. Its open descriptor pins that generation
after the lock is released. Retention can unlink an old generation while a
reader finishes: the reader continues using its descriptor and captured rows.

This is copy-on-write publication with one transactional root, not a
two-resource commit protocol. There is no CURRENT file, symlink, recovery
journal, pending-generation table, or election of the newest file. File
creation is preparation; the SQLite commit is publication; unlink is GC.

The existing compressed block/segment design remains. The outer file header
and database schema change to identify generations. Compression, body hashes,
open segments and cross-block dedup are reused.

## Scope and guarantees

This fixes retention crash consistency, directory-sync error handling,
concurrent body reads, WARC capture and ledger backup pairing. Retention stays
at the existing quiesced writer/shutdown boundary. Do not add online background
compaction, another writer or the #206 subprocess in this change.

For supported local filesystems with working locking and durability barriers:

1. Every durably committed body reference resolves within its selected
   generation's committed extent, or reports actual corruption explicitly.
2. A crash before publication leaves the old generation authoritative. A
   crash after publication leaves the new generation authoritative. An
   interrupted commit may recover either complete SQLite transaction.
3. Readers never combine one generation's offsets with another generation's
   bytes. Legitimate retention never increments corruption/skip counters.
4. No old generation is mutated after publication of its successor. An
   already-open reader may finish even if that generation is unlinked.
5. Only the ledger writer creates, appends, publishes or removes generation
   files. Read workers own query execution and pins; routes receive results.
6. RAM consumption is bounded independently of ledger size. Temporary disk
   and open-descriptor retention have explicit budgets and deadlines.

These guarantees cover process death and ordered, successful durability
barriers under OS/power failure. They do not authenticate data against a
compromised same-UID host process, make snapshots securely erase expired data,
or compensate for hardware that lies about flush completion. The future #206
producer hash chain and confinement address a different threat.

## Files and identities

For a disk ledger `session.db`, the owning logger resolves:

```text
session.db
session.db-wal / session.db-shm     SQLite-owned
session.db-writer.lock             existing lifetime exclusive writer lock
session.db-archive.lock            stable acquisition/publication lock
session.bodies/                    private directory, mode 0700
  g-<32 lowercase hex digits>.cbl  generation file, mode 0600
```

The basename is derived from a typed 16-byte `GenerationId`, never a path
supplied by a route, stored SQL string or producer. Use CSPRNG UUIDv4 values
and the UUID bytes in canonical byte order. `ArchiveId` is another UUIDv4,
created once with the ledger. It identifies storage lineage, not authorization
or the runtime session identity. A coherent fork preserves ArchiveId and the
copied GenerationId; future generations diverge. Session identity remains the
owner's grant and is never inferred from these IDs.

Create a candidate directly under its final unique name using exclusive
creation, no symlink following, close-on-exec and regular-file validation.
No staging rename is needed: until SQL references it, even a fully written
candidate is uncommitted garbage. Collision means choose a new random ID;
never truncate/reuse an existing generation name. Keep the created descriptor
for adoption by the writer after commit.

Anchor operations to a validated private directory descriptor. Validate the
directory, lock file and generation ownership/type; refuse links and special
files rather than following them. No hard-link backups: a generation may
still be the active append target. Never unlink or recreate either lock file
while the ledger directory is live. Readers open the existing archive lock;
they must not create the directory, repair permissions or initialize state.
Use/extend the foundation's existing lock and no-follow primitives.

An old regular file at `session.bodies` is a v2 layout, not a directory to
replace automatically. Refuse it explicitly and preserve it.

## File format v3

The exact generation header is 80 bytes. Unsigned integers are little-endian;
UUID and hash fields are raw bytes, without integer byte reversal.

| Offset | Bytes | Value |
|---:|---:|---|
| 0 | 8 | ASCII `CAPSEMBL` |
| 8 | 2 | File version `3` |
| 10 | 2 | Flags `0` |
| 12 | 4 | Header length `80` |
| 16 | 16 | ArchiveId |
| 32 | 16 | GenerationId |
| 48 | 32 | BLAKE3 of bytes `[0,48)` |

Reject unsupported version/flags/header length, truncated headers, invalid
UUIDv4 encoding, hash mismatch, filename/generation mismatch and SQL/header
identity mismatch. A header hash detects corruption; it is not a MAC.
Header bytes never change after creation. Do not place a mutable EOF, current
generation marker or compression dictionary pointer in this header.

At offset 80, retain the existing v2 `BLK2` block and `SGMT` segment layout:

| Record | Layout |
|---|---|
| Block, 8 bytes | `BLK2`, u8 codec, u8 flags=0, u16 reserved=0 |
| Segment, 52 bytes + payload | `SGMT`, u8 flags, 3 reserved zero bytes, u32 raw_start, u32 raw_len, u32 comp_len, 32-byte BLAKE3(raw segment), compressed bytes |

Codec 1 remains raw deflate. Segment flag bit 0 is FINAL; reject all other
bits. Existing sync-flush, decompression and per-segment hash checks remain.
Keep the 1 MiB target, 16 MiB block ceiling, 10 MiB stored-body cap and current
compressed-segment expansion bound in their owning constants. Use checked
arithmetic before allocations, seeks and integer conversion. Reject overflow.

An on-disk BodyRef remains `(block_offset: u64, offset: u32, len: u32)`;
block_offset is absolute from the generation file's beginning. SQL stores
only values representable as nonnegative signed 64-bit integers. A reference
is meaningful **only inside a captured generation lease**. The public Rust
read interface must not accept a naked reference plus a freshly resolved
current path.

Each read also carries the selected block's committed `disk_len` and
`raw_len`. Stop parsing at those bounds, even if the actual descriptor has
additional committed-later segments or an uncommitted tail. Verify the body's
existing index hash after segment verification. Row count/length and content
integrity are separate checks.

Future codec IDs can reuse the envelope only if they satisfy these segment
rules and their exact decoding contract. #206 must not load C zstd outside
the confined ledger. Dictionary identification is deliberately not invented
in reserved bytes here: that extension needs its own explicit block-format
contract and compatibility tests.

## SQLite contract

Add one required, disk-only singleton to the session schema:

```sql
CREATE TABLE archive_state (
    singleton INTEGER PRIMARY KEY CHECK (singleton = 1),
    archive_id BLOB NOT NULL CHECK (length(archive_id) = 16),
    generation_id BLOB NOT NULL CHECK (length(generation_id) = 16),
    format_version INTEGER NOT NULL CHECK (format_version = 3),
    committed_end INTEGER NOT NULL CHECK (committed_end >= 80),
    revision INTEGER NOT NULL CHECK (revision >= 1)
);
```

Exactly one row is required; absence is a broken/uninitialized schema, never
an empty archive. Validate SQLite storage classes as well as lengths in the
decoder/schema contract (STRICT table or explicit `typeof` constraints).
`revision` increments at publication and recovery durability fences; it need
not increment for every body append. Overflow fails explicitly.

All `body_blocks` and `event_body_blobs` rows in a SQLite snapshot belong to
that snapshot's singleton generation. Do **not** repeat generation_id in every
body row or store filesystem paths. A transactional pointer switch plus
transactional remapping is sufficient because only one generation supplies
the entire live index. Keep foreign keys enforced.

`committed_end` is the maximum durable extent indexed by that snapshot, or 80
when empty. In the same transaction that inserts/extends block and body rows,
advance it to cover every indexed segment. It may include unreferenced gaps;
it is not a promise that every byte is indexed. Validate
`block_offset >= 80`, `block_offset + disk_len <= committed_end`, and the
body's raw span against the joined block row. The descriptor must have at
least committed_end bytes. The path's mtime/size is not a freshness signal.

Use the repository's schema identity mechanism to reject older ledgers. There
is no automatic v2 migration and no recreation of a damaged archive. Regenerate
approved fixtures via their existing owner. Preserve old persistent ledgers
and emit a precise version incompatibility error; never delete them to pass
tests. A future migration is a separate explicitly scoped operation.

## Durability primitives and normal appends

**Disk session writer connections use WAL plus `main.synchronous=FULL`.**
On macOS enable SQLite `fullfsync`; verify effective values when opening the
writer. Scope this change to session durability, not every unrelated database.
Keep batching: FULL does not mean one SQL transaction or device flush per
request. Record its performance impact in the final Gemma measurement.

The present NORMAL setting is insufficient before reclaiming a previous
generation: SQLite permits a power failure to roll back a visible NORMAL
commit. A FULL WAL commit is the durable decision this protocol needs.
[SQLite's durability contract](https://www.sqlite.org/pragma.html#pragma_synchronous)
and [macOS fullfsync setting](https://www.sqlite.org/pragma.html#pragma_fullfsync).

Centralize fallible durability operations in the existing foundation owner:
Linux file `fsync` (including new length), macOS file `F_FULLFSYNC`, plus
directory `fsync` to persist creation/deletion of names. Propagate failures.
Qualify the chosen directory/filesystem path on APFS and Linux ext4; no
ignored EINVAL or best-effort fallback may be called durable. All barriers
precede dependent publication. Filesystem namespace durability needs its own
barrier; syncing file content alone does not persist its directory entry.
[fsync contract](https://man7.org/linux/man-pages/man2/fsync.2.html).

For ordinary batches:

1. Write segments only to the currently adopted generation descriptor.
2. Durably sync the archive bytes/length.
3. In one FULL SQL transaction, commit the related ledger rows, body/block
   index and committed_end. Respect existing producer-buffer admission versus
   flush-barrier semantics; enqueue acceptance is not a durability promise.
4. Release acknowledged flush results only after this succeeds.

Bytes appended without an index commit are harmless unreferenced tails. On
reopen, do not resume an existing compressor or truncate the file under
readers. Start a new block at actual EOF as today; later retention removes
gaps. Missing/truncated active committed bytes fail readiness. Archive I/O
failure must not let the durability barrier report success for lost bodies;
keep admission/failure handling explicit and fail closed where required.

## Locks, acquisition and publication

There are two roles, not two writers:

| Lock | Holder | Duration |
|---|---|---|
| writer.lock EX | one writer | writer thread lifetime |
| archive.lock SH | reader acquisition | fresh snapshot selection and descriptor open only |
| archive.lock EX | writer publication/recovery/GC | short metadata operation and SQL commit |

Lock order is writer ownership (when needed), then archive.lock, then SQLite
transaction. Never acquire archive.lock while already holding a SQLite
transaction. Reader acquisition starts on a connection with no active
statement/transaction or retained old snapshot. Never call from a writer
transaction back into a reader that must reacquire archive.lock.

Use bounded nonblocking lock attempts in a DB-owned blocking worker, with the
operation deadline/cancellation token. Do not use foundation's unbounded
`acquire` loop or block a Tokio worker. Each acquisition must own an independent
lock descriptor; do not share one flock handle whose unlock releases another
thread's protection. Preserve foundation's same-process reservations and
cross-process lock tests. Lock upgrading is forbidden.
[flock lifetime semantics](https://man7.org/linux/man-pages/man2/flock.2.html).

The shared lock is released after the reader has established a fresh SQLite
snapshot and opened/validated its generation FD. It need not remain held
while that snapshot's rows are materialized. Publication can then proceed;
the old SQLite snapshot and old FD remain a matched pair. SQLite WAL readers
retain a stable database view until their transaction ends.
[SQLite snapshot isolation](https://www.sqlite.org/isolation.html).

## Retention state machine

Run on the one writer at its existing quiescence barrier. Stop accepting new
producer work for shutdown, drain accepted operations, close/index the open
block and complete a durable flush. No writer mutation runs concurrently with
candidate construction. The writer lock remains held throughout.

Let G be current and H the newly allocated generation. Fix the cutoff once.

**R0 — Determine survivors.** Read the existing retention predicate once as
a logical snapshot. Preserve its semantics: a block survives if its newest
segment is within the window **or any recent indexed body references it**.
Do not change row-age policy or delete source/counter rows in this repair.
Skip compaction when nothing expires and there are no reclaimable gaps.

**R1 — Build H.** Stream retained blocks in ascending old offset, copying only
their committed extents verbatim behind H's new header. Validate segment
boundaries; do not inflate/recompress whole blocks to copy them. Previously
crash-stranded blocks may end at a non-FINAL committed segment and remain
readable; no writer will extend those streams. Close the normal active stream
at the quiescence barrier. Keep a bounded I/O buffer and scalar counts/end.

Do not retain the whole offset map in RAM. The writer is quiescent: a second
ordered pass over the surviving block rows can recompute each new offset from
80 plus preceding disk_len values. During SQL remapping use keyset traversal
`old_offset > last_old_offset`, fetching one bounded page/row at a time; move
only downward in ascending order. Do not mutate a table while a live cursor
is scanning it. Defer FK checks within the transaction and update both parent
and child offsets. Assert copied/remapped counts and resulting end agree.

**R2 — Make H durable.** Sync H's data/length, then fsync `session.bodies/`.
If either fails, do not change SQLite. Close/remove the unreferenced candidate
when safely possible; cleanup failure is reported as garbage pending. Keep G
unchanged and usable. There is no rename result to compensate.

**R3 — Acquire archive.lock EX.** No file copying is done under this lock.
Ensure the current tuple still equals the G tuple used for staging. A mismatch
is a violated writer invariant: abort before publication, not a silent retry.

**R4 — Publish.** In one FULL SQL transaction, delete expired body/block index
rows, remap all survivors, and set archive_state to H, its new committed_end
and revision+1. Preserve ArchiveId. This successful commit is the linearization
point. Before it, G is authoritative; after it, H is authoritative.

**R5 — Adopt.** Using the already-held H descriptor at its new end, reset the
writer's block/compressor/dedup caches and adopt H. Close G's writer descriptor.
Do not reopen G by path or append there if adoption fails; transition the
writer to unavailable and recover through the owning startup protocol.

**R6 — Reclaim.** Still under EX, unlink only unreferenced owned generation
names, including G, then sync the archive directory. A deletion/sync failure
does not unpublish H or roll back SQL: return `Published { gc_pending: true }`
and retain diagnostics. Release EX. Future writer startup retries GC.

The writer may release EX immediately after adoption and run GC in a later EX
section with a fresh authoritative state read, if cleanup is large. It must
not delete from a saved list without rechecking the active ID under EX.

Use typed internal outcomes:

```text
NotPublished { phase, cause, orphan_cleanup_pending }
Published { old_generation, new_generation, logical_bytes_removed, gc_pending }
OutcomeUnknown { phase, cause }  // writer unavailable pending recovery
```

Do not equate unlink success with immediately reclaimed physical bytes: an
export may still hold the old inode open. Report logical archive size and
pending/pinned storage separately where evidence is collected. Preserve
existing public response schemas; these are internal types.

### COMMIT errors and cancellation

A COMMIT error or cancellation after entering commit is not proof of rollback.
On uncertain outcome, retain **both** generations, stop appending and do not
GC. Close the failed writer connection/descriptor state. Under the writer
lock and archive EX, open/recover a fresh SQLite connection; inspect its
authoritative row and validate the selected file. Then durably commit a real
`revision = revision + 1` update using FULL before reclaiming anything. The
extra write establishes a durability fence even if the previous commit was
visible but its sync reported failure. A no-op empty transaction is not a
substitute. If the fence fails, remain unavailable and keep both files.

Cancellation before R4 leaves an orphan, which is safe. Cancellation after a
successful R4 is a published operation: adopt or retire the writer, then let
GC complete/defer. Do not let a generic RAII candidate destructor remove H
once commit has been attempted and its outcome is uncertain. Kill during this
resolution is another ordinary startup-recovery case.

## Startup and crash recovery

Acquire writer.lock EX and archive.lock EX before recovery. SQLite first
recovers its WAL. Validate schema, singleton and the selected generation
header/length before readiness. Never scan names to pick a replacement.
For a recovered existing ledger, perform the FULL revision fence above before
GC; fail closed if it cannot be completed. Reopen appending only after this.

Cleanup examines the dedicated managed directory and removes exact
`g-[0-9a-f]{32}.cbl` names other than the active ID. Recognized unreferenced
candidates can have torn headers from interrupted writes. Validate each entry
is an owned regular file with no unexpected links before unlinking. Unknown
names, foreign ownership, symlinks or special files are not deletion targets;
report them. If the authoritative state cannot be validated, do no cleanup.
Directory iteration and deletion use bounded memory, never a list of all files.

Read-only handles do not perform recovery mutations or GC. They may acquire a
consistent published snapshot after process death using the existing lock and
valid SQL state; an invalid/missing active file is an explicit error.

| Failure boundary | Recovered SQL | Required outcome |
|---|---|---|
| During H creation/copy | G | G intact; partial H is garbage |
| H synced, directory not yet synced | G | G intact; H may exist or vanish |
| Directory synced, before SQL transaction | G | H is durable but unreferenced |
| During remap, before commit | G after rollback | No partially moved offsets visible |
| Commit in flight / error returned | G or H | Validate elected SQL file, durability fence, then GC |
| FULL commit succeeds, before writer adoption | H | Reopen H; never resume G |
| During unlink of G | H | Open G readers work; restart selects H |
| Unlink succeeds, directory sync fails | H | G may reappear after crash; GC retry only |
| GC completed | H | H remains complete and authoritative |

Initial creation follows the same order: create/sync private directories and
stable lock entries (including parent-directory barriers), create/sync the
empty generation, then create the required schema/singleton in a FULL
transaction before publishing session readiness. A crash before a valid
initial schema exists leaves an incomplete session, not an empty valid ledger.
Only the lifecycle owner may clean up an unpublished directory it created;
opening an existing persistent path must not guess or erase it.

## Reader and WARC API

Replace `query offsets -> await -> open current path` with one logger-owned
capture request. The synchronous reader worker performs:

```text
acquire SH with deadline
BEGIN fresh read transaction
SELECT archive_state                   // establishes SQLite snapshot
open selected generation, validate identity and size
release SH                             // FD and SQL snapshot now pin the pair
materialize required rows + committed block bounds from that transaction
COMMIT read transaction
return CapturedBodies { identity, descriptor, bounded rows }
```

Errors drop the SQL transaction, descriptor and lock via RAII. A missing
selected file is corruption, not an ENOENT retry against a different generation.
All chunks of `read_bodies_for_events` belong to this one SQL snapshot; its
parameter chunking is not permission to change generation halfway through.
Empty result still validates the required singleton/schema, then closes the FD.
The capture executes its SQL in that transaction, bypassing generic cached
JSON/index results. A cached result from an earlier snapshot cannot be paired
with the newly opened generation. Ordinary route caches keep their existing
DB-owned data_version contract; they are not an archive lease.

The body inflater consumes the captured descriptor. It does not stat/reopen
the current path. Any cached decoded block is keyed by ArchiveId, GenerationId
and block offset; do not leave idle cached FDs pinning deleted files. The first
implementation uses one request-scoped `BodyLogReader`, retaining its inflater
across that request's batch. Optimize cross-request caching only if measured.
This adds one generation open/header validation per body batch; measure it.

WARC must finish capturing its joined index/URI/date metadata from one snapshot
before network backpressure begins. Replace the current unbounded
`Vec<ExportRow>`/JSON copy with a bounded private metadata spool owned by the
logger; stream SQL rows into it while the generation FD is pinned. End the SQL
transaction before streaming bodies to the client, so a stalled download does
not hold WAL checkpoints back. The spool is temporary typed data, not a second
ledger or a public archive format. Use bounded framed serialization and reject
oversized records rather than allocating from unchecked lengths.

Do not keep the current export UNION/global GROUP BY materialization hidden
behind the spool. Walk body rows by indexed keyset
`(block_offset, body_offset, id)` in the same snapshot, join their block bounds,
and resolve each source row with an indexed `event_id ORDER BY id LIMIT 1`
lookup. Preserve the existing first-source-row semantics and missing-row skip
accounting. Add/reuse the supporting index and verify EXPLAIN QUERY PLAN and
large-ledger measurements. Check text/blob lengths through borrowed SQLite
values before allocating owned strings. Thus the spool bounds Rust metadata
without moving an unbounded sort into SQLite memory.

Implementation limits, defined once in the logger-owned operation budget:

| Resource | Initial limit / behavior |
|---|---|
| Archive lock acquisition | 5 seconds, cancellable; typed busy error |
| Interactive metadata capture | 10,000 requested IDs and 16 MiB decoded metadata; 5 second deadline |
| One WARC metadata record | 64 KiB encoded ceiling; fail capture explicitly if exceeded |
| WARC metadata spool | 256 MiB per export; fail before streaming if exceeded |
| WARC capture | 30 seconds with SQLite progress-handler cancellation |
| WARC exports | 1 per session and 2 per owning process; reject excess, no unbounded queue |
| WARC stream | 15 minute total deadline and 30 second blocked-write deadline |
| Retention/copy I/O buffer | 1 MiB; constant memory plus bounded SQL page/cache |
| Retention attempt | 5 minute deadline; timeout before publication leaves G |

Preserve existing tighter response-body budgets. Enforce metadata budgets
during materialization, not after a huge Vec/JSON allocation. These are internal
resource limits, not new public configuration keys. Instrument limit refusals
and adjust only with measured evidence and updated tests, never by removing a
bound. Stream cancellation must wake blocked channel writes: a token checked
only between calls to an arbitrary blocking `Write` is not a deadline. Use the
existing controlled channel sink with timed/cancellable sends and RAII permits.

Existing WARC corrupt-row skip reporting and terminal warcinfo remain. Actual
corruption may be counted; acquisition, generation mismatch, resource refusal
or hard I/O failure must not be reclassified as a successful export with skips.
Hard failures have no completion marker. Avoid unbounded `summary.skipped`:
retain exact counts by reason and only a bounded diagnostic sample of IDs.

Retained old inodes consume disk until active leases end. The deadlines and
concurrency bounds limit the number and lifetime of pins, not the size of an
individual generation. Budget peak disk as current generation + candidate +
pinned retired generations + spools + SQL/WAL. No claim of a fixed byte ceiling
independentently of ledger size is valid. ENOSPC before publication leaves the
old state intact; cleanup must never delete an active generation to make room.

## Fork, backup and forensic collection

`snapshot_session_ledger` is part of this change. Copying a database and then
opening whatever archive is currently at a path is invalid under retention.

For #227, keep `VACUUM INTO` and hold source archive.lock SH from before that
operation until its completed destination database has been inspected and the
selected source generation FD has been opened/validated. This keeps generation
publication stable while VACUUM takes its own consistent snapshot. Do not wrap
VACUUM in BEGIN; use the actual destination singleton/committed_end as the
backup's authority. Normal source appends may continue. Bound the snapshot
operation (initial 5 minutes, progress cancellation). The longer SH here is an
explicit backup exception; retention may return busy and retry later.

Release SH after pinning. Copy exactly the destination snapshot's committed_end
bytes from that FD into a private destination generation, independent of any
later source append/retention. Sync it and its directory. Validate destination
DB/FD identity, block bounds and schema. Create fresh destination lock files;
do not copy runtime locks, WAL/SHM files or arbitrary orphan generations.

Build the destination ledger in the snapshot owner's unpublished directory.
Only expose that snapshot/fork after DB and archive are both durable; use the
existing snapshot publication mechanism, with its parent-directory barrier.
Never overwrite a visible destination ledger in place. This guarantees a
coherent ledger capture, not atomicity of all workspace/rootfs snapshot bytes;
whole-workspace snapshot behavior is outside this change.

Apply the same logger-owned capture contract to Gemma's preserved evidence.
Do not separately copy a live `.db`, `-wal` and archive directory. Prefer a
coherent snapshot before teardown, plus measurements captured during the run.
Doctor helpers must resolve the snapshot's singleton/generation and understand
v3; the only Python decoder remains the existing fixture/diagnostic owner.

## Implementation boundaries and rejected shortcuts

- `capsem-archive`: v3 header, typed identities, descriptor-based bounded reads,
  streaming retained-block copy. No SQLite or runtime session semantics.
- `capsem-logger`: schema, generation selection/publication, locks/leases,
  retention predicate, reader-worker capture, WARC spool, backup and recovery.
- `capsem-foundation`: secure file/lock primitives and durability barriers.
- `capsem-process`: existing lifecycle invokes logger barrier; no direct GC.
- Service/gateway: consume results and cancellation-aware export sink; never
  open SQLite or derive generation paths themselves.

No compensating offset transaction after a filesystem error. No current-file
rename protocol with an in-memory recovery flag. No mtime/inode freshness
inference. No catch-and-retry hash failure that hides a wrong generation.
No SQL transaction or publication lock held during network streaming. No
in-memory map proportional to all retained blocks. No reader registration
database or PID lease file: kernel descriptors already provide pin lifetime.

#223 counters must be committed with the data they represent and define
whether retention affects them; this repair does not silently change lifetime
counts. #209 does not block this local storage protocol. Under #206, these
operations move together inside the confined ledger; outside clients send
typed query intent and receive verified bytes, never SQLite handles, archive
paths or descriptors that reintroduce the C reader attack surface.

## Required proof before merge

### Verification cost is part of the contract

The user explicitly accepts the improved format, but rejects expanding every
Capsem test run into a prolonged qualification exercise. The matrix below is
a list of assertions to cover, **not a Cartesian product of test runs**.
Keep the v3 design; do not build a general chaos framework to implement it.

- Put format, malformed-input and failure-result cases in the existing archive
  and logger tests. Use one small three-block ledger fixture and table-driven
  cases, not a separate large database or VM for every assertion.
- Use one subprocess crash driver, at most 12 selected publication/recovery
  cut points, and at most six deterministic reader interleavings. Synchronize
  through pipes/barriers; no random stress loops, sleep-based races or retries
  until green. Each case checks several applicable assertions below.
- Target **under 15 seconds of added compiled-test runtime per platform** for
  the complete focused protocol group; bound that group to **60 seconds**.
  Measure the incremental duration against the baseline. Compilation is
  reported separately. If the group exceeds the budget, improve fixtures and
  synchronization; do not silently increase timeouts or drop correctness cases.
- Inject file/sync/commit errors at the owning test seams. Do not fill the
  machine's disk, exhaust host descriptors, reboot hosts or create filesystem
  images for each case. Actual process death plus the abstract durability model
  covers the two different questions without pretending SIGKILL is power loss.
- Reuse the existing macOS and Linux CI jobs. No new workflow, platform matrix,
  Docker/Tart build or full `just test` invocation per milestone. Existing
  required gates still run at their normal point in the workflow.
- Add generation/retention assertions to **one existing VM lifecycle fixture**,
  reusing its boot, persistent stop/reopen and snapshot path. Reuse bodies
  already emitted by existing HTTP/model/tool/exec/security fixtures. Do not
  multiply VM boots by failpoint, protocol, body size or platform combination.
- The abstract model stays a design artifact, not an additional production
  gate. Its hundreds of thousands of logical schedules are memoized over
  hundreds of states and the whole recorded run took under one second.
- Run the **30-minute Gemma measurement once on the final candidate**, manually.
  It is never added to routine tests or CI. Run focused existing performance
  benchmarks once for changed hot paths; rerun only when a relevant fix changes
  the result. No repeated soak or million-row sweep at every milestone.

Record actual added test time in the final evidence. The budget is currently
a design requirement, not a claim that the unimplemented Rust suite meets it.

Implement deterministic fault hooks behind test support, using process pipes
or barriers rather than sleeps. Existing Rename fault injection is insufficient.

| Proof | Exact assertion |
|---|---|
| Format golden vectors | Fixed header bytes/hash, UUID order, malformed/reserved/version rejection |
| Crash cut points | Selected distinct R0–R6/recovery durability boundaries within the 12-case driver; reopen returns exact retained hashes and the specified G/H state |
| Durability fault model | Distinguish visible/durable namespace, bytes and SQL; power-loss subsets satisfy referential integrity |
| File/directory sync errors | No publication before R2; post-commit GC errors never restore offsets |
| Uncertain SQL commit | Exercise both rolled-back and visible-new outcomes; no GC until successful recovery fence |
| Recovery interrupted | Kill before/during/after fence and GC; repeating startup converges without data loss |
| Reader ordering | At most six selected schedules spanning SH, snapshot, FD open, release and row capture; exact bodies, zero retention-induced skips |
| WARC ordering | Export captured G while G is unlinked and H appended; exact record identities/hashes and terminal summary |
| Slow/cancelled client | Deadline releases FD/spool/permits; no active SQLite snapshot or flock remains |
| Backup overlap | Run appends and attempt retention during VACUUM/copy; destination generation/rows agree after reopen |
| Empty/dedup/stranded blocks | All-expired produces 80-byte file; recent refs keep old blocks; non-FINAL committed prefixes survive |
| Append after compaction | New writer uses H descriptor and correct EOF; G is never appended after publication |
| Same/cross-process ownership | Two writers refused, reader leases coexist, EX excludes acquisition, crash releases locks |
| Resource pressure | ENOSPC, descriptor exhaustion, row/spool caps, queue-full and deadlines leave recoverable state |
| Real VM | One existing lifecycle fixture verifies stop/reopen/restore; existing traffic fixtures verify their exact archived bodies |
| Performance | Final-head Gemma once; RSS/disk slopes, body poll latency, capture latency, FULL sync overhead, logical versus pinned disk |

Test production primitives, not only a duplicate algorithm. The accompanying
abstract state model checks the ordering argument; it does not qualify actual
fsync, SQLite, APFS/ext4, Rust cancellation or real VM behavior. Keep both
levels of evidence distinct. Source guards should prevent unleased body reads,
missing FULL session durability, direct route filesystem access and legacy
single-file archive copying. Run the existing focused owners and full required
CI after implementation; do not claim this design document makes #227 green.

## Sol pickup

Resume the explicit Sprinty binding above, open its artifacts, and merge current
main into the implementation branch before coding. The separate Codex task owns
#199; integrate its landed changes without duplicating its fix. This checkout
is an isolated review/design checkout, not the canonical PR branch. Cherry-pick
the design commit into the chosen PR implementation checkout and keep Sprinty
explicitly bound to the checkout actually being changed.

Build in dependency order: format/primitives; schema/durable publication and
recovery; reader capture and bounded WARC; coherent backup/diagnostics; fault
and VM evidence; CI repair/final Gemma/merge. Keep each implemented milestone
revertable and tied to its Sprinty item. All items remain open until their real
gates pass. Stop after #227 qualification and merge. Snapshot retirement, SDK
cleanup, counters, IPC and confinement require separate future work.
The verification-cost contract above applies to every implementation item;
milestones are commit boundaries, not instructions to repeat the full gate.

The implementation items in this binding are:

| Item | Deliverable | Prerequisite |
|---|---|---|
| S01-006 | This design, model evidence and handoff | Design review |
| S06-001 | v3 header, secure files and durability/lock primitives | S01-006 |
| S06-002 | Schema, publication, uncertain outcomes and recovery | S06-001 |
| S06-003 | Reader leases and bounded WARC capture/stream | S06-002 |
| S06-004 | Coherent ledger copies, diagnostics, fixtures and saved evidence | S06-002, S06-003 |
| S06-005 | Production fault/platform/VM qualification | S06-004 |
| S01-003 | Main integration and remaining CI diagnoses/fixes | Can begin independently |
| S01-004 | Final CI, one final-head Gemma run, merge readiness | S06-005, S01-003 |

The abstract model is attached in Sprinty as
`.sprinty/archive_protocol_model.py`. On 2026-09-21 it checked 24 modeled
crash/recovery durable outcomes, 35 states/36 complete schedules with one
reader, and 283 states/494,720 complete schedules with two readers. Four
negative controls detect the original offset/rename ordering, post-rename
offset rollback, GC before durable SQL publication, and unlocked FD acquisition.
These counts describe the deliberately small model, not exhaustive validation
of the implementation. The model does not verify UUID parsing, SQLite query
plans, disk hardware, resource budgets or backup execution.
