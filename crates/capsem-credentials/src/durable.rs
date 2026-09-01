use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use tracing::warn;

use crate::provider::credential_provider_from_str;
use crate::{credential_store_account, CredentialProvider};

pub const STORE_PATH_ENV: &str = "CAPSEM_CREDENTIAL_STORE_PATH";

static STORE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

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
    let _guard = store_lock()
        .lock()
        .map_err(|_| "credential disk store lock poisoned".to_string())?;
    let mut map = disk_store_load(path)?;
    map.insert(
        credential_store_account(provider, credential_ref),
        raw_value.to_string(),
    );
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| format!("create credential store dir: {error}"))?;
    }
    let json =
        serde_json::to_string_pretty(&map).map_err(|error| format!("serialize credential disk store: {error}"))?;
    write_secret_file(path, json.as_bytes())
}

fn disk_store_read(path: &Path, provider: CredentialProvider, credential_ref: &str) -> Result<String, String> {
    let _guard = store_lock()
        .lock()
        .map_err(|_| "credential disk store lock poisoned".to_string())?;
    let map = disk_store_load(path)?;
    let account = credential_store_account(provider, credential_ref);
    map.get(&account)
        .cloned()
        .ok_or_else(|| format!("credential reference not found in disk store: {account}"))
}

fn disk_store_hydrate(path: &Path) -> Result<Vec<(CredentialProvider, String, String)>, String> {
    let _guard = store_lock()
        .lock()
        .map_err(|_| "credential disk store lock poisoned".to_string())?;
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

fn store_lock() -> &'static Mutex<()> {
    STORE_LOCK.get_or_init(|| Mutex::new(()))
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
#[cfg(unix)]
fn write_secret_file(path: &Path, data: &[u8]) -> Result<(), String> {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    let parent = path
        .parent()
        .ok_or_else(|| "credential store path has no parent directory".to_string())?;
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("credential-store.json");
    let tmp = parent.join(format!(".{file_name}.tmp.{}", std::process::id()));

    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&tmp)
        .map_err(|error| format!("create credential store temp: {error}"))?;
    // `.mode()` applies only when the temp is freshly created; a pre-existing
    // temp keeps its old mode, so force 0600 before any secret is written.
    file.set_permissions(std::fs::Permissions::from_mode(0o600))
        .map_err(|error| format!("restrict credential store temp: {error}"))?;
    file.write_all(data)
        .map_err(|error| format!("write credential store temp: {error}"))?;
    file.sync_all()
        .map_err(|error| format!("sync credential store temp: {error}"))?;
    drop(file);

    std::fs::rename(&tmp, path).map_err(|error| {
        let _ = std::fs::remove_file(&tmp);
        format!("rename credential store into place: {error}")
    })
}

#[cfg(not(unix))]
fn write_secret_file(path: &Path, data: &[u8]) -> Result<(), String> {
    std::fs::write(path, data).map_err(|error| format!("write credential disk store: {error}"))
}
