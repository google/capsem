use std::collections::HashMap;
use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};
use anyhow::{Context, Result};
use std::time::SystemTime;
use std::io::Write;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageEntry {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub source_vm: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_image: Option<String>,
    pub base_version: String,
    #[serde(with = "crate::manifest_compat::time_format")]
    pub created_at: SystemTime,
    pub size_bytes: u64,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ImageRegistryData {
    pub images: HashMap<String, ImageEntry>,
}

#[derive(Clone)]
pub struct ImageRegistry {
    pub images_dir: PathBuf,
    registry_path: PathBuf,
}

impl ImageRegistry {
    pub fn new(base_dir: &Path) -> Self {
        let images_dir = base_dir.join("images");
        let registry_path = images_dir.join("image_registry.json");
        Self { registry_path, images_dir }
    }

    pub fn load(&self) -> Result<ImageRegistryData> {
        if !self.registry_path.exists() {
            return Ok(ImageRegistryData::default());
        }
        let content = std::fs::read_to_string(&self.registry_path)
            .context("failed to read image_registry.json")?;
        Ok(serde_json::from_str(&content)?)
    }

    fn save(&self, data: &ImageRegistryData) -> Result<()> {
        if !self.images_dir.exists() {
            std::fs::create_dir_all(&self.images_dir)?;
        }
        let content = serde_json::to_string_pretty(data)?;
        // Atomic write: write to temp file then rename
        let tmp_path = self.registry_path.with_extension("json.tmp");
        let mut f = std::fs::File::create(&tmp_path)?;
        f.write_all(content.as_bytes())?;
        f.sync_all()?;
        std::fs::rename(&tmp_path, &self.registry_path)?;
        Ok(())
    }

    /// Acquire an exclusive file lock for registry mutations.
    fn lock_registry(&self) -> Result<std::fs::File> {
        if !self.images_dir.exists() {
            std::fs::create_dir_all(&self.images_dir)?;
        }
        let lock_path = self.images_dir.join(".registry.lock");
        let f = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&lock_path)
            .context("failed to open registry lock")?;
        use std::os::unix::io::AsRawFd;
        let rc = unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX) };
        if rc != 0 {
            anyhow::bail!("failed to acquire registry lock: {}", std::io::Error::last_os_error());
        }
        Ok(f)
    }

    pub fn get(&self, name: &str) -> Result<Option<ImageEntry>> {
        let data = self.load()?;
        Ok(data.images.get(name).cloned())
    }

    pub fn list(&self) -> Result<Vec<ImageEntry>> {
        let data = self.load()?;
        let mut entries: Vec<_> = data.images.into_values().collect();
        entries.sort_by(|a, b| b.created_at.cmp(&a.created_at)); // newest first
        Ok(entries)
    }

    pub fn insert(&self, entry: ImageEntry) -> Result<()> {
        let _lock = self.lock_registry()?;
        let mut data = self.load()?;
        data.images.insert(entry.name.clone(), entry);
        self.save(&data)
    }

    pub fn remove(&self, name: &str) -> Result<bool> {
        let _lock = self.lock_registry()?;
        let mut data = self.load()?;
        let removed = data.images.remove(name).is_some();
        if removed {
            self.save(&data)?;
            let img_dir = self.images_dir.join(name);
            if img_dir.exists() {
                std::fs::remove_dir_all(&img_dir)?;
            }
        }
        Ok(removed)
    }

    pub fn image_dir(&self, name: &str) -> PathBuf {
        assert!(
            !name.contains('/') && !name.contains('\\') && !name.contains(".."),
            "image_dir path escape: {name}"
        );
        self.images_dir.join(name)
    }

    /// Return names of images that depend on a specific base squashfs version.
    pub fn images_for_base_version(&self, version: &str) -> Result<Vec<String>> {
        let data = self.load()?;
        let mut names = Vec::new();
        for entry in data.images.values() {
            if entry.base_version == version {
                names.push(entry.name.clone());
            }
        }
        Ok(names)
    }
}

/// Copy a session's workspace and rootfs into a new image directory.
/// Note: We do NOT clone session.db or serial.log to ensure the new image starts clean.
pub fn create_image_from_session(
    registry: &ImageRegistry,
    session_dir: &Path,
    image_name: &str,
    description: Option<String>,
    source_vm: &str,
    parent_image: Option<String>,
    base_version: &str,
) -> Result<ImageEntry> {
    let dst_dir = registry.image_dir(image_name);
    // Atomic: create_dir fails if already exists, avoiding TOCTOU race
    if let Err(e) = std::fs::create_dir_all(dst_dir.parent().unwrap()) {
        anyhow::bail!("failed to create images directory: {e}");
    }
    if let Err(e) = std::fs::create_dir(&dst_dir) {
        if e.kind() == std::io::ErrorKind::AlreadyExists {
            anyhow::bail!("Image {} already exists", image_name);
        }
        return Err(e.into());
    }

    let sys_src = session_dir.join("system");
    let ws_src = session_dir.join("workspace");
    let sys_dst = dst_dir.join("system");
    let ws_dst = dst_dir.join("workspace");

    // Flush the host page cache for rootfs.img before cloning.
    // Guest writes arrive via VirtioFS and land in the macOS page cache.
    // Without fsync, clonefile() captures stale APFS data, missing
    // recently written overlay changes (e.g. installed packages).
    let rootfs_path = sys_src.join("rootfs.img");
    if rootfs_path.exists() {
        if let Ok(f) = std::fs::OpenOptions::new().write(true).open(&rootfs_path) {
            f.sync_all().context("failed to fsync rootfs.img before fork")?;
        }
    }

    if sys_src.exists() {
        crate::auto_snapshot::clone_directory(&sys_src, &sys_dst)
            .context("failed to clone system dir")?;
    }
    if ws_src.exists() {
        crate::auto_snapshot::clone_directory(&ws_src, &ws_dst)
            .context("failed to clone workspace dir")?;
    }

    // Include session.db in the image as requested by the user.
    let db_src = session_dir.join("session.db");
    if db_src.exists() {
        let db_dst = dst_dir.join("session.db");
        // Use clone_file (CoW) for the database file.
        crate::auto_snapshot::clone_file(&db_src, &db_dst)
            .context("failed to clone session.db")?;
    }

    let size_bytes = crate::session::disk_usage_bytes(&dst_dir);

    let entry = ImageEntry {
        name: image_name.to_string(),
        description,
        source_vm: source_vm.to_string(),
        parent_image,
        base_version: base_version.to_string(),
        created_at: SystemTime::now(),
        size_bytes,
    };

    registry.insert(entry.clone())?;
    Ok(entry)
}

/// Create a new session directory by cloning an image's workspace and system layers.
/// Data is placed under `guest/` so the VM can see it via VirtioFS, with compat
/// symlinks at the session root. Leaves telemetry and logs empty for a fresh start.
pub fn create_session_from_image(
    registry: &ImageRegistry,
    image_name: &str,
    session_dir: &Path,
) -> Result<()> {
    let img_dir = registry.image_dir(image_name);
    if !img_dir.exists() {
        anyhow::bail!("Image directory not found for {}", image_name);
    }

    // Clone into guest/ subdirectories matching VirtioFS share layout.
    // The VM only sees session_dir/guest/ via VirtioFS, so data must live there.
    let guest_dir = session_dir.join("guest");
    std::fs::create_dir_all(&guest_dir)?;

    let sys_src = img_dir.join("system");
    let ws_src = img_dir.join("workspace");
    let sys_dst = guest_dir.join("system");
    let ws_dst = guest_dir.join("workspace");

    if sys_src.exists() {
        crate::auto_snapshot::clone_directory(&sys_src, &sys_dst)
            .context("failed to clone system dir to session")?;
    }
    if ws_src.exists() {
        crate::auto_snapshot::clone_directory(&ws_src, &ws_dst)
            .context("failed to clone workspace dir to session")?;
    }

    // Compat symlinks so code using session_dir/system still works
    for name in &["system", "workspace"] {
        let link = session_dir.join(name);
        let target = std::path::Path::new("guest").join(name);
        if !link.exists() {
            std::os::unix::fs::symlink(&target, &link)
                .with_context(|| format!("failed to create compat symlink for {name}"))?;
        }
    }

    // Restore session.db at session root (host-only, not in guest/)
    let db_src = img_dir.join("session.db");
    if db_src.exists() {
        let db_dst = session_dir.join("session.db");
        crate::auto_snapshot::clone_file(&db_src, &db_dst)
            .context("failed to clone session.db from image")?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn registry_crud() {
        let dir = tempdir().unwrap();
        let reg = ImageRegistry::new(dir.path());
        
        let entry = ImageEntry {
            name: "test-img".into(),
            description: Some("A test image".into()),
            source_vm: "src-vm".into(),
            parent_image: None,
            base_version: "0.16.1".into(),
            created_at: SystemTime::now(),
            size_bytes: 1024,
        };

        reg.insert(entry.clone()).unwrap();
        let loaded = reg.get("test-img").unwrap().unwrap();
        assert_eq!(loaded.name, "test-img");
        assert_eq!(loaded.base_version, "0.16.1");
        
        let list = reg.list().unwrap();
        assert_eq!(list.len(), 1);
        
        let bases = reg.images_for_base_version("0.16.1").unwrap();
        assert_eq!(bases.len(), 1);
        assert_eq!(bases[0], "test-img");

        let empty_bases = reg.images_for_base_version("99.0.0").unwrap();
        assert!(empty_bases.is_empty());

        reg.remove("test-img").unwrap();
        assert!(reg.get("test-img").unwrap().is_none());
        assert!(reg.list().unwrap().is_empty());
    }

    #[test]
    fn create_image_fails_when_already_exists() {
        let dir = tempdir().unwrap();
        let reg = ImageRegistry::new(dir.path());

        let session_dir = dir.path().join("mock-session");
        std::fs::create_dir_all(session_dir.join("system")).unwrap();
        std::fs::create_dir_all(session_dir.join("workspace")).unwrap();
        std::fs::write(session_dir.join("system/rootfs.img"), "fake").unwrap();

        create_image_from_session(&reg, &session_dir, "dup-img", None, "vm1", None, "0.16.1")
            .unwrap();

        let err = create_image_from_session(&reg, &session_dir, "dup-img", None, "vm2", None, "0.16.1")
            .unwrap_err();
        assert!(
            err.to_string().contains("already exists"),
            "Expected 'already exists' error, got: {err}"
        );
    }

    #[test]
    fn create_session_from_nonexistent_image() {
        let dir = tempdir().unwrap();
        let reg = ImageRegistry::new(dir.path());

        let session_dir = dir.path().join("new-session");
        let err = create_session_from_image(&reg, "no-such-image", &session_dir).unwrap_err();
        assert!(
            err.to_string().contains("not found"),
            "Expected 'not found' error, got: {err}"
        );
    }

    #[test]
    fn create_image_empty_session() {
        let dir = tempdir().unwrap();
        let reg = ImageRegistry::new(dir.path());

        // Session dir with nothing inside
        let session_dir = dir.path().join("empty-session");
        std::fs::create_dir_all(&session_dir).unwrap();

        let entry = create_image_from_session(
            &reg, &session_dir, "empty-img", Some("empty".into()),
            "empty-vm", None, "0.16.1",
        ).unwrap();

        assert_eq!(entry.name, "empty-img");
        let img_dir = reg.image_dir("empty-img");
        assert!(img_dir.exists());
        // No system or workspace should exist
        assert!(!img_dir.join("system").exists());
        assert!(!img_dir.join("workspace").exists());
        // But it should be in the registry
        assert!(reg.get("empty-img").unwrap().is_some());
    }

    #[test]
    fn remove_nonexistent_returns_false() {
        let dir = tempdir().unwrap();
        let reg = ImageRegistry::new(dir.path());
        assert!(!reg.remove("no-such-image").unwrap());
    }

    #[test]
    fn image_creation_and_session_restore() {
        let dir = tempdir().unwrap();
        let reg = ImageRegistry::new(dir.path());

        let session_dir = dir.path().join("mock-session");
        std::fs::create_dir_all(session_dir.join("system")).unwrap();
        std::fs::create_dir_all(session_dir.join("workspace")).unwrap();
        std::fs::write(session_dir.join("system/rootfs.img"), "fake img").unwrap();
        std::fs::write(session_dir.join("workspace/user_file.txt"), "user data").unwrap();
        std::fs::write(session_dir.join("session.db"), "fake db").unwrap();
        std::fs::write(session_dir.join("serial.log"), "fake logs").unwrap();

        let entry = create_image_from_session(
            &reg,
            &session_dir,
            "my-fork",
            Some("My fork desc".into()),
            "mock-session",
            None,
            "0.16.1"
        ).unwrap();

        assert_eq!(entry.name, "my-fork");
        assert_eq!(entry.description.as_deref(), Some("My fork desc"));

        let img_dir = reg.image_dir("my-fork");
        assert!(img_dir.join("system/rootfs.img").exists());
        assert!(img_dir.join("workspace/user_file.txt").exists());

        // session.db must be cloned as requested by user
        assert!(img_dir.join("session.db").exists(), "session.db should be cloned");
        // serial.log should NOT be cloned (keep it clean)
        assert!(!img_dir.join("serial.log").exists(), "serial.log should NOT be cloned");

        // Restore session from image
        let new_session = dir.path().join("new-session");
        create_session_from_image(&reg, "my-fork", &new_session).unwrap();

        // Data must land under guest/ for VirtioFS visibility
        assert!(new_session.join("guest/system/rootfs.img").exists(),
            "rootfs.img must be under guest/system/");
        assert_eq!(
            std::fs::read_to_string(new_session.join("guest/workspace/user_file.txt")).unwrap(),
            "user data",
            "workspace files must be under guest/workspace/"
        );

        // Compat symlinks at session root must resolve to guest/ dirs
        assert!(new_session.join("system").is_symlink(), "system should be a symlink");
        assert!(new_session.join("workspace").is_symlink(), "workspace should be a symlink");
        assert!(new_session.join("system/rootfs.img").exists(),
            "system symlink must resolve to guest/system/rootfs.img");
        assert!(new_session.join("workspace/user_file.txt").exists(),
            "workspace symlink must resolve to guest/workspace/user_file.txt");

        // session.db restored at session root (host-only)
        assert!(new_session.join("session.db").exists(), "session.db should be restored");
    }

    #[test]
    fn create_image_duplicate_name_fails() {
        let dir = tempdir().unwrap();
        let reg = ImageRegistry::new(dir.path());

        let session_dir = dir.path().join("mock-session");
        std::fs::create_dir_all(session_dir.join("workspace")).unwrap();

        create_image_from_session(&reg, &session_dir, "dup", None, "vm1", None, "0.16.1").unwrap();
        let err = create_image_from_session(&reg, &session_dir, "dup", None, "vm1", None, "0.16.1");
        assert!(err.is_err(), "duplicate image name should fail");
        assert!(err.unwrap_err().to_string().contains("already exists"));
    }

    #[test]
    fn concurrent_registry_inserts_all_survive() {
        let dir = tempdir().unwrap();
        let reg = ImageRegistry::new(dir.path());

        let handles: Vec<_> = (0..8)
            .map(|i| {
                let reg = reg.clone();
                std::thread::spawn(move || {
                    reg.insert(ImageEntry {
                        name: format!("img-{i}"),
                        description: None,
                        source_vm: "vm".into(),
                        parent_image: None,
                        base_version: "1.0".into(),
                        created_at: SystemTime::now(),
                        size_bytes: 100,
                    })
                })
            })
            .collect();

        for h in handles {
            h.join().unwrap().unwrap();
        }

        let entries = reg.list().unwrap();
        assert_eq!(entries.len(), 8, "all concurrent inserts should survive");
    }

    #[test]
    #[should_panic(expected = "image_dir path escape")]
    fn image_dir_rejects_path_traversal() {
        let dir = tempdir().unwrap();
        let reg = ImageRegistry::new(dir.path());
        let _ = reg.image_dir("../../etc");
    }
}
