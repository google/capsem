//! Profiles that ship with a new binary are judged by that binary (#288).
//!
//! 0.6.3 staged 0.6.4's profiles with its own `deny_unknown_fields` parser
//! before installing 0.6.4, refused a field only 0.6.4 knew, and failed every
//! automatic update. These tests hold the replacement: with a binary upgrade in
//! the plan the old parser stays out of it, the newly installed binary parses
//! the exact staged tree, and nothing activates until it has said yes.

use super::super::runtime_contract_tests::{
    assert_profile_uses_release_manifest_pins, profile_stage_plan, staged_profile_fixture_with, update_plan_check,
};
use super::*;

/// A key no released `ProfileConfigFile` has, standing in for the next
/// `default_for`.
const FIELD_FROM_A_LATER_RELEASE: &str = "field_from_a_later_release = true\n";
const SELECTED_BINARY: &str = "99.0.0";
const INSTALLER_BYTES: &[u8] = b"verified-native-package";

fn upgrade_plan() -> VerifiedUpdatePlan {
    VerifiedUpdatePlan {
        installed_binary: env!("CARGO_PKG_VERSION").to_string(),
        selected_binary: SELECTED_BINARY.to_string(),
        steps: vec![UpdatePlanStep::Binary, UpdatePlanStep::Profiles],
    }
}

/// An update check for `upgrade_plan`, with its native package already in the
/// verified installer cache so staging needs no network.
fn upgrade_check(capsem_home: &Path, body: &[u8], source: &str) -> UpdateCheck {
    let mut check = update_plan_check(true, true, true, true);
    check.latest_version = Some(SELECTED_BINARY.to_string());
    check.source = Some(source.to_string());
    check.channel_hash = Some(channel_payload_hash(body));
    let installer = check.binary_installer.as_mut().unwrap();
    installer.sha256 = sha256_hex(INSTALLER_BYTES);
    installer.blake3 = blake3::hash(INSTALLER_BYTES).to_hex().to_string();
    installer.size = INSTALLER_BYTES.len() as u64;
    let cached = binary_installer_cache_path_at(capsem_home, installer).unwrap();
    std::fs::create_dir_all(cached.parent().unwrap()).unwrap();
    std::fs::write(cached, INSTALLER_BYTES).unwrap();
    check
}

fn profile_only_check(body: &[u8], source: &str) -> UpdateCheck {
    let mut check = update_plan_check(false, true, true, true);
    check.latest_version = Some(env!("CARGO_PKG_VERSION").to_string());
    check.source = Some(source.to_string());
    check.channel_hash = Some(channel_payload_hash(body));
    check
}

struct Install {
    _temp: tempfile::TempDir,
    capsem_home: PathBuf,
    release_dir: PathBuf,
    installed_assets: PathBuf,
    installed_profile: PathBuf,
}

fn install_with_previous_release() -> Install {
    let temp = tempfile::tempdir().unwrap();
    let capsem_home = temp.path().join("home");
    let installed_assets = capsem_home.join("assets");
    let installed_profile = capsem_home.join("profiles/code/profile.toml");
    std::fs::create_dir_all(&installed_assets).unwrap();
    std::fs::create_dir_all(installed_profile.parent().unwrap()).unwrap();
    std::fs::write(installed_assets.join("manifest.json"), b"installed-manifest").unwrap();
    std::fs::write(&installed_profile, b"installed-profile").unwrap();
    Install {
        release_dir: temp.path().join("release"),
        _temp: temp,
        capsem_home,
        installed_assets,
        installed_profile,
    }
}

async fn stage_upgrade_carrying_a_later_field(install: &Install) -> (StagedUpdate, UpdateCheck) {
    let (body, source, _) = staged_profile_fixture_with(&install.release_dir, false, FIELD_FROM_A_LATER_RELEASE);
    let check = upgrade_check(&install.capsem_home, &body, &source);
    let staged = stage_verified_update_at(&install.capsem_home, &upgrade_plan(), &check, &body)
        .await
        .expect("an upgrade must stage profiles its installed parser cannot read");
    (staged, check)
}

fn activate(install: &Install, staged: &StagedUpdate, check: &UpdateCheck) -> Result<()> {
    activate_staged_update_at(
        &install.capsem_home,
        &install.installed_assets,
        staged,
        check,
        &ChannelTransition::Preserve,
    )
}

fn assert_previous_release_still_installed(install: &Install) {
    assert_eq!(std::fs::read(&install.installed_profile).unwrap(), b"installed-profile");
    assert_eq!(
        std::fs::read(install.installed_assets.join("manifest.json")).unwrap(),
        b"installed-manifest"
    );
}

/// A stand-in for the newly installed `capsem`: a script that records its
/// arguments beside itself, then runs `body`.
fn fake_new_binary(dir: &Path, body: &str) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join("capsem");
    let argv = dir.join("argv");
    std::fs::write(
        &path,
        format!(
            "#!/bin/sh\nfor arg in \"$@\"; do printf '%s\\n' \"$arg\"; done > '{}'\n{body}\n",
            argv.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

fn recorded_argv(validator: &Path) -> Vec<String> {
    std::fs::read_to_string(validator.with_file_name("argv"))
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect()
}

fn awaiting(staged: &StagedUpdate) -> bool {
    staged.profiles.as_ref().unwrap().validation == StagedProfileValidation::AwaitingNewBinary
}

#[test]
fn a_profile_only_plan_keeps_the_strict_in_process_parse() {
    assert_eq!(
        profile_stage_plan().profile_validation(),
        StagedProfileValidation::InProcess
    );
}

#[test]
fn a_binary_upgrade_with_profiles_defers_the_parse_to_the_new_binary() {
    assert_eq!(
        upgrade_plan().profile_validation(),
        StagedProfileValidation::AwaitingNewBinary
    );
}

/// A channel switch can install an older binary, which may predate
/// `--validate-profile-catalog` entirely. Profiles published for an older
/// binary only use fields this one already knows, so it keeps judging them.
#[test]
fn a_binary_downgrade_keeps_the_strict_in_process_parse() {
    let mut plan = upgrade_plan();
    plan.installed_binary = SELECTED_BINARY.to_string();
    plan.selected_binary = "1.0.0".to_string();
    assert_eq!(plan.profile_validation(), StagedProfileValidation::InProcess);
}

#[test]
fn a_binary_only_plan_has_no_profiles_to_defer() {
    let mut plan = upgrade_plan();
    plan.steps = vec![UpdatePlanStep::Binary];
    assert_eq!(plan.profile_validation(), StagedProfileValidation::InProcess);
}

#[tokio::test]
async fn without_a_binary_step_the_installed_parser_still_refuses_an_unknown_field() {
    let install = install_with_previous_release();
    let (body, source, _) = staged_profile_fixture_with(&install.release_dir, false, FIELD_FROM_A_LATER_RELEASE);
    let error = stage_verified_update_at(
        &install.capsem_home,
        &profile_stage_plan(),
        &profile_only_check(&body, &source),
        &body,
    )
    .await
    .expect_err("profiles this binary cannot parse must not stage for it");

    assert!(format!("{error:#}").contains("field_from_a_later_release"), "{error:#}");
    assert_previous_release_still_installed(&install);
}

/// The 0.6.3 -> 0.6.4 failure, replayed: the field the installed parser does
/// not know no longer stops the update, and the profile still activates once
/// the new binary accepts it.
#[tokio::test]
async fn with_a_binary_upgrade_an_unknown_field_stages_and_activates_after_the_new_binary_accepts_it() {
    let install = install_with_previous_release();
    let (mut staged, check) = stage_upgrade_carrying_a_later_field(&install).await;
    assert!(awaiting(&staged));

    let bin = tempfile::tempdir().unwrap();
    let validator = fake_new_binary(bin.path(), "exit 0");
    staged
        .profiles
        .as_mut()
        .unwrap()
        .validate_with(&validator, Duration::from_secs(30))
        .await
        .unwrap();
    activate(&install, &staged, &check).unwrap();

    let installed = std::fs::read_to_string(&install.installed_profile).unwrap();
    assert!(installed.contains("field_from_a_later_release"), "{installed}");
    assert_profile_uses_release_manifest_pins(&install.installed_profile, &install.release_dir);
}

#[tokio::test]
async fn the_new_binary_is_asked_about_exactly_the_digest_verified_staged_tree() {
    let install = install_with_previous_release();
    let (mut staged, _) = stage_upgrade_carrying_a_later_field(&install).await;
    let bin = tempfile::tempdir().unwrap();
    let validator = fake_new_binary(bin.path(), "exit 0");
    let profiles = staged.profiles.as_mut().unwrap();
    profiles
        .validate_with(&validator, Duration::from_secs(30))
        .await
        .unwrap();

    let expected_dir = install
        .capsem_home
        .join("updates/candidates")
        .join(channel_payload_hash(&std::fs::read(&staged.manifest_path).unwrap()))
        .join("profiles");
    assert_eq!(staged.profiles.as_ref().unwrap().dir, expected_dir);
    assert_eq!(
        recorded_argv(&validator),
        [
            "update".to_string(),
            "--validate-profile-catalog".to_string(),
            expected_dir.display().to_string(),
        ]
    );
}

#[tokio::test]
async fn activation_refuses_profiles_the_new_binary_has_not_accepted() {
    let install = install_with_previous_release();
    let (staged, check) = stage_upgrade_carrying_a_later_field(&install).await;

    let error = activate(&install, &staged, &check).expect_err("unvalidated profiles must not activate");

    assert!(format!("{error:#}").contains("new capsem"), "{error:#}");
    assert_previous_release_still_installed(&install);
}

#[tokio::test]
async fn a_new_binary_that_refuses_the_catalog_blocks_activation() {
    let install = install_with_previous_release();
    let (mut staged, check) = stage_upgrade_carrying_a_later_field(&install).await;
    let bin = tempfile::tempdir().unwrap();
    let validator = fake_new_binary(bin.path(), "echo 'unknown field `nope`' >&2\nexit 3");

    let error = staged
        .profiles
        .as_mut()
        .unwrap()
        .validate_with(&validator, Duration::from_secs(30))
        .await
        .expect_err("a refusal must fail the update");

    assert!(format!("{error:#}").contains("unknown field `nope`"), "{error:#}");
    assert!(awaiting(&staged));
    activate(&install, &staged, &check).expect_err("refused profiles must not activate");
    assert_previous_release_still_installed(&install);
}

#[tokio::test]
async fn a_missing_new_binary_blocks_activation() {
    let install = install_with_previous_release();
    let (mut staged, check) = stage_upgrade_carrying_a_later_field(&install).await;
    let bin = tempfile::tempdir().unwrap();

    staged
        .profiles
        .as_mut()
        .unwrap()
        .validate_with(&bin.path().join("capsem"), Duration::from_secs(30))
        .await
        .expect_err("no validator is not a yes");

    assert!(awaiting(&staged));
    activate(&install, &staged, &check).expect_err("unvalidated profiles must not activate");
    assert_previous_release_still_installed(&install);
}

#[tokio::test]
async fn a_new_binary_that_never_answers_is_stopped_and_blocks_activation() {
    let install = install_with_previous_release();
    let (mut staged, check) = stage_upgrade_carrying_a_later_field(&install).await;
    let bin = tempfile::tempdir().unwrap();
    let validator = fake_new_binary(bin.path(), "exec sleep 60");

    let started = std::time::Instant::now();
    let error = staged
        .profiles
        .as_mut()
        .unwrap()
        .validate_with(&validator, Duration::from_millis(300))
        .await
        .expect_err("silence is not a yes");

    assert!(started.elapsed() < Duration::from_secs(10), "{:?}", started.elapsed());
    assert!(format!("{error:#}").contains("within"), "{error:#}");
    assert!(awaiting(&staged));
    activate(&install, &staged, &check).expect_err("unvalidated profiles must not activate");
    assert_previous_release_still_installed(&install);
}

/// A yes about a tree the updater did not verify is not a yes about the one
/// it did: a validator that rewrites what it was shown must not get its
/// rewrite activated.
#[tokio::test]
async fn a_tree_changed_while_the_new_binary_validated_it_is_refused() {
    let install = install_with_previous_release();
    let (mut staged, check) = stage_upgrade_carrying_a_later_field(&install).await;
    let bin = tempfile::tempdir().unwrap();
    let validator = fake_new_binary(bin.path(), "echo 'smuggled = true' >> \"$3/code/profile.toml\"\nexit 0");

    let error = staged
        .profiles
        .as_mut()
        .unwrap()
        .validate_with(&validator, Duration::from_secs(30))
        .await
        .expect_err("a changed tree must not count as validated");

    assert!(format!("{error:#}").contains("changed"), "{error:#}");
    assert!(awaiting(&staged));
    activate(&install, &staged, &check).expect_err("unvalidated profiles must not activate");
    assert_previous_release_still_installed(&install);
}

#[tokio::test]
async fn activation_refuses_a_staged_tree_changed_after_validation() {
    let install = install_with_previous_release();
    let (mut staged, check) = stage_upgrade_carrying_a_later_field(&install).await;
    let bin = tempfile::tempdir().unwrap();
    let validator = fake_new_binary(bin.path(), "exit 0");
    let profiles = staged.profiles.as_mut().unwrap();
    profiles
        .validate_with(&validator, Duration::from_secs(30))
        .await
        .unwrap();
    std::fs::create_dir(profiles.dir.join("planted")).unwrap();

    let error = activate(&install, &staged, &check).expect_err("bytes nobody verified must not activate");

    assert!(format!("{error:#}").contains("changed"), "{error:#}");
    assert_previous_release_still_installed(&install);
    assert!(!install.capsem_home.join("profiles/planted").exists());
}

#[tokio::test]
async fn activation_refuses_an_in_process_stage_changed_after_staging() {
    let install = install_with_previous_release();
    let (body, source, _) = staged_profile_fixture_with(&install.release_dir, false, "");
    let check = profile_only_check(&body, &source);
    let staged = stage_verified_update_at(&install.capsem_home, &profile_stage_plan(), &check, &body)
        .await
        .unwrap();
    let staged_profile = staged.profiles.as_ref().unwrap().dir.join("code/profile.toml");
    let mut tampered = std::fs::read(&staged_profile).unwrap();
    tampered.extend_from_slice(b"\n# changed after its digests were checked\n");
    std::fs::write(&staged_profile, tampered).unwrap();

    activate(&install, &staged, &check).expect_err("bytes nobody verified must not activate");

    assert_previous_release_still_installed(&install);
}

#[test]
fn the_staged_tree_digest_covers_names_bytes_and_empty_directories() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path();
    std::fs::create_dir_all(root.join("code")).unwrap();
    std::fs::write(root.join("code/profile.toml"), b"id = \"code\"\n").unwrap();
    let original = profile_tree_digest(root).unwrap();

    std::fs::rename(root.join("code/profile.toml"), root.join("code/profile.tom")).unwrap();
    assert_ne!(profile_tree_digest(root).unwrap(), original, "renamed file");
    std::fs::rename(root.join("code/profile.tom"), root.join("code/profile.toml")).unwrap();
    assert_eq!(profile_tree_digest(root).unwrap(), original);

    std::fs::create_dir(root.join("empty")).unwrap();
    assert_ne!(profile_tree_digest(root).unwrap(), original, "empty directory");
}

#[test]
fn the_staged_tree_digest_refuses_a_symlink() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir(temp.path().join("code")).unwrap();
    std::os::unix::fs::symlink("/etc/passwd", temp.path().join("code/profile.toml")).unwrap();

    profile_tree_digest(temp.path()).expect_err("a staged symlink must not be followed or trusted");
}

#[tokio::test]
async fn nothing_to_validate_is_not_an_error_when_no_binary_was_staged() {
    validate_staged_profiles_with_new_binary(None, &InstallLayout::Development)
        .await
        .unwrap();
}

#[tokio::test]
async fn deferred_profiles_without_an_installed_binary_directory_are_refused() {
    let install = install_with_previous_release();
    let (mut staged, _) = stage_upgrade_carrying_a_later_field(&install).await;

    validate_staged_profiles_with_new_binary(Some(&mut staged), &InstallLayout::Development)
        .await
        .expect_err("no installed binary to ask is not a yes");

    assert!(awaiting(&staged));
}

/// What `capsem update --validate-profile-catalog` runs: this binary's strict
/// parse, which accepts the profiles it knows and names the field it does not.
#[tokio::test]
async fn the_hidden_validator_is_this_binarys_strict_parse() {
    let install = install_with_previous_release();
    let (body, source, _) = staged_profile_fixture_with(&install.release_dir, false, "");
    let check = profile_only_check(&body, &source);
    let known = stage_verified_update_at(&install.capsem_home, &profile_stage_plan(), &check, &body)
        .await
        .unwrap();
    validate_profile_catalog_dir(&known.profiles.as_ref().unwrap().dir).unwrap();

    let later = install_with_previous_release();
    let (unknown, _) = stage_upgrade_carrying_a_later_field(&later).await;
    let error = validate_profile_catalog_dir(&unknown.profiles.as_ref().unwrap().dir)
        .expect_err("an unknown field must be refused");
    assert!(format!("{error:#}").contains("field_from_a_later_release"), "{error:#}");
}
