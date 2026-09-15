//! Host-only listener intent; guest workspace and fork snapshots carry no ports.
use super::*;
use capsem_proto::{ipc::PublicationInfo, PublicationTarget};
use std::path::{Path, PathBuf};

/// One declared listener. Records written before targets existed are the
/// container's, which is all a publication could reach then.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) struct SavedPublication {
    pub(super) host: u16,
    pub(super) guest: u16,
    #[serde(default)]
    pub(super) target: PublicationTarget,
}

pub(super) struct Mappings {
    path: PathBuf,
    lock: tokio::sync::Mutex<()>,
}

fn read(path: &Path) -> Result<Vec<SavedPublication>> {
    let bytes = match capsem_foundation::unix::fs::read_regular_file_no_follow(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    ensure!(bytes.len() <= 4096, "saved publication record is too large");
    let ports: Vec<SavedPublication> = serde_json::from_slice(&bytes)?;
    ensure!(ports.len() <= 8, "too many saved publications");
    let mut seen = std::collections::HashSet::new();
    for port in &ports {
        ensure!(
            port.host != 0 && port.target.admits(port.guest) && seen.insert(port.host),
            "invalid saved publication"
        );
    }
    Ok(ports)
}

/// Drop `host` from the saved record so a revoked publication does not come
/// back when the owner restores. Returns whether it was saved.
fn forget(path: &Path, host: u16) -> Result<bool> {
    let mut ports = read(path)?;
    let before = ports.len();
    ports.retain(|port| port.host != host);
    if ports.len() == before {
        return Ok(false);
    }
    capsem_foundation::unix::fs::atomic_write_private(path, &serde_json::to_vec(&ports)?)?;
    Ok(true)
}

impl Publisher {
    pub fn for_session(session_dir: &Path, budgets: capsem_config::router::RouterConfig) -> Result<Self> {
        let mut publisher = Self::configured(budgets)?;
        publisher.saved = Some(Mappings {
            path: session_dir.join("published-ports.json"),
            lock: tokio::sync::Mutex::new(()),
        });
        Ok(publisher)
    }

    /// Re-open every saved publication; returns how many were restored.
    pub async fn restore(self: &Arc<Self>, control: mpsc::Sender<ServiceToProcess>) -> Result<usize> {
        let Some(saved) = &self.saved else {
            return Ok(0);
        };
        let path = saved.path.clone();
        let ports = tokio::task::spawn_blocking(move || read(&path)).await??;
        let mut restored = 0;
        for port in ports {
            let reopened = self
                .open(
                    port.host,
                    port.guest,
                    port.target,
                    control.clone(),
                    crate::security_engine::network::NetworkLifecycleAction::Restored,
                )
                .await;
            match reopened {
                Ok(publication) => {
                    self.declare(port.guest, port.target, publication);
                    restored += 1;
                }
                // The rules changed since it was published: forget it rather
                // than retry a refusal on every start.
                Err(error) if error.is::<super::security::ExposureRefused>() => {
                    tracing::warn!(%error, host_port = port.host, "saved exposure refused on restore");
                    let path = saved.path.clone();
                    tokio::task::spawn_blocking(move || forget(&path, port.host)).await??;
                }
                Err(error) => return Err(error),
            }
        }
        Ok(restored)
    }

    /// Publish, declare and save a publication so the owner restores it.
    pub async fn publish_saved(
        self: &Arc<Self>,
        host: u16,
        guest: u16,
        target: PublicationTarget,
        control: mpsc::Sender<ServiceToProcess>,
    ) -> Result<PublicationInfo> {
        let Some(saved) = &self.saved else {
            let publication = self.publish(host, guest, target, control).await?;
            return Ok(self.declare(guest, target, publication));
        };
        let _lock = saved.lock.lock().await;
        let publication = self.publish(host, guest, target, control).await?;
        let port = SavedPublication {
            host: publication.host_port,
            guest,
            target,
        };
        let path = saved.path.clone();
        tokio::task::spawn_blocking(move || {
            let mut ports = read(&path)?;
            ports.retain(|existing| existing.host != port.host);
            ensure!(ports.len() < 8, "too many saved publications");
            ports.push(port);
            capsem_foundation::unix::fs::atomic_write_private(&path, &serde_json::to_vec(&ports)?)?;
            Ok::<_, anyhow::Error>(())
        })
        .await??;
        Ok(self.declare(guest, target, publication))
    }

    /// Close a declared publication and forget its saved record.
    pub async fn revoke(&self, host: u16) -> Result<bool> {
        // Closed first: the listener does not wait on its audit row.
        let declared = match self.declared.remove(host) {
            Some(entry) => {
                let (publication_id, guest, target) = (entry.handle.publication_id, entry.guest_port, entry.target);
                drop(entry);
                if let Err(error) = self.audit_revoked(publication_id, host, guest, target).await {
                    tracing::warn!(%error, host_port = host, "exposure revocation audit was not admitted");
                }
                true
            }
            None => false,
        };
        let Some(saved) = &self.saved else {
            return Ok(declared);
        };
        let _lock = saved.lock.lock().await;
        let path = saved.path.clone();
        let forgotten = tokio::task::spawn_blocking(move || forget(&path, host)).await??;
        Ok(declared || forgotten)
    }
}

#[cfg(test)]
mod tests;
