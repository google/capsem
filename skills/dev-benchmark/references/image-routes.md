# Finite image and runtime preparation routes

The existing `routes` collector also owns `[benchmark.routes.images]` cases.
These are finite operations, separate from the high-rate read loop: cached
catalogue and actual hermetic refresh, warm verified inventory reads, cold and
warm image pulls, and asset-ensure admission plus materialization completion.
Each cold trial owns a new `ServiceInstance` home and empty product OCI cache;
this does not claim an empty operating-system page cache. Registry request
receipts must show image data transfer for cold pull and none for warm pull.
Inventory reads compare every package/tool version with the canonical producer's
output; warm reads may still authenticate and fetch immutable manifests.

Preparation requires the exact `reference_image` OCI layout/prepared root,
its matching retained local Docker recipe image, and existing image tools.
`prepare_local_evidence` refuses missing or mismatched retained identities.
Its normal canonical export/scan/repack produces inventory and OBOM for the
final local registry subject outside endpoint clocks; no foreign evidence is
relabeled. Setup and service startup are outside individual request clocks.
Missing inputs fail before product children; preparation is not a timed pull.
The normal collector timeout still bounds all setup. Explicit `--quick` reduces
sample counts, not input correctness or timeout enforcement.

Each asset trial unlinks only one entry inside its owned cloned assets directory,
then uses the existing local-file manifest hydration rail. It checks actual
readiness, reconciliation/download counts and exact restored bytes against the
retained source. Shared source assets and the installed user's home are untouched.
Raw headers, client-observed first-body-byte and complete-body samples use the
existing transport and Rust statistics/store owners. Asset completion samples
include the initial admission and bounded public readiness polls; their units
are milliseconds. Existing process-tree CPU/RSS/PSS receipts cover each finite
cycle, including its setup reads/validation/polls, rather than claiming isolated
endpoint CPU from one very short request. Complete sample counts are mandatory.

This is source coverage, not executed native performance evidence. The supported
collector must still run twice on the matching fit cohort; quick/contaminated
results remain diagnostic under the existing benchmark admission rules.
