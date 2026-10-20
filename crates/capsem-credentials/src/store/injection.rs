//! Explicit injection uses the same canonical references as captured credentials.

use capsem_proto::credential_injection::CredentialMaterial;

use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialPersistence {
    File,
    Memory,
}

impl CredentialStore {
    /// Persist before publishing when file storage is requested. Memory injection
    /// performs no file access and does not remove any previously persisted copy.
    pub fn inject(
        &self,
        provider: CredentialProvider,
        value: &str,
        persistence: CredentialPersistence,
    ) -> Result<String, String> {
        validate_value(value)?;
        let reference = capsem_proto::credential_reference::credential_reference(provider.as_str(), value);
        let _guard = self.durable_lock.lock().map_err(|_| "credential store unavailable")?;
        if persistence == CredentialPersistence::File {
            durable::write(provider, &reference, value).map_err(|_| "credential file persistence failed")?;
        }
        let key = credential_store_key(provider, &reference);
        let mut cache = self.cache.lock().map_err(|_| "credential store unavailable")?;
        let mut memory = self.memory_refs.lock().map_err(|_| "credential store unavailable")?;
        cache.insert(key.clone(), value.to_owned());
        if persistence == CredentialPersistence::Memory {
            memory.insert(key);
        } else {
            memory.remove(&key);
        }
        drop(memory);
        drop(cache);
        Ok(reference)
    }

    /// Trusted service-to-owner handoff only. Never return this from HTTP routes.
    pub fn memory_credentials(&self) -> Result<Vec<CredentialMaterial>, String> {
        let _guard = self.durable_lock.lock().map_err(|_| "credential store unavailable")?;
        let cache = self.cache.lock().map_err(|_| "credential store unavailable")?;
        let memory = self.memory_refs.lock().map_err(|_| "credential store unavailable")?;
        Ok(memory
            .iter()
            .filter_map(|key| {
                let (provider, credential_ref) = key.split_once(':')?;
                Some(CredentialMaterial {
                    provider: provider.to_owned(),
                    credential_ref: credential_ref.to_owned(),
                    value: cache.get(key)?.clone(),
                })
            })
            .collect())
    }

    /// Validate all entries before publishing any of them. Owner-side imports
    /// intentionally cannot write the durable captured-credential store.
    pub fn import_memory_credentials(&self, entries: Vec<CredentialMaterial>) -> Result<(), String> {
        for entry in &entries {
            validate_value(&entry.value)?;
            let provider = credential_provider_from_str(&entry.provider).ok_or("invalid credential provider")?;
            if entry.credential_ref
                != capsem_proto::credential_reference::credential_reference(provider.as_str(), &entry.value)
            {
                return Err("credential reference mismatch".into());
            }
        }
        let _guard = self.durable_lock.lock().map_err(|_| "credential store unavailable")?;
        let mut cache = self.cache.lock().map_err(|_| "credential store unavailable")?;
        let mut memory = self.memory_refs.lock().map_err(|_| "credential store unavailable")?;
        for entry in entries {
            let key = format!("{}:{}", entry.provider, entry.credential_ref);
            cache.insert(key.clone(), entry.value);
            memory.insert(key);
        }
        drop(memory);
        drop(cache);
        Ok(())
    }
}

fn validate_value(value: &str) -> Result<(), String> {
    if value.is_empty() || value.chars().any(char::is_control) {
        return Err("invalid credential material".into());
    }
    Ok(())
}
