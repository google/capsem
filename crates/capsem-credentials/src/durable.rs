use std::collections::HashMap;
use std::fs::File;
use std::path::{Path, PathBuf};

use tracing::warn;

use crate::provider::credential_provider_from_str;
use crate::{credential_store_account, CredentialProvider};

pub const STORE_PATH_ENV: &str = "CAPSEM_CREDENTIAL_STORE_PATH";

#[cfg(test)]
mod tests;

pub(crate) fn backend_name() -> &'static str {
    if store_path_override().is_some() {
        return "disk_override";
    }
    "disk"
}

pub(crate) fn write(provider: CredentialProvider, credential_ref: &str, raw_value: &str) -> Result<(), String> {
    disk_store_write(
        store_path_override().unwrap_or_else(default_store_path).as_path(),
        provider,
        credential_ref,
        raw_value,
    )
}

pub(crate) fn read(provider: CredentialProvider, credential_ref: &str) -> Result<String, String> {
    disk_store_read(
        store_path_override().unwrap_or_else(default_store_path).as_path(),
        provider,
        credential_ref,
    )
}

pub(crate) fn hydrate() -> Result<Vec<(CredentialProvider, String, String)>, String> {
    disk_store_hydrate(store_path_override().unwrap_or_else(default_store_path).as_path())
}

fn store_path_override() -> Option<PathBuf> {
    std::env::var_os(STORE_PATH_ENV)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

fn default_store_path() -> PathBuf {
    capsem_foundation::paths::capsem_home()
        .join("credentials")
        .join("credential-store.json")
}

fn disk_store_write(
    path: &Path,
    provider: CredentialProvider,
    credential_ref: &str,
    raw_value: &str,
) -> Result<(), String> {
    let _guard = StoreLock::exclusive(path)?;
    let mut map = disk_store_load(path)?;
    map.insert(
        credential_store_account(provider, credential_ref),
        raw_value.to_string(),
    );
    let json =
        serde_json::to_string_pretty(&map).map_err(|error| format!("serialize credential disk store: {error}"))?;
    write_secret_file(path, json.as_bytes())
}

fn disk_store_read(path: &Path, provider: CredentialProvider, credential_ref: &str) -> Result<String, String> {
    let _guard = StoreLock::shared(path)?;
    let map = disk_store_load(path)?;
    let account = credential_store_account(provider, credential_ref);
    map.get(&account)
        .cloned()
        .ok_or_else(|| format!("credential reference not found in disk store: {account}"))
}

fn disk_store_hydrate(path: &Path) -> Result<Vec<(CredentialProvider, String, String)>, String> {
    let _guard = StoreLock::shared(path)?;
    let map = disk_store_load(path)?;
    let mut entries = Vec::new();
    for (account, raw_value) in map {
        let Some((provider, credential_ref)) = parse_store_account(&account) else {
            warn!(account, "credential store: ignoring malformed disk account");
            continue;
        };
        entries.push((provider, credential_ref.to_string(), raw_value));
    }
    Ok(entries)
}

/// Cross-process lock on the store, held for one read-modify-write.
///
/// Every `capsem-process` on the host captures into the same file, and a
/// capture is load -> insert -> write-temp -> rename. A process-local mutex
/// orders that inside one process only; two processes interleave and the
/// later rename drops everything the earlier one added. The lock is an
/// `flock(2)` on a sibling file (never the store itself: the store is renamed
/// into place, so a lock on its inode would not survive the swap) and is
/// taken before the load, so the map written is the map that was read.
struct StoreLock {
    _file: File,
}

impl StoreLock {
    fn exclusive(store: &Path) -> Result<Self, String> {
        let file = Self::open(store)?;
        file.lock().map_err(|error| format!("lock credential store: {error}"))?;
        Ok(Self { _file: file })
    }

    fn shared(store: &Path) -> Result<Self, String> {
        let file = Self::open(store)?;
        file.lock_shared()
            .map_err(|error| format!("lock credential store for reading: {error}"))?;
        Ok(Self { _file: file })
    }

    fn open(store: &Path) -> Result<File, String> {
        let parent = store
            .parent()
            .ok_or_else(|| "credential store path has no parent directory".to_string())?;
        std::fs::create_dir_all(parent).map_err(|error| format!("create credential store dir: {error}"))?;
        let file_name = store
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("credential-store.json");
        let mut options = std::fs::OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        options
            .open(parent.join(format!(".{file_name}.lock")))
            .map_err(|error| format!("open credential store lock: {error}"))
    }
}

fn disk_store_load(path: &Path) -> Result<HashMap<String, String>, String> {
    if !path.exists() {
        return Ok(HashMap::new());
    }
    let text = std::fs::read_to_string(path).map_err(|error| format!("read credential disk store: {error}"))?;
    if text.trim().is_empty() {
        return Ok(HashMap::new());
    }
    serde_json::from_str(&text).map_err(|error| format!("parse credential disk store: {error}"))
}

fn parse_store_account(account: &str) -> Option<(CredentialProvider, &str)> {
    let (provider, credential_ref) = account.split_once(':')?;
    Some((credential_provider_from_str(provider)?, credential_ref))
}

/// Write the plaintext credential store owner-only and atomically.
///
/// The store holds every provider's raw secret, so it must never exist even
/// briefly as a world-readable file. The previous `fs::write` + `chmod` left a
/// 0644 window under the default umask. Here the bytes go into a sibling temp
/// created 0600, then a rename swings it into place -- the target is never
/// observable at looser permissions or as a partial file.
fn write_secret_file(path: &Path, data: &[u8]) -> Result<(), String> {
    use std::io::Write;

    let parent = path
        .parent()
        .ok_or_else(|| "credential store path has no parent directory".to_string())?;
    let file_name = path
        .file_name()
        .unwrap_or_else(|| std::ffi::OsStr::new("credential-store.json"));
    let prefix = format!(".{}.tmp.", file_name.to_string_lossy());
    let mut file = tempfile::Builder::new()
        .prefix(&prefix)
        .tempfile_in(parent)
        .map_err(|error| format!("create credential store temp: {error}"))?;
    restrict_secret_file(file.as_file())?;
    file.write_all(data)
        .map_err(|error| format!("write credential store temp: {error}"))?;
    file.as_file()
        .sync_all()
        .map_err(|error| format!("sync credential store temp: {error}"))?;
    file.persist(path)
        .map_err(|error| format!("rename credential store into place: {}", error.error))?;
    sync_secret_parent(parent)
}

#[cfg(unix)]
fn restrict_secret_file(file: &std::fs::File) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;

    file.set_permissions(std::fs::Permissions::from_mode(0o600))
        .map_err(|error| format!("restrict credential store temp: {error}"))
}

#[cfg(not(unix))]
fn restrict_secret_file(_file: &std::fs::File) -> Result<(), String> {
    Ok(())
}

#[cfg(unix)]
fn sync_secret_parent(parent: &Path) -> Result<(), String> {
    std::fs::File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| format!("sync credential store directory: {error}"))
}

#[cfg(not(unix))]
fn sync_secret_parent(_parent: &Path) -> Result<(), String> {
    Ok(())
}
