//! Host-only listener intent; guest workspace and fork snapshots carry no ports.
use super::*;
use crate::container::PortMapping;
use std::path::{Path, PathBuf};

pub(super) struct Mappings {
    path: PathBuf,
    lock: tokio::sync::Mutex<()>,
}

fn read(path: &Path) -> Result<Vec<PortMapping>> {
    let bytes = match capsem_foundation::unix::fs::read_regular_file_no_follow(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error.into()),
    };
    ensure!(bytes.len() <= 4096, "saved publication record is too large");
    let ports: Vec<PortMapping> = serde_json::from_slice(&bytes)?;
    ensure!(ports.len() <= 8, "too many saved publications");
    let mut seen = std::collections::HashSet::new();
    for port in &ports {
        ensure!(
            port.host != 0 && port.guest != 0 && seen.insert(port.host),
            "invalid saved publication"
        );
    }
    Ok(ports)
}

impl Publisher {
    pub fn for_session(session_dir: &Path) -> Self {
        Self {
            saved: Some(Mappings {
                path: session_dir.join("published-ports.json"),
                lock: tokio::sync::Mutex::new(()),
            }),
            ..Self::default()
        }
    }

    pub async fn restore(self: &Arc<Self>, control: mpsc::Sender<ServiceToProcess>) -> Result<Vec<Publication>> {
        let Some(saved) = &self.saved else {
            return Ok(Vec::new());
        };
        let path = saved.path.clone();
        let ports = tokio::task::spawn_blocking(move || read(&path)).await??;
        let mut publications = Vec::new();
        for port in ports {
            publications.push(self.publish(port.host, port.guest, control.clone()).await?);
        }
        Ok(publications)
    }

    pub async fn publish_saved(
        self: &Arc<Self>,
        host: u16,
        guest: u16,
        control: mpsc::Sender<ServiceToProcess>,
    ) -> Result<Publication> {
        let Some(saved) = &self.saved else {
            return self.publish(host, guest, control).await;
        };
        let _lock = saved.lock.lock().await;
        let publication = self.publish(host, guest, control).await?;
        let port = PortMapping {
            host: publication.host_port,
            guest,
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
        Ok(publication)
    }
}

#[cfg(test)]
mod tests;
