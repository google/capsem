# Data-path performance

Capsem must sustain many simultaneous terminal and graphical sessions. Treat
every payload traversal, allocation, copy, wakeup and cross-process exchange as
a cost that needs a purpose. At 60 FPS each frame has 16.67 ms end to end; our
processing should remain comfortably below 10 ms, including tail latency under
load, to leave room for rendering. Measure the complete path before claiming it.

## Encode once, bound where bytes are produced

Internal protocols use typed, named MessagePack and binary byte fields. JSON
arrays expand binary data; their size is unrelated to the binary wire budget.
A retained 10 MiB exec output once passed the MessagePack frame limit but was
refused using its expanded JSON size, poisoning ledger admission and refusing
later execs. Fix the encoding boundary, not the limit.

Use one bounded final frame buffer, reserve its header, serialize directly into
it, then fill the header from its length. Reject an overflowing write before it
reaches the buffer or socket. On receive, reject an excessive declared frame
length before allocation. A separate counting serialization doubles work and
can drift from the encoder. Do not allocate an encoded body and then copy it
into another header-plus-body buffer.

## Move or borrow the payload

- Transfer ownership through channels instead of cloning events and their bodies.
- Borrow slices for synchronous consumers. Use shared immutable buffers when
  independent asynchronous consumers need the same bytes and the protocol
  supports them. Bound their lifetimes and queues; sharing is not free memory.
- Forward opaque terminal/graphical traffic without JSON conversion, image
  inspection, decoding or re-encoding in intermediate relays.
- Keep byte fields binary (`serde_bytes` where needed). Do not turn a byte buffer
  into strings or integer arrays merely to fit a generic DTO.
- Prefer a safe, measured implementation. Calling buffered socket I/O
  "zero-copy" does not make it kernel zero-copy. State which allocation or copy
  was actually removed and which remains.

## Require a reason for hashing

When a hash is needed, use `capsem-crypto`: `xxh3::hash` / `xxh3::Hasher` for
XXH3-128 non-security fingerprints, quick integrity and deduplication; BLAKE3
for security integrity against a trusted reference; SHA-256 when compliance or
an external protocol requires it. Do not select a provider at the call site.
Do not replace a mandated algorithm. For owned stored formats, advance their
explicit version/algorithm, reject obsolete inputs and regenerate owned fixtures.
Do not add legacy readers or migration paths unless explicitly requested.
Use full XXH3-128 fingerprints for accepted non-security dedup semantics;
compare bytes when an API promises exact equality.

Document the protected object, the trusted reference and when it is checked.
Examples of concrete reasons include signed artifact verification, certificate
fingerprints, content-addressed OCI objects, archive deduplication, and protocol
compatibility identities. Test the actual guarantee. A hash is not encryption,
authentication, or proof against an owner that can replace both data and digest.

Do not hash every runtime event as speculative protection. The retired ledger
commitment pipeline repeatedly JSON-serialized and hashed producer output,
reserved and anchored hash chains over another descriptor, and later compared
commitment records instead of independently verifying stored event contents.
Descriptor authentication, role grants, ordered requests and durable flushes are
separate guarantees and remain necessary without content hashes.

When hashing is justified, feed the existing bytes directly to the approved
crypto owner. Do not serialize a second representation or allocate a temporary
copy solely to hash it. Changing a durable hash preimage is a format change.

## Require a reason for JSON

Name the external wire contract or persisted format requiring JSON, and keep
that conversion at its boundary. Examples include HTTP/OpenAPI responses, MCP
JSON-RPC, OCI manifests, browser settings and existing user-readable config.
An internal MessagePack request containing an external JSON document may retain
those exact document bytes; this is not a reason to serialize its whole envelope
as JSON. Parse only the fields needed. An owned `Box<RawValue>` avoids a DOM but
still allocates owned storage; use borrowed `&RawValue` when its lifetime permits.

## Prove and preserve the result

Use production codecs in functional tests. Include binary bytes, exact bounds,
over-limit refusal before socket writes, and a valid operation after refusal.
Measure unchanged code twice, then changed code twice through the benchmark
owner. Record CPU, memory, throughput, latency distribution and available stage
attribution. An average or a viewer FPS counter cannot prove a sub-10-ms pipe.
Keep the Citadel guards adversarial: reproduce the retired pattern and observe
the guard fail before trusting its green result.
