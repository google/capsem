//! manyfaces: Capsem must not care how many profiles a channel carries, what
//! they are called, or which of their revisions are on disk at once.
//!
//! Named for the Faceless Men, because a profile is an identity Capsem wears,
//! not a thing it is. Docker settled this model years ago and it is the one to
//! copy:
//!
//! * blobs are content-addressed and shared -- two profiles pinning the same
//!   kernel store one file, as two images sharing a layer do;
//! * a profile is a repository and its `image_revision` is a tag, so several
//!   revisions coexist and each is independently addressable;
//! * lifetime is reference-counted -- a blob dies when nothing references it,
//!   never because some channel-wide pointer moved on.
//!
//! There is no channel-wide "current" asset set in that model, and Docker has
//! no equivalent of one, because it cannot work: the moment two profiles
//! disagree, a single global pointer has to pick a winner and discard the rest.
//!
//! These tests are written against the scenario that breaks the wheel: profiles
//! running side by side, one updated, another added, another removed. They are
//! deliberately red where Capsem still routes assets through a single global
//! version, and they are the specification for making it not.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use capsem_core::asset_manager::{
    cleanup_unused_assets, hash_filename, host_manifest_arch, release_graph_profile_state,
    ManifestV2,
};

/// A distinct BLAKE3 digest per (profile, kind, revision) so any confusion
/// between profiles or revisions shows up as a concrete hash mismatch.
///
/// The same hash the product pins with, so these digests distribute the way
/// real ones do. That matters here: `hash_filename` keeps only the first 16 hex
/// characters, and digests sharing a prefix would collide on disk for fixture
/// reasons having nothing to do with the model under test.
fn digest(profile: &str, kind: &str, revision: &str) -> String {
    blake3::hash(format!("{profile}/{kind}/{revision}").as_bytes()).to_hex().to_string()
}

fn image(profile: &str, kind: &str, name: &str, revision: &str) -> serde_json::Value {
    serde_json::json!({
        "kind": kind,
        "name": name,
        "bytes": 1,
        "status": "current",
        "digest": {
            "blake3": digest(profile, kind, revision),
            "sha256": digest(profile, kind, revision),
        }
    })
}

/// One profile at one revision, for the host architecture.
fn profile(id: &str, revision: &str, image_revision: &str) -> serde_json::Value {
    serde_json::json!({
        "revision": revision,
        "status": "current",
        "architectures": [{
            "architecture": host_manifest_arch(),
            "image_revision": image_revision,
            "config": [{
                "kind": "profile",
                "path": format!("profiles/{id}/profile.toml"),
                "bytes": 1,
                "digest": {
                    "blake3": digest(id, "config", image_revision),
                    "sha256": digest(id, "config", image_revision),
                }
            }],
            "images": [
                image(id, "kernel", "vmlinuz", image_revision),
                image(id, "initrd", "initrd.img", image_revision),
                image(id, "rootfs", "rootfs.erofs", image_revision),
            ],
            "evidence": [{
                "kind": "obom",
                "bytes": 1,
                "digest": {
                    "blake3": digest(id, "obom", image_revision),
                    "sha256": digest(id, "obom", image_revision),
                }
            }]
        }]
    })
}

/// A channel graph carrying an arbitrary set of profiles.
fn channel(profiles: &[(&str, &str, &str)]) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    for (id, revision, image_revision) in profiles {
        map.insert((*id).to_string(), profile(id, revision, image_revision));
    }
    serde_json::json!({ "profiles": serde_json::Value::Object(map) })
}

/// The scenario that breaks a single-pointer model: two profiles running, one
/// updated, a third added.
fn wheel_breaker() -> serde_json::Value {
    channel(&[
        ("profile1", "2030.0101.1", "2030.0101.10"),
        ("profile2", "2030.0202.2", "2030.0202.20"),
        ("profile3", "2030.0303.3", "2030.0303.30"),
    ])
}

fn kernel_digest_of(graph: &serde_json::Value, id: &str) -> String {
    graph["profiles"][id]["architectures"][0]["images"]
        .as_array()
        .expect("images")
        .iter()
        .find(|image| image["kind"] == "kernel")
        .expect("kernel image")["digest"]["blake3"]
        .as_str()
        .expect("blake3")
        .to_string()
}

fn write_blob(dir: &Path, name: &str, hash: &str) -> std::path::PathBuf {
    let path = dir.join(hash_filename(name, hash));
    fs::create_dir_all(dir).expect("asset dir");
    fs::write(&path, b"blob").expect("write blob");
    path
}

// ---------------------------------------------------------------------------
// Identity: blobs are content-addressed and shared, as Docker layers are.
// ---------------------------------------------------------------------------

#[test]
fn a_blob_is_named_by_its_digest_not_by_its_profile() {
    let graph = wheel_breaker();
    let one = kernel_digest_of(&graph, "profile1");
    let two = kernel_digest_of(&graph, "profile2");

    assert_ne!(one, two, "fixture must give profiles distinct kernels");
    assert_ne!(
        hash_filename("vmlinuz", &one),
        hash_filename("vmlinuz", &two),
        "distinct kernels must occupy distinct files"
    );
}

#[test]
fn profiles_pinning_one_kernel_share_a_single_file() {
    // Two profiles, independently authored, that happen to pin the same kernel.
    // Docker stores such a layer once; so must Capsem, and the storage name is
    // what decides it.
    let shared = digest("shared", "kernel", "r1");
    let mut graph = channel(&[
        ("profile1", "2030.0101.1", "2030.0101.10"),
        ("profile2", "2030.0202.2", "2030.0202.20"),
    ]);
    for id in ["profile1", "profile2"] {
        graph["profiles"][id]["architectures"][0]["images"][0]["digest"]["blake3"] =
            serde_json::json!(shared.clone());
    }

    let one = kernel_digest_of(&graph, "profile1");
    let two = kernel_digest_of(&graph, "profile2");

    assert_eq!(one, two, "fixture pins the same kernel in both profiles");
    assert_eq!(
        hash_filename("vmlinuz", &one),
        hash_filename("vmlinuz", &two),
        "an identical kernel must occupy one file, however many profiles pin it"
    );
}

// ---------------------------------------------------------------------------
// Coexistence: every profile in the channel is real, none is discarded.
// ---------------------------------------------------------------------------

#[test]
fn every_profile_in_the_channel_survives_parsing() {
    let state = release_graph_profile_state(&wheel_breaker()).expect("graph parses");

    assert_eq!(
        state.profiles.keys().cloned().collect::<Vec<_>>(),
        ["profile1", "profile2", "profile3"],
        "a channel with three profiles must yield three profiles"
    );
}

#[test]
fn each_profile_keeps_its_own_revision() {
    let state = release_graph_profile_state(&wheel_breaker()).expect("graph parses");

    assert_eq!(state.profiles["profile1"].revision, "2030.0101.1");
    assert_eq!(state.profiles["profile2"].revision, "2030.0202.2");
    assert_eq!(state.profiles["profile3"].revision, "2030.0303.3");
}

#[test]
fn no_channel_wide_asset_version_may_stand_in_for_three_profiles() {
    let manifest =
        ManifestV2::from_json(&wheel_breaker().to_string()).expect("channel graph is accepted");
    let arch = host_manifest_arch();

    // The flat view keeps one release. Three profiles with three image
    // revisions cannot be represented by it, so this is the assertion that
    // fails until assets are addressed per profile.
    let releases: BTreeMap<_, _> = manifest
        .assets
        .releases
        .iter()
        .map(|(version, release)| (version.clone(), release.arches.contains_key(arch)))
        .collect();

    assert_eq!(
        releases.len(),
        3,
        "three profiles carry three independent image sets, got {releases:?}"
    );
}

#[test]
fn a_profiles_kernel_is_reachable_by_its_own_name() {
    let graph = wheel_breaker();
    let manifest = ManifestV2::from_json(&graph.to_string()).expect("channel graph is accepted");
    let arch = host_manifest_arch();

    for id in ["profile1", "profile2", "profile3"] {
        let wanted = kernel_digest_of(&graph, id);
        let found = manifest.assets.releases.values().any(|release| {
            release
                .arches
                .get(arch)
                .and_then(|assets| assets.get("vmlinuz"))
                .is_some_and(|entry| entry.hash == wanted)
        });
        assert!(found, "{id}'s kernel must be reachable, not discarded");
    }
}

#[test]
fn every_profiles_boot_hashes_are_retrievable_without_a_global_pointer() {
    let graph = wheel_breaker();
    let manifest = ManifestV2::from_json(&graph.to_string()).expect("channel graph is accepted");
    let arch = host_manifest_arch();

    // Booting profileN must be verifiable against profileN's own kernel. A
    // single channel-wide answer can only ever match one of the three, which is
    // what failed the 1.6 release gate: the path came from the booting profile
    // and the hash came from the global pointer.
    for id in ["profile1", "profile2", "profile3"] {
        let wanted = kernel_digest_of(&graph, id);
        let verifiable = manifest.assets.releases.values().any(|release| {
            release
                .arches
                .get(arch)
                .and_then(|assets| assets.get("vmlinuz"))
                .is_some_and(|entry| entry.hash == wanted)
        });
        assert!(
            verifiable,
            "{id} must be bootable against its own kernel hash, not another profile's"
        );
    }

    // And the global pointer must not be the thing anyone verifies against: it
    // answers for exactly one profile out of three.
    let global = manifest
        .expected_hashes_current(arch)
        .expect("a flat manifest still answers for its default");
    let profiles_it_covers = ["profile1", "profile2", "profile3"]
        .into_iter()
        .filter(|id| kernel_digest_of(&graph, id) == global.kernel)
        .count();
    assert_eq!(
        profiles_it_covers, 1,
        "the global answer covers one profile, so boot must not use it"
    );
}

// ---------------------------------------------------------------------------
// Tags: several revisions of one profile coexist.
// ---------------------------------------------------------------------------

#[test]
fn two_revisions_of_one_profile_occupy_two_files() {
    let before = digest("profile2", "kernel", "2030.0202.20");
    let after = digest("profile2", "kernel", "2030.0303.30");

    assert_ne!(before, after);
    assert_ne!(
        hash_filename("vmlinuz", &before),
        hash_filename("vmlinuz", &after),
        "an updated profile must not overwrite the revision a VM is running"
    );
}

#[test]
fn updating_one_profile_leaves_the_others_addressable() {
    let before = wheel_breaker();
    let after = channel(&[
        ("profile1", "2030.0101.1", "2030.0101.10"),
        ("profile2", "2030.0404.4", "2030.0404.40"),
        ("profile3", "2030.0303.3", "2030.0303.30"),
    ]);

    let state = release_graph_profile_state(&after).expect("graph parses");
    assert_eq!(state.profiles["profile2"].revision, "2030.0404.4");
    assert_eq!(
        state.profiles["profile1"].revision,
        release_graph_profile_state(&before).expect("graph parses").profiles["profile1"].revision,
        "profile1 must be untouched by a profile2 update"
    );
}

// ---------------------------------------------------------------------------
// Lifetime: reference-counted, never "not in the current set".
// ---------------------------------------------------------------------------

/// The manifest's own reference set must cover every profile.
///
/// The daemon additionally preserves the profile catalog and persistent-VM boot
/// pins (see `cleanup_unused_assets_preserving` at its call site in
/// capsem-service), so a manifest that dropped profiles was covered in practice.
/// This asserts the manifest does not depend on that: a reference set derived
/// from the manifest alone must already name every installed profile's assets,
/// so cleanup is safe for any caller rather than only the one with the extra
/// preserve list.
#[test]
fn a_manifest_alone_references_every_profiles_kernel() {
    let graph = wheel_breaker();
    let manifest = ManifestV2::from_json(&graph.to_string()).expect("channel graph is accepted");
    let temp = tempfile::tempdir().expect("tempdir");
    let arch_dir = temp.path().join(host_manifest_arch());

    let mut expected = Vec::new();
    for id in ["profile1", "profile2", "profile3"] {
        expected.push(write_blob(&arch_dir, "vmlinuz", &kernel_digest_of(&graph, id)));
    }

    cleanup_unused_assets(&arch_dir, &manifest).expect("cleanup runs");

    for path in expected {
        assert!(
            path.exists(),
            "an installed profile's kernel was deleted as unreferenced: {}",
            path.display()
        );
    }
}

#[test]
fn refresh_collects_a_kernel_no_profile_references() {
    let graph = wheel_breaker();
    let manifest = ManifestV2::from_json(&graph.to_string()).expect("channel graph is accepted");
    let temp = tempfile::tempdir().expect("tempdir");
    let arch_dir = temp.path().join(host_manifest_arch());

    let orphan = write_blob(&arch_dir, "vmlinuz", &digest("removed", "kernel", "old"));
    for id in ["profile1", "profile2", "profile3"] {
        write_blob(&arch_dir, "vmlinuz", &kernel_digest_of(&graph, id));
    }

    cleanup_unused_assets(&arch_dir, &manifest).expect("cleanup runs");

    assert!(
        !orphan.exists(),
        "a blob no profile references must be collected"
    );
}

/// Overwrite a profile's kernel digest so two profiles can be made to share one.
fn pin_kernel(graph: &mut serde_json::Value, id: &str, hash: &str) {
    for field in ["blake3", "sha256"] {
        graph["profiles"][id]["architectures"][0]["images"][0]["digest"][field] =
            serde_json::json!(hash);
    }
}

#[test]
fn removing_a_profile_keeps_a_kernel_another_profile_shares() {
    // profile1 and profile2 both pin the same kernel, as two Docker images share
    // a layer. Remove profile2: the blob is still referenced, so it must stay.
    let shared = digest("shared", "kernel", "r1");
    let mut before = channel(&[
        ("profile1", "2030.0101.1", "2030.0101.10"),
        ("profile2", "2030.0202.2", "2030.0202.20"),
    ]);
    pin_kernel(&mut before, "profile1", &shared);
    pin_kernel(&mut before, "profile2", &shared);

    let temp = tempfile::tempdir().expect("tempdir");
    let arch_dir = temp.path().join(host_manifest_arch());
    let path = write_blob(&arch_dir, "vmlinuz", &shared);

    // Both present: the blob is shared, one file for two profiles.
    let installed =
        ManifestV2::from_json(&before.to_string()).expect("channel graph is accepted");
    cleanup_unused_assets(&arch_dir, &installed).expect("cleanup runs");
    assert!(path.exists(), "a shared blob must survive while both pin it");

    // profile2 removed from the channel; profile1 still pins the same kernel.
    let mut after = channel(&[("profile1", "2030.0101.1", "2030.0101.10")]);
    pin_kernel(&mut after, "profile1", &shared);
    let remaining = ManifestV2::from_json(&after.to_string()).expect("channel graph is accepted");

    cleanup_unused_assets(&arch_dir, &remaining).expect("cleanup runs");

    assert!(
        path.exists(),
        "removing one sharer must not collect a blob another profile still pins"
    );
}

#[test]
fn removing_the_last_profile_that_pinned_a_kernel_collects_it() {
    // The other half of reference counting: once nothing pins it, it goes.
    // Without this, the test above would pass on a cleanup that never deletes.
    let lonely = digest("profile2", "kernel", "only");
    let mut before = channel(&[
        ("profile1", "2030.0101.1", "2030.0101.10"),
        ("profile2", "2030.0202.2", "2030.0202.20"),
    ]);
    pin_kernel(&mut before, "profile2", &lonely);

    let temp = tempfile::tempdir().expect("tempdir");
    let arch_dir = temp.path().join(host_manifest_arch());
    let path = write_blob(&arch_dir, "vmlinuz", &lonely);
    // profile1's own kernel, so the directory is not left empty.
    write_blob(&arch_dir, "vmlinuz", &kernel_digest_of(&before, "profile1"));

    let after = channel(&[("profile1", "2030.0101.1", "2030.0101.10")]);
    let remaining = ManifestV2::from_json(&after.to_string()).expect("channel graph is accepted");

    cleanup_unused_assets(&arch_dir, &remaining).expect("cleanup runs");

    assert!(
        !path.exists(),
        "a blob no surviving profile pins must be collected, or nothing ever is"
    );
}

// ---------------------------------------------------------------------------
// Agnosticism: the model must not privilege a count, a name, or an order.
// ---------------------------------------------------------------------------

#[test]
fn a_single_profile_channel_is_not_a_special_case() {
    let state =
        release_graph_profile_state(&channel(&[("only", "2030.0101.1", "2030.0101.10")]))
            .expect("graph parses");

    assert_eq!(state.profiles.len(), 1);
    assert_eq!(state.profiles["only"].revision, "2030.0101.1");
}

#[test]
fn ten_profiles_behave_like_two() {
    let owned: Vec<(String, String, String)> = (0..10)
        .map(|index| {
            (
                format!("profile{index}"),
                format!("2030.0101.{index}"),
                format!("2030.0101.{}", index * 10),
            )
        })
        .collect();
    let borrowed: Vec<(&str, &str, &str)> = owned
        .iter()
        .map(|(id, revision, images)| (id.as_str(), revision.as_str(), images.as_str()))
        .collect();

    let state = release_graph_profile_state(&channel(&borrowed)).expect("graph parses");

    assert_eq!(state.profiles.len(), 10, "profile count must not be a limit");
}

#[test]
fn no_profile_name_is_privileged() {
    // A model that prefers a profile literally called "default" answers
    // differently for channels that have one, which is a name deciding
    // behaviour.
    let without = ManifestV2::from_json(&channel(&[
        ("alpha", "2030.0101.1", "2030.0101.10"),
        ("omega", "2030.0202.2", "2030.0202.20"),
    ])
    .to_string())
    .expect("channel graph is accepted");
    let with = ManifestV2::from_json(&channel(&[
        ("default", "2030.0101.1", "2030.0101.10"),
        ("omega", "2030.0202.2", "2030.0202.20"),
    ])
    .to_string())
    .expect("channel graph is accepted");

    assert_eq!(
        without.assets.releases.len(),
        with.assets.releases.len(),
        "naming a profile \"default\" must not change how many asset sets exist"
    );
}

#[test]
fn alphabetical_order_does_not_decide_which_profile_boots() {
    let graph = channel(&[
        ("aaa", "2030.0101.1", "2030.0101.10"),
        ("zzz", "2030.0202.2", "2030.0202.20"),
    ]);
    let manifest = ManifestV2::from_json(&graph.to_string()).expect("channel graph is accepted");
    let arch = host_manifest_arch();

    let last = kernel_digest_of(&graph, "zzz");
    let found = manifest.assets.releases.values().any(|release| {
        release
            .arches
            .get(arch)
            .and_then(|assets| assets.get("vmlinuz"))
            .is_some_and(|entry| entry.hash == last)
    });

    assert!(
        found,
        "the alphabetically last profile must be as real as the first"
    );
}

#[test]
fn hyphenated_and_plain_names_are_equally_valid() {
    let state = release_graph_profile_state(&channel(&[
        ("co-work", "2030.0101.1", "2030.0101.10"),
        ("code", "2030.0202.2", "2030.0202.20"),
    ]))
    .expect("graph parses");

    // "co-work" sorts before "code" because '-' precedes 'd'. Nothing may
    // depend on that.
    assert_eq!(state.profiles.len(), 2);
    assert_eq!(state.profiles["co-work"].revision, "2030.0101.1");
    assert_eq!(state.profiles["code"].revision, "2030.0202.2");
}

// ---------------------------------------------------------------------------
// Add and remove, the operations the wheel has to survive.
// ---------------------------------------------------------------------------

#[test]
fn adding_a_profile_does_not_disturb_the_existing_ones() {
    let before = release_graph_profile_state(&channel(&[
        ("profile1", "2030.0101.1", "2030.0101.10"),
        ("profile2", "2030.0202.2", "2030.0202.20"),
    ]))
    .expect("graph parses");
    let after = release_graph_profile_state(&wheel_breaker()).expect("graph parses");

    assert_eq!(
        before.profiles["profile1"].revision,
        after.profiles["profile1"].revision
    );
    assert_eq!(
        before.profiles["profile2"].revision,
        after.profiles["profile2"].revision
    );
    assert!(after.profiles.contains_key("profile3"));
}

#[test]
fn removing_a_profile_does_not_disturb_the_survivors() {
    let before = release_graph_profile_state(&wheel_breaker()).expect("graph parses");
    let after = release_graph_profile_state(&channel(&[
        ("profile1", "2030.0101.1", "2030.0101.10"),
        ("profile3", "2030.0303.3", "2030.0303.30"),
    ]))
    .expect("graph parses");

    assert_eq!(
        before.profiles["profile1"].revision,
        after.profiles["profile1"].revision
    );
    assert_eq!(
        before.profiles["profile3"].revision,
        after.profiles["profile3"].revision
    );
    assert!(!after.profiles.contains_key("profile2"));
}

#[test]
fn a_revoked_profile_is_not_silently_promoted_over_a_current_one() {
    let mut graph = wheel_breaker();
    graph["profiles"]["profile1"]["status"] = serde_json::json!("revoked");
    let manifest = ManifestV2::from_json(&graph.to_string()).expect("channel graph is accepted");
    let arch = host_manifest_arch();

    let revoked = kernel_digest_of(&graph, "profile1");
    let serves_revoked = manifest.assets.releases.values().any(|release| {
        release
            .arches
            .get(arch)
            .and_then(|assets| assets.get("vmlinuz"))
            .is_some_and(|entry| entry.hash == revoked)
    });

    assert!(
        !serves_revoked,
        "a revoked profile's kernel must not be served to anyone"
    );
}
