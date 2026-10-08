//! Host startup inputs never enter a VM owner's launch environment.
use capsem_foundation::unix::contained::ContainedDir;
use std::io::Read;
use zeroize::Zeroizing;

use super::*;

const INPUT_FILE_ENV: &str = "CAPSEM_CREDENTIAL_INJECTION_FILE";
const INPUT_STORAGE_ENV: &str = "CAPSEM_CREDENTIAL_INJECTION_STORAGE";
const INPUT_SELECTION_ENV: &str = "CAPSEM_CREDENTIAL_INJECTION_ENV";
const MAX_INPUT_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    credentials: Vec<api::CredentialInjectRequest>,
}

pub(crate) fn import_host_inputs() -> Result<usize, String> {
    let storage = match std::env::var(INPUT_STORAGE_ENV).as_deref() {
        Ok("file") | Err(std::env::VarError::NotPresent) => api::CredentialStorage::File,
        Ok("memory") => api::CredentialStorage::Memory,
        _ => return Err("invalid host credential storage mode".into()),
    };
    let mut credentials = Vec::new();
    if let Some(path) = std::env::var_os(INPUT_FILE_ENV).filter(|path| !path.is_empty()) {
        let path = std::path::absolute(PathBuf::from(path)).map_err(|_| "invalid credential input path")?;
        let parent = ContainedDir::open_root(path.parent().ok_or("invalid credential input path")?)
            .map_err(|_| "credential input directory unavailable")?;
        let file = parent
            .open_existing_private_append(path.file_name().ok_or("invalid credential input path")?)
            .map_err(|_| "credential input must be an owner-only regular file")?;
        let mut bytes = Zeroizing::new(Vec::new());
        file.take(MAX_INPUT_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "credential input read failed")?;
        if bytes.len() as u64 > MAX_INPUT_BYTES {
            return Err("credential input exceeds its byte budget".into());
        }
        credentials = serde_json::from_slice::<Input>(&bytes)
            .map_err(|_| "invalid credential input file")?
            .credentials;
    }
    let providers = [
        ("ANTHROPIC_API_KEY", api::CredentialInjectProvider::Anthropic),
        ("OPENAI_API_KEY", api::CredentialInjectProvider::Openai),
        ("GEMINI_API_KEY", api::CredentialInjectProvider::Google),
        ("GOOGLE_API_KEY", api::CredentialInjectProvider::Google),
        ("GITHUB_TOKEN", api::CredentialInjectProvider::Github),
        ("GH_TOKEN", api::CredentialInjectProvider::Github),
    ];
    let selection = std::env::var(INPUT_SELECTION_ENV).unwrap_or_default();
    let names: BTreeSet<_> = selection
        .split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .collect();
    for name in names {
        let provider = providers
            .iter()
            .find(|(known, _)| *known == name)
            .map(|(_, provider)| *provider)
            .ok_or("invalid host credential selection")?;
        let value = std::env::var(name).map_err(|_| "selected host credential unavailable")?;
        if value.is_empty() {
            return Err("selected host credential unavailable".into());
        }
        credentials.push(api::CredentialInjectRequest {
            provider,
            value,
            storage,
        });
    }
    let count = credentials.len();
    for request in credentials {
        let provider = CredentialProvider::all()
            .iter()
            .copied()
            .find(|provider| provider.as_str() == request.provider.as_str())
            .expect("shared provider spelling");
        let persistence = match request.storage {
            api::CredentialStorage::File => CredentialPersistence::File,
            api::CredentialStorage::Memory => CredentialPersistence::Memory,
        };
        let reference = CredentialStore::global().inject(provider, &request.value, persistence)?;
        info!(provider = provider.as_str(), credential_ref = %reference, "host credential injected");
    }
    Ok(count)
}

#[cfg(test)]
mod tests;
