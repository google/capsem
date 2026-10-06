//! Retained image accounting uses the cache owner's descriptor namespace.
use super::*;
use crate::oci::{CacheInventory, CacheKey, ImageInventory};
use std::collections::{HashMap, HashSet};

type Inode = (u64, u64);

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Category {
    Control,
    Unassociated,
    Unmanaged,
}

struct Fact {
    logical: u64,
    allocated: u64,
    links: u64,
    category: Category,
}

impl BlobCache {
    pub(in super::super) async fn inventory(&self) -> Result<CacheInventory> {
        let lease = read_lock(self.mutation_lock()).await?;
        let cache = self.clone();
        tokio::task::spawn_blocking(move || {
            let _lease = lease;
            let root = ContainedDir::open_root(&cache.root)?;
            root.validate_private()?;
            observe(&cache, &root)
        })
        .await?
    }
}

fn observe(cache: &BlobCache, root: &ContainedDir) -> Result<CacheInventory> {
    let directory = root.walk(&cache.policy.entry_root)?;
    let mut receipts = Vec::new();
    let mut invalid_receipts = 0u64;
    for entry in directory.entries()? {
        let name = entry.name.to_string_lossy();
        let Some(key) = name.strip_prefix("receipt-") else {
            continue;
        };
        let receipt = CacheKey::parse(key)
            .and_then(|key| super::receipts::read(&directory, &key)?.context("inventory receipt disappeared"));
        match receipt {
            Ok(receipt) => receipts.push(receipt),
            Err(_) => {
                invalid_receipts = invalid_receipts
                    .checked_add(1)
                    .context("invalid receipt count overflow")?
            }
        }
    }
    receipts.sort_unstable_by_key(|receipt| receipt.key().as_str().to_owned());
    let (facts, files) = scan(cache, root)?;
    let busy = super::removal::materializing_with(root, true)?;
    let mut references: HashMap<Inode, usize> = HashMap::new();
    let mut rows = Vec::new();
    for receipt in receipts {
        let key = receipt.key();
        let mut selected = HashSet::new();
        let mut missing = 0;
        for name in super::removal::names(cache, &receipt)?.into_keys() {
            match files.get(&cache.policy.entry_root.join(name)) {
                Some(inode) => {
                    selected.insert(*inode);
                }
                None => add(&mut missing, 1)?,
            }
        }
        if let Some(inode) = files.get(&cache.policy.entry_root.join(format!("receipt-{}", key.as_str()))) {
            selected.insert(*inode);
        }
        for inode in &selected {
            let count = references.entry(*inode).or_default();
            *count = count.checked_add(1).context("inventory reference count overflow")?;
        }
        rows.push((receipt, selected, missing));
    }
    let usage = crate::oci::inventory::measure(root)?;
    let mut inventory = CacheInventory {
        usage,
        logical_bytes: 0,
        associated_allocated_bytes: 0,
        control_allocated_bytes: 0,
        unassociated_allocated_bytes: 0,
        unmanaged_allocated_bytes: 0,
        reference_graph_complete: invalid_receipts == 0,
        invalid_receipts,
        images: Vec::new(),
    };
    for (inode, fact) in &facts {
        add(&mut inventory.logical_bytes, fact.logical)?;
        let total = if fact.category == Category::Control {
            &mut inventory.control_allocated_bytes
        } else if references.contains_key(inode) {
            &mut inventory.associated_allocated_bytes
        } else if fact.category == Category::Unassociated {
            &mut inventory.unassociated_allocated_bytes
        } else {
            &mut inventory.unmanaged_allocated_bytes
        };
        add(total, fact.allocated)?;
    }
    let mut allocation = inventory.associated_allocated_bytes;
    for value in [
        inventory.control_allocated_bytes,
        inventory.unassociated_allocated_bytes,
        inventory.unmanaged_allocated_bytes,
    ] {
        add(&mut allocation, value)?;
    }
    ensure!(
        allocation == usage.allocated_bytes,
        "cache allocation changed during inventory"
    );
    for (receipt, selected, missing_blobs) in rows {
        let mut row = ImageInventory {
            snapshot: cache.snapshot(&receipt.key())?,
            receipt,
            logical_bytes: 0,
            allocated_bytes: 0,
            shared_allocated_bytes: 0,
            reclaimable_allocated_bytes: 0,
            missing_blobs,
            busy,
        };
        for inode in selected {
            let fact = &facts[&inode];
            add(&mut row.logical_bytes, fact.logical)?;
            add(&mut row.allocated_bytes, fact.allocated)?;
            if references[&inode] > 1 {
                add(&mut row.shared_allocated_bytes, fact.allocated)?;
            } else if inventory.reference_graph_complete && !busy && fact.links == 1 {
                add(&mut row.reclaimable_allocated_bytes, fact.allocated)?;
            }
        }
        inventory.images.push(row);
    }
    Ok(inventory)
}

type Scan = (HashMap<Inode, Fact>, HashMap<PathBuf, Inode>);

fn scan(cache: &BlobCache, root: &ContainedDir) -> Result<Scan> {
    let mut facts: HashMap<Inode, Fact> = HashMap::new();
    let mut files = HashMap::new();
    let mut directories = vec![(root.try_clone()?, PathBuf::new())];
    while let Some((directory, relative)) = directories.pop() {
        let metadata = directory.metadata()?;
        let inode = (metadata.dev(), metadata.ino());
        if facts.contains_key(&inode) {
            continue;
        }
        facts.insert(
            inode,
            Fact {
                logical: 0,
                allocated: metadata
                    .blocks()
                    .checked_mul(512)
                    .context("inventory allocation overflow")?,
                links: 0,
                category: if relative.as_os_str().is_empty()
                    || relative == cache.policy.entry_root
                    || relative == Path::new("locks")
                {
                    Category::Control
                } else {
                    Category::Unmanaged
                },
            },
        );
        for entry in directory.entries()? {
            let path = relative.join(&entry.name);
            if entry.kind == EntryKind::Directory {
                directories.push((directory.descend(&entry.name)?, path));
                continue;
            }
            let inode = (entry.identity.dev, entry.identity.ino);
            let links = if entry.kind == EntryKind::File {
                let file = directory.open_file(&entry.name, ContainedOpenOptions::read_only())?;
                let current = file.metadata()?;
                ensure!(
                    (current.dev(), current.ino()) == inode,
                    "inventory file changed before observation"
                );
                files.insert(path.clone(), inode);
                current.nlink()
            } else {
                0
            };
            let category = category(cache, &path, entry.kind);
            let fact = facts.entry(inode).or_insert(Fact {
                logical: entry.size,
                allocated: entry.allocated,
                links,
                category,
            });
            fact.category = fact.category.min(category);
        }
    }
    Ok((facts, files))
}

fn category(cache: &BlobCache, path: &Path, kind: EntryKind) -> Category {
    if kind != EntryKind::File {
        return Category::Unmanaged;
    }
    if path.starts_with("locks") || cache.policy.mutation_locks.iter().any(|lock| path == lock) {
        return Category::Control;
    }
    if path.parent() != Some(cache.policy.entry_root.as_path()) {
        return Category::Unmanaged;
    }
    let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
        return Category::Unmanaged;
    };
    if name
        .strip_prefix("receipt-")
        .is_some_and(|key| CacheKey::parse(key).is_ok())
    {
        return Category::Control;
    }
    let identity = name.strip_prefix("immutable-").unwrap_or(name);
    if name.starts_with(".partial-")
        || (matches!(identity.len(), 64 | 129)
            && identity
                .split('-')
                .all(|part| crate::oci::digest_hex(&format!("sha256:{part}")).is_ok()))
    {
        Category::Unassociated
    } else {
        Category::Unmanaged
    }
}

fn add(total: &mut u64, value: u64) -> Result<()> {
    *total = total.checked_add(value).context("OCI inventory count overflow")?;
    Ok(())
}

#[cfg(test)]
mod tests;
