# Original real CLI update check

`[benchmark.routes.update_check]` binds one finite POST `/update/check` case
over service UDS and the original authenticated gateway. It never installs,
applies or activates an update. Check admission/first-body/response completion
and independently verified cache/status completion are separate raw clocks.

Before any routes collector fixture, require the existing frozen/cohort runner's
explicit `CAPSEM_RELEASE_BIN_DIR/capsem` and `CAPSEM_ASSETS_DIR/manifest.json`.
The selected executable must be a regular executable, not a symlink, and the
prepared graph must contain current packages and a runtime. No shared Cargo
fallback, standalone synthetic graph fallback, in-collector build, tag fetch or
second cohort/digest authority is introduced. The runner's exact source/artifact
receipt remains authoritative. Missing prerequisites fail before any collector
can start a service, registry or workload.

The original install-test manifest server, graph-update fixture and UserDir CLI
copy are shared through `tests/helpers/update_release.py`. Every finite case has
its own service home. A private byte copy of the selected real CLI is bound only
to that service's child `CAPSEM_CLI`; global environment and installed paths are
untouched. The original service fixture retains startup/teardown ownership.
Private provenance selects the hermetic manifest; installed URL overrides are
not used. Automatic updates and catalog refresh are disabled in the isolated
settings. All linked runtime assets remain readonly input; only the private
manifest/metadata are written.

One original check must return actual child success/exit0 and the exact bound
CLI/argv, with bounded complete output. Development's exit0 “update from source”
message is a failure. Independently the manifest server must see exactly one
request for the known candidate graph and no other requests; the owned cache's
SHA256/source/tracks must match independently known bytes; public update status
must agree and be valid/fresh. No cached/no-op response or automatic poll can
satisfy these checks. Artifact URLs remain under the original graph's authority
and are never downloaded by this manifest-only case. Final upstream counts are
checked again after the original service teardown.

The existing strict single-attempt Transport, Cycle, raw Observations and
ProcessTreeSampler own receipts/resources. Sampled child CPU/RSS peaks are
sampled observations: a short-lived CLI can fall between sampler ticks; they
are not an exact process-wide CPU total. Rust remains the single statistics,
raw-store and fitness owner. Counts and all corpus/output/deadline bounds live
in config. Finite checks never join concurrent512-request read loops.

Source doubles only validate wiring and adversaries. Genuine real CLI/native
acceptance and fit two baseline/two candidate runs on one matching cohort remain
mandatory. `/update/apply`, package self-retirement, new-service readiness and
signed installed-product effects remain separate existing glowup/native-owner
work. A successful check does not qualify those effects or bypass update
network/integrity boundaries. Updater process lifetime bounds remain the service
owner's responsibility, not an outer client timeout or fixture workaround.
