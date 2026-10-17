//! Native exit receipts outlive the ephemeral files they authorize retiring.
use super::*;
use capsem_foundation::unix::{contained::EntryKind, fs::ensure_private_dir};

const STORE_LOCK: &str = "retirements.lock";

pub struct RetirementStore {
    root: ContainedDir,
}

impl RetirementStore {
    pub fn open(path: &Path) -> Result<Self> {
        ensure_private_dir(path)?;
        let root = ContainedDir::open_root(path)?;
        root.validate_private()?;
        // Sync the directory's parent link too. A newly created store must
        // survive loss of the session bytes even if the host then reboots.
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        ContainedDir::open_root(parent)?.sync()?;
        Ok(Self { root })
    }

    /// A matching retained receipt is proof of an already confirmed original
    /// exit, including after deleting its session or rebooting the host.
    pub fn contains(&self, binding: &VmBinding) -> Result<bool> {
        validate_binding(binding)?;
        let _lease = self.lease()?;
        Ok(self.read(binding)?.is_some())
    }

    /// Only the owner of a confirmed durable fence can publish a receipt.
    /// Metadata or fence acquisition alone cannot construct this authority.
    pub fn retain(&self, fence: &SpawnFence) -> Result<()> {
        ensure!(
            fence.record.fenced && fence.record.retired,
            "original native exit not confirmed"
        );
        let _lease = self.lease()?;
        if let Some(previous) = self.read(&fence.record.binding)? {
            ensure!(previous == fence.record, "retained native exit identity changed");
            return Ok(());
        }
        super::super::spawn_identity::publish(
            &self.root,
            &filename(&fence.record.binding),
            &serde_json::to_vec(&fence.record)?,
        )
    }

    // Registry holds its lease while deriving references and pruning. An
    // older snapshot must never prune a receipt of a newly prepared request.
    pub(crate) fn prune_unreferenced(&self, references: &[VmBinding]) -> Result<usize> {
        let _lease = self.lease()?;
        let mut candidates = Vec::new();
        let mut temporary = Vec::new();
        for entry in self.root.entries()? {
            ensure!(
                entry.kind == EntryKind::File,
                "native retirement inventory contains a non-file"
            );
            let name = entry.name.to_str().context("invalid native retirement filename")?;
            if name == STORE_LOCK {
                continue;
            }
            if let Some(id) = name.strip_prefix(".pending-spawn-") {
                let parsed: uuid::Uuid = id.parse()?;
                ensure!(
                    parsed.to_string() == id && !parsed.is_nil(),
                    "invalid retirement temporary name"
                );
                // No publisher owns the store lease. Validate every leftover
                // before removing any receipt or interrupted temporary file.
                self.root.open_existing_private_append(&entry.name)?;
                temporary.push(entry.name);
                continue;
            }
            let id = name.strip_suffix(".json").context("unknown native retirement entry")?;
            let generation: uuid::Uuid = id.parse()?;
            ensure!(
                generation.to_string() == id && !generation.is_nil(),
                "invalid native retirement generation"
            );
            let record = read_named(&self.root, name)?.context("native retirement disappeared during inventory")?;
            ensure!(
                record.binding.generation() == generation,
                "native retirement filename changed"
            );
            ensure!(record.retired && record.fenced, "native retirement proof is incomplete");
            if !references.contains(&record.binding) {
                candidates.push(entry.name);
            }
        }
        for name in candidates.iter().chain(&temporary) {
            self.root.remove_private_file(name)?;
        }
        if !candidates.is_empty() || !temporary.is_empty() {
            self.root.sync()?;
        }
        Ok(candidates.len())
    }

    fn read(&self, binding: &VmBinding) -> Result<Option<Record>> {
        let Some(record) = read_named(&self.root, &filename(binding))? else {
            return Ok(None);
        };
        ensure!(record.binding == *binding, "retained native exit generation changed");
        ensure!(
            record.fenced && record.retired,
            "retained native exit proof is incomplete"
        );
        Ok(Some(record))
    }

    fn lease(&self) -> Result<FileLock> {
        match lock::try_acquire(&self.root.path().join(STORE_LOCK), LockMode::Exclusive)? {
            LockAttempt::Acquired(lease) => Ok(lease),
            LockAttempt::Contended => anyhow::bail!("native retirement store is busy"),
        }
    }
}

fn filename(binding: &VmBinding) -> String {
    format!("{}.json", binding.generation())
}

#[cfg(test)]
mod tests;
