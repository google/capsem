use super::*;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

#[cfg(unix)]
fn mode_of(path: &std::path::Path) -> u32 {
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

#[cfg(unix)]
#[test]
fn write_secret_file_creates_owner_only() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("credential-store.json");
    write_secret_file(&path, b"{}").unwrap();
    assert_eq!(
        mode_of(&path),
        0o600,
        "plaintext secret store must be created owner-only, never briefly 0644"
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"{}");
}

#[cfg(unix)]
#[test]
fn write_secret_file_downgrades_permissive_existing_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("credential-store.json");
    std::fs::write(&path, b"stale").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666)).unwrap();
    write_secret_file(&path, b"{\"k\":\"v\"}").unwrap();
    assert_eq!(mode_of(&path), 0o600);
    assert_eq!(std::fs::read(&path).unwrap(), b"{\"k\":\"v\"}");
}

#[cfg(unix)]
#[test]
fn write_secret_file_leaves_no_temp_sibling() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("credential-store.json");
    write_secret_file(&path, b"{}").unwrap();
    let leftovers: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|name| name != "credential-store.json")
        .collect();
    assert!(
        leftovers.is_empty(),
        "atomic write must not leave temp files: {leftovers:?}"
    );
}

/// Set by [`two_processes_capturing_concurrently_keep_every_entry`] when it
/// re-executes this test binary as a writer child: `<store path>|<prefix>`.
const CHILD_WRITER_ENV: &str = "CAPSEM_TEST_DURABLE_STORE_CHILD";
const CHILD_WRITER_ENTRIES: usize = 300;

/// The child half of the cross-process test. A no-op unless spawned with
/// [`CHILD_WRITER_ENV`], so the normal test run only registers it.
#[test]
fn durable_store_child_writer() {
    let Ok(spec) = std::env::var(CHILD_WRITER_ENV) else {
        return;
    };
    let (path, prefix) = spec.split_once('|').expect("child spec is `<path>|<prefix>`");
    for index in 0..CHILD_WRITER_ENTRIES {
        disk_store_write(
            Path::new(path),
            CredentialProvider::OpenAi,
            &format!("{prefix}-{index}"),
            &format!("secret-{prefix}-{index}"),
        )
        .unwrap();
    }
}

fn spawn_child_writer(store: &Path, prefix: &str) -> std::process::Child {
    std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "durable::tests::durable_store_child_writer", "--nocapture"])
        .env(CHILD_WRITER_ENV, format!("{}|{prefix}", store.display()))
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn child writer")
}

/// Two `capsem-process` instances share one credential store. Each capture is
/// load -> insert -> write-temp -> rename; a process-local mutex orders that
/// inside one process only, so two processes interleave and the second
/// rename silently drops everything the first one added (an auditor's
/// simulation kept 318 of 600). The store must be locked across processes.
#[test]
fn two_processes_capturing_concurrently_keep_every_entry() {
    let dir = tempfile::tempdir().unwrap();
    let store = dir.path().join("credential-store.json");
    let mut children = vec![spawn_child_writer(&store, "a"), spawn_child_writer(&store, "b")];
    for child in &mut children {
        let output = child.wait_with_output_ref();
        assert!(output.0.success(), "child writer failed: {}", output.1);
    }

    let map = disk_store_load(&store).unwrap();
    let mut missing = Vec::new();
    for prefix in ["a", "b"] {
        for index in 0..CHILD_WRITER_ENTRIES {
            let account = credential_store_account(CredentialProvider::OpenAi, &format!("{prefix}-{index}"));
            if map.get(&account).map(String::as_str) != Some(&format!("secret-{prefix}-{index}")) {
                missing.push(account);
            }
        }
    }
    assert!(
        missing.is_empty(),
        "{} of {} captures lost across processes; first missing: {:?}",
        missing.len(),
        2 * CHILD_WRITER_ENTRIES,
        missing.first()
    );
    let leftovers: Vec<String> = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|name| name.contains(".tmp."))
        .collect();
    assert!(leftovers.is_empty(), "no temp files may survive: {leftovers:?}");
}

trait WaitOutputRef {
    fn wait_with_output_ref(&mut self) -> (std::process::ExitStatus, String);
}

impl WaitOutputRef for std::process::Child {
    fn wait_with_output_ref(&mut self) -> (std::process::ExitStatus, String) {
        use std::io::Read;
        let mut stderr = String::new();
        if let Some(mut pipe) = self.stderr.take() {
            let _ = pipe.read_to_string(&mut stderr);
        }
        (self.wait().unwrap(), stderr)
    }
}

#[cfg(unix)]
#[test]
fn store_lock_file_is_owner_only_and_never_the_store_itself() {
    let dir = tempfile::tempdir().unwrap();
    let store = dir.path().join("nested").join("credential-store.json");
    disk_store_write(&store, CredentialProvider::Github, "ref-1", "secret-1").unwrap();
    let lock = store.parent().unwrap().join(".credential-store.json.lock");
    assert!(
        lock.exists(),
        "the lock is a sibling file that survives the store's rename"
    );
    assert_eq!(mode_of(&lock), 0o600);
    assert_eq!(mode_of(&store), 0o600);
    assert_eq!(
        disk_store_read(&store, CredentialProvider::Github, "ref-1").unwrap(),
        "secret-1"
    );
}
