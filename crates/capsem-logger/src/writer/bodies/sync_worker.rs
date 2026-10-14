//! Dedicated archive durability worker.

use std::sync::mpsc;

use capsem_archive::BodyLogSync;

pub(super) struct ArchiveSyncWorker {
    requests: Option<mpsc::SyncSender<()>>,
    completions: mpsc::Receiver<Result<(), String>>,
    join: Option<std::thread::JoinHandle<()>>,
}

impl ArchiveSyncWorker {
    pub(super) fn start(sync: BodyLogSync) -> std::io::Result<Self> {
        let (requests, work) = mpsc::sync_channel(1);
        let (completed, completions) = mpsc::sync_channel(1);
        let join = std::thread::Builder::new()
            .name("capsem-archive-sync".into())
            .stack_size(128 * 1024)
            .spawn(move || {
                while work.recv().is_ok() {
                    let result = sync.sync().map_err(|error| error.to_string());
                    if completed.send(result).is_err() {
                        break;
                    }
                }
            })?;
        Ok(Self {
            requests: Some(requests),
            completions,
            join: Some(join),
        })
    }

    pub(super) fn request(&self) -> Result<(), String> {
        self.requests
            .as_ref()
            .ok_or_else(|| "archive sync worker stopped".to_string())?
            .send(())
            .map_err(|_| "archive sync worker stopped".to_string())
    }

    pub(super) fn try_complete(&self) -> Option<Result<(), String>> {
        match self.completions.try_recv() {
            Ok(result) => Some(result),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => Some(Err("archive sync worker stopped".into())),
        }
    }

    pub(super) fn complete(&self) -> Result<(), String> {
        self.completions
            .recv()
            .map_err(|_| "archive sync worker stopped".to_string())?
    }
}

impl Drop for ArchiveSyncWorker {
    fn drop(&mut self) {
        self.requests.take();
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}
