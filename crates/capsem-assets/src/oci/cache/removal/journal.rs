//! A synced intent precedes each unlink; retries never invent a new target.

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use super::super::super::{
    removal::{preview_token, Witness},
    RemovalResult, METADATA_LIMIT,
};
use super::*;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Step {
    pub(super) name: String,
    pub(super) file: FileState,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Journal {
    schema_version: u32,
    pub(super) key: String,
    pub(super) token: String,
    pub(super) reason: String,
    pub(super) receipt_json: String,
    pub(super) witness: Witness,
    pub(super) pending: Option<Step>,
    pub(super) done: BTreeSet<String>,
    pub(super) result: RemovalResult,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Applied {
    schema_version: u32,
    key: String,
    token: String,
    reason: String,
    pub(super) result: RemovalResult,
    checksum: String,
}

impl Applied {
    fn checksum(key: &str, token: &str, reason: &str, result: &RemovalResult) -> Result<String> {
        let mut digest = Sha256::new();
        digest.update(b"capsem-oci-removal-applied-v1\0");
        digest.update(serde_json::to_vec(&(key, token, reason, result))?);
        Ok(format!("{:x}", digest.finalize()))
    }

    pub(super) fn validate(&self, key: &CacheKey, token: &str, reason: &str) -> Result<()> {
        ensure!(
            self.schema_version == 2
                && self.key == key.as_str()
                && self.token == token
                && self.reason == reason
                && self.result.complete,
            "applied removal request mismatch"
        );
        ensure!(
            self.checksum == Self::checksum(&self.key, &self.token, &self.reason, &self.result)?,
            "applied removal checksum mismatch"
        );
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
pub(super) enum Saved {
    Applied(Applied),
    Pending(Journal),
}

impl Journal {
    /// Reserve the observed allocation of both versions of the largest
    /// progress record before any payload change. Root/name allocation is
    /// measured by the real filesystem, not estimated from logical length.
    pub(super) fn reserve(&self, cache: &BlobCache, root: &ContainedDir) -> Result<()> {
        let names = std::iter::once(format!("receipt-{}", self.key))
            .chain(self.witness.blobs.iter().map(|blob| blob.name.clone()))
            .collect::<Vec<_>>();
        let pending = std::iter::once(Step {
            name: names[0].clone(),
            file: self.witness.receipt.clone(),
        })
        .chain(self.witness.blobs.iter().filter_map(|blob| {
            blob.file.clone().map(|file| Step {
                name: blob.name.clone(),
                file,
            })
        }))
        .map(|step| serde_json::to_vec(&step).map(|bytes| bytes.len()))
        .collect::<std::result::Result<Vec<_>, _>>()?
        .into_iter()
        .max()
        .unwrap_or(0);
        let counters = RemovalResult {
            removed_allocated_bytes: u64::MAX,
            removed_entries: u64::MAX,
            already_missing_entries: u64::MAX,
            retained_entries: u64::MAX,
            complete: false,
        };
        let counters_size = serde_json::to_vec(&counters)?.len();
        let size = serde_json::to_vec(self)?
            .len()
            .checked_add(serde_json::to_vec(&names)?.len())
            .and_then(|size| size.checked_add(pending))
            .and_then(|size| size.checked_add(counters_size))
            .context("removal reservation overflow")?;
        ensure!(
            size <= METADATA_LIMIT,
            "removal intent exceeds bounded metadata allowance"
        );
        ensure!(
            super::super::super::inventory::measure(root)?.allocated_bytes < cache.policy.max_size_bytes,
            "OCI cache has no control headroom"
        );
        // Repeated zeros could compress to almost nothing. Sample the upper
        // bound with varied bytes so compressed filesystems cannot underbill
        // the future JSON records; this stream carries no secret or authority.
        let mut sample = vec![0; size];
        for (index, chunk) in sample.chunks_mut(32).enumerate() {
            let mut digest = Sha256::new();
            digest.update(self.token.as_bytes());
            digest.update(index.to_be_bytes());
            chunk.copy_from_slice(&digest.finalize()[..chunk.len()]);
        }
        let mut reservations = Vec::new();
        for _ in 0..2 {
            let mut file = tempfile::Builder::new()
                .prefix(&format!(".partial-removal-budget-{}-", self.token))
                .tempfile_in(root.path())?;
            file.write_all(&sample)?;
            file.as_file().sync_all()?;
            reservations.push(file);
        }
        cache.check_capacity(root)?;
        drop(reservations);
        Ok(())
    }

    pub(super) fn new(plan: RemovalPreview, receipt: &CacheReceipt, reason: &str) -> Result<Self> {
        Ok(Self {
            schema_version: 1,
            key: plan.key().as_str().into(),
            token: plan.token().into(),
            reason: reason.into(),
            receipt_json: String::from_utf8(receipt.encode()?)?,
            witness: plan.witness,
            pending: None,
            done: BTreeSet::new(),
            result: RemovalResult::default(),
        })
    }

    pub(super) fn validate(&self, cache: &BlobCache, key: &CacheKey, token: &str, reason: &str) -> Result<()> {
        ensure!(
            self.schema_version == 1
                && self.key == key.as_str()
                && self.witness.key == key.as_str()
                && self.token == token
                && self.reason == reason,
            "removal journal request mismatch"
        );
        ensure!(
            preview_token(&self.witness)? == token,
            "removal journal witness mismatch"
        );
        let receipt = CacheReceipt::decode(self.receipt_json.as_bytes(), key)?;
        ensure!(
            receipt.generation()? == self.witness.generation,
            "removal journal generation mismatch"
        );
        let derived = names(cache, &receipt)?;
        let recorded = self
            .witness
            .blobs
            .iter()
            .map(|blob| (blob.name.clone(), blob.kind))
            .collect::<BTreeMap<_, _>>();
        ensure!(derived == recorded, "removal journal contains an unowned target");
        let receipt_name = format!("receipt-{}", key.as_str());
        ensure!(
            self.done
                .iter()
                .all(|name| name == &receipt_name || derived.contains_key(name)),
            "removal journal contains an unowned result"
        );
        let completed = self
            .result
            .removed_entries
            .checked_add(self.result.already_missing_entries)
            .and_then(|count| count.checked_add(self.result.retained_entries));
        ensure!(
            completed == Some(self.done.len() as u64),
            "removal journal result count mismatch"
        );
        ensure!(
            !self.result.complete || (self.pending.is_none() && self.done.len() == derived.len() + 1),
            "removal journal completion mismatch"
        );
        if let Some(step) = &self.pending {
            ensure!(
                !self.done.contains(&step.name),
                "removal journal repeats a completed pending step"
            );
            let expected = if step.name == receipt_name {
                Some(&self.witness.receipt)
            } else {
                self.witness
                    .blobs
                    .iter()
                    .find(|blob| blob.name == step.name && !blob.shared)
                    .and_then(|blob| blob.file.as_ref())
            };
            ensure!(
                expected == Some(&step.file),
                "removal journal contains an unowned pending step"
            );
        }
        Ok(())
    }

    pub(super) fn save(&self, root: &ContainedDir) -> Result<()> {
        let bytes = if self.result.complete {
            serde_json::to_vec(&Applied {
                schema_version: 2,
                key: self.key.clone(),
                token: self.token.clone(),
                reason: self.reason.clone(),
                result: self.result.clone(),
                checksum: Applied::checksum(&self.key, &self.token, &self.reason, &self.result)?,
            })?
        } else {
            serde_json::to_vec(self)?
        };
        ensure!(bytes.len() <= METADATA_LIMIT, "removal journal exceeds metadata limit");
        let mut temporary = tempfile::Builder::new()
            .prefix(".partial-removal-")
            .tempfile_in(root.path())?;
        temporary.write_all(&bytes)?;
        temporary.as_file().sync_all()?;
        temporary.persist(root.path().join(format!("removal-{}.json", self.token)))?;
        root.sync()?;
        Ok(())
    }
}

pub(super) fn load(root: &ContainedDir, token: &str) -> Result<Option<Saved>> {
    let file = match root.open_file(
        format!("removal-{token}.json").as_ref(),
        ContainedOpenOptions::read_only(),
    ) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    ensure!(
        file.metadata()?.len() <= METADATA_LIMIT as u64,
        "removal journal exceeds metadata limit"
    );
    let mut bytes = Vec::new();
    file.take((METADATA_LIMIT + 1) as u64).read_to_end(&mut bytes)?;
    Ok(Some(serde_json::from_slice(&bytes).context("invalid removal journal")?))
}
