# S05-175: Retire the exact broken pre-0.6 public graph

## Why

Hosted stable/code release run `31835485800` selected the structurally valid
stable graph, then failed fetching its package cohort because every package
points at deleted GitHub release `v1.5.1783857731`. Treating every 404 as an
empty channel would weaken the release gate; treating this byte-identical,
known legacy graph as a live cohort makes the first 0.6 release unreachable.

## Design

- Declare the retired first-party channel and exact public-manifest SHA-256 in
  `config/gate.toml`, with a closed stable/nightly channel type and lowercase
  64-hex digest validation.
- Share one standard-library Python authority between runtime selection and
  serialized source resolution. Retirement is accepted only when channel,
  configured digest, catalog digest, and fetched payload digest all agree.
- Extend the hidden `capsem-admin release` bootstrap rail with a
  digest-verified retired-graph mode. It emits an inactive source for the same
  channel with empty package and profile membership; arbitrary Python does not
  author the graph.
- Preserve the public graph until the normal profile/build/pairing/publish
  workflow succeeds. A changed graph, another channel, missing config, or an
  arbitrary broken artifact remains fatal.

## Proof matrix

- Unit/contract: exact retirement selection and source bootstrap.
- Adversarial: digest drift, catalog/payload disagreement, wrong channel,
  malformed digest, non-retired 404, and forged empty source all fail closed.
- Functional: release-assets workflow threads the same retirement decision
  into public-before package/profile staging.
- E2E/release: a newly committed SHA passes `just test`; stable/code hosted
  release stages the inactive profile without fetching the retired package.
- IronBank: the complete candidate at the new SHA remains the black-box VM,
  package, Doctor, Winterfell, and glow-up authority.
- Telemetry/performance: no runtime telemetry change; record hosted run IDs and
  gate timings in Sprinty.

## Done

The exact legacy graph can be replaced through profile-first 0.6 publication,
while all unknown or mutated broken public graphs still stop release.
