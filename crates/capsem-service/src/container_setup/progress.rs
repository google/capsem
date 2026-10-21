//! Only live pull requests own progress. Polling clones a memory snapshot;
//! it never hashes files, contacts registries or retains completed history.
use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};

use capsem_api::{ImageDownloadPhase, ImageDownloadProgress};
use capsem_assets::oci::{BlobProgress, ProgressObserver};

type TransferState = Mutex<Transfer>;
type ActiveDownloads = HashMap<String, Vec<Weak<TransferState>>>;

#[derive(Default)]
pub(super) struct Downloads(Mutex<ActiveDownloads>);

#[derive(Default)]
struct Transfer {
    blobs: HashMap<String, BlobProgress>,
    total_known: bool,
}

impl Transfer {
    fn update(&mut self, progress: BlobProgress, final_transfer: bool) {
        if progress.completed_bytes > progress.total_bytes
            || (progress.verified && progress.completed_bytes != progress.total_bytes)
        {
            return;
        }
        if let Some(previous) = self.blobs.get_mut(&progress.digest) {
            if previous.total_bytes != progress.total_bytes {
                return;
            }
            previous.completed_bytes = previous.completed_bytes.max(progress.completed_bytes);
            previous.verified |= progress.verified;
        } else {
            self.blobs.insert(progress.digest.clone(), progress);
        }
        self.total_known |= final_transfer;
    }

    fn snapshot(&self) -> ImageDownloadProgress {
        let completed_bytes = self.blobs.values().map(|blob| blob.completed_bytes).sum();
        let total_bytes = self
            .total_known
            .then(|| self.blobs.values().map(|blob| blob.total_bytes).sum());
        ImageDownloadProgress {
            completed_bytes,
            total_bytes,
            phase: if self.blobs.is_empty() {
                ImageDownloadPhase::Preparing
            } else if total_bytes == Some(completed_bytes) {
                ImageDownloadPhase::Verifying
            } else {
                ImageDownloadPhase::Downloading
            },
        }
    }
}

impl Downloads {
    pub(super) fn begin(self: &Arc<Self>, image: &str) -> Download {
        let state = Arc::new(Mutex::new(Transfer::default()));
        self.0
            .lock()
            .unwrap()
            .entry(image.to_owned())
            .or_default()
            .push(Arc::downgrade(&state));
        Download {
            owner: self.clone(),
            image: image.to_owned(),
            state,
        }
    }

    pub(super) fn snapshot(&self, image: &str) -> Option<ImageDownloadProgress> {
        let state = self.0.lock().unwrap().get(image)?.last()?.upgrade()?;
        let progress = state.lock().unwrap().snapshot();
        Some(progress)
    }
}

pub(super) struct Download {
    owner: Arc<Downloads>,
    image: String,
    state: Arc<TransferState>,
}

impl Download {
    pub(super) fn observer(&self, final_transfer: bool) -> ProgressObserver {
        let state = self.state.clone();
        Arc::new(move |progress| state.lock().unwrap().update(progress, final_transfer))
    }
}

impl Drop for Download {
    fn drop(&mut self) {
        let mut active = self.owner.0.lock().unwrap();
        if let Some(states) = active.get_mut(&self.image) {
            states.retain(|state| state.as_ptr() != Arc::as_ptr(&self.state));
            if states.is_empty() {
                active.remove(&self.image);
            }
        }
        drop(active);
    }
}

#[cfg(test)]
mod tests;
