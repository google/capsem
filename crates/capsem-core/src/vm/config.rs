use std::path::{Path, PathBuf};

use thiserror::Error;
use tracing::debug_span;

const MIN_CPU: u32 = 1;
const MAX_CPU: u32 = 8;
const MIN_RAM: u64 = 256 * 1024 * 1024; // 256 MB
const MAX_RAM: u64 = 16 * 1024 * 1024 * 1024; // 16 GB

/// Default kernel command line (arch-dependent console device).
fn default_kernel_cmdline() -> &'static str {
    #[cfg(target_arch = "aarch64")]
    {
        "console=hvc0 root=/dev/vda ro init_on_alloc=1 slab_nomerge page_alloc.shuffle=1"
    }
    #[cfg(target_arch = "x86_64")]
    {
        "console=ttyS0 root=/dev/vda ro init_on_alloc=1 slab_nomerge page_alloc.shuffle=1"
    }
    #[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
    {
        "console=hvc0 root=/dev/vda ro init_on_alloc=1 slab_nomerge page_alloc.shuffle=1"
    }
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("cpu count {0} out of range [{MIN_CPU}, {MAX_CPU}]")]
    CpuOutOfRange(u32),
    #[error("ram {0} bytes out of range [{MIN_RAM}, {MAX_RAM}]")]
    RamOutOfRange(u64),
    #[error("kernel path does not exist: {0}")]
    MissingKernel(PathBuf),
    #[error("initrd path does not exist: {0}")]
    MissingInitrd(PathBuf),
    #[error("disk path does not exist: {0}")]
    MissingDisk(PathBuf),
    #[error("hash mismatch for {0}: expected {1}, got {2}")]
    HashMismatch(String, String, String),
    #[error("failed to read file for hashing: {0}")]
    Io(#[from] std::io::Error),
    #[error("VirtioFS share directory does not exist: {0}")]
    MissingVirtioFsDir(PathBuf),
    #[error("VirtioFS tag is invalid (must be 1-36 ASCII bytes): {0:?}")]
    InvalidVirtioFsTag(String),
    #[error("duplicate VirtioFS tag: {0:?}")]
    DuplicateVirtioFsTag(String),
    #[error("kernel architecture mismatch: {0}")]
    ArchMismatch(String),
}

/// A VirtioFS shared directory to expose to the guest via virtio-fs.
#[derive(Debug, Clone)]
pub struct VirtioFsShare {
    /// Mount tag visible in guest (e.g., "capsem"). Max 36 ASCII bytes.
    pub tag: String,
    /// Host directory to share.
    pub host_path: PathBuf,
    /// If true, guest cannot write to this share.
    pub read_only: bool,
}

#[derive(Debug, Clone)]
pub struct VmConfig {
    pub cpu_count: u32,
    pub ram_bytes: u64,
    pub kernel_path: PathBuf,
    pub initrd_path: Option<PathBuf>,
    pub disk_path: Option<PathBuf>,
    pub scratch_disk_path: Option<PathBuf>,
    pub virtio_fs_shares: Vec<VirtioFsShare>,
    pub kernel_cmdline: String,
    pub expected_kernel_hash: Option<String>,
    pub expected_initrd_hash: Option<String>,
    pub checkpoint_path: Option<PathBuf>,
    pub expected_disk_hash: Option<String>,
    /// Sidecar file holding the persisted VZGenericMachineIdentifier bytes.
    /// Required for save/restore parity: VZ generates a fresh identifier on
    /// every VZGenericPlatformConfiguration unless explicitly set, and a
    /// mismatched identifier causes restoreMachineStateFromURL to fail with
    /// VZErrorRestore.
    pub machine_identifier_path: Option<PathBuf>,
    /// Append every byte from the VM serial console to this file. Writer is
    /// attached before the VM is started/resumed so no post-resume output is
    /// dropped while a tokio broadcast subscriber is still scheduling.
    pub serial_log_path: Option<PathBuf>,
}

impl VmConfig {
    pub fn builder() -> VmConfigBuilder {
        VmConfigBuilder::default()
    }
}

#[derive(Debug, Clone)]
pub struct VmConfigBuilder {
    cpu_count: u32,
    ram_bytes: u64,
    kernel_path: Option<PathBuf>,
    initrd_path: Option<PathBuf>,
    disk_path: Option<PathBuf>,
    scratch_disk_path: Option<PathBuf>,
    virtio_fs_shares: Vec<VirtioFsShare>,
    kernel_cmdline: String,
    expected_kernel_hash: Option<String>,
    expected_initrd_hash: Option<String>,
    checkpoint_path: Option<PathBuf>,
    expected_disk_hash: Option<String>,
    machine_identifier_path: Option<PathBuf>,
    serial_log_path: Option<PathBuf>,
}

impl Default for VmConfigBuilder {
    fn default() -> Self {
        Self {
            cpu_count: 4,
            ram_bytes: 4 * 1024 * 1024 * 1024, // 4 GB
            kernel_path: None,
            initrd_path: None,
            disk_path: None,
            scratch_disk_path: None,
            virtio_fs_shares: Vec::new(),
            kernel_cmdline: default_kernel_cmdline().to_string(),
            expected_kernel_hash: None,
            expected_initrd_hash: None,
            checkpoint_path: None,
            expected_disk_hash: None,
            machine_identifier_path: None,
            serial_log_path: None,
        }
    }
}

impl VmConfigBuilder {
    pub fn cpu_count(mut self, count: u32) -> Self {
        self.cpu_count = count;
        self
    }

    pub fn ram_bytes(mut self, bytes: u64) -> Self {
        self.ram_bytes = bytes;
        self
    }

    pub fn kernel_path(mut self, path: impl AsRef<Path>) -> Self {
        self.kernel_path = Some(path.as_ref().to_path_buf());
        self
    }

    pub fn initrd_path(mut self, path: impl AsRef<Path>) -> Self {
        self.initrd_path = Some(path.as_ref().to_path_buf());
        self
    }

    pub fn disk_path(mut self, path: impl AsRef<Path>) -> Self {
        self.disk_path = Some(path.as_ref().to_path_buf());
        self
    }

    pub fn scratch_disk_path(mut self, path: impl AsRef<Path>) -> Self {
        self.scratch_disk_path = Some(path.as_ref().to_path_buf());
        self
    }

    pub fn virtio_fs_share(
        mut self,
        tag: impl Into<String>,
        path: impl AsRef<Path>,
        read_only: bool,
    ) -> Self {
        self.virtio_fs_shares.push(VirtioFsShare {
            tag: tag.into(),
            host_path: path.as_ref().to_path_buf(),
            read_only,
        });
        self
    }

    pub fn kernel_cmdline(mut self, cmdline: impl Into<String>) -> Self {
        self.kernel_cmdline = cmdline.into();
        self
    }

    pub fn expected_kernel_hash(mut self, hash: impl Into<String>) -> Self {
        self.expected_kernel_hash = Some(hash.into());
        self
    }

    pub fn expected_initrd_hash(mut self, hash: impl Into<String>) -> Self {
        self.expected_initrd_hash = Some(hash.into());
        self
    }

    pub fn expected_disk_hash(mut self, hash: impl Into<String>) -> Self {
        self.expected_disk_hash = Some(hash.into());
        self
    }

    pub fn checkpoint_path(mut self, path: std::path::PathBuf) -> Self {
        self.checkpoint_path = Some(path);
        self
    }

    pub fn machine_identifier_path(mut self, path: impl AsRef<Path>) -> Self {
        self.machine_identifier_path = Some(path.as_ref().to_path_buf());
        self
    }

    pub fn serial_log_path(mut self, path: impl AsRef<Path>) -> Self {
        self.serial_log_path = Some(path.as_ref().to_path_buf());
        self
    }

    /// Reject kernels built for the wrong CPU architecture.
    /// Reads the first 1024 bytes and checks for architecture-specific magic.
    fn validate_kernel_arch(path: &Path) -> Result<(), ConfigError> {
        use std::io::Read;

        let mut file = std::fs::File::open(path).map_err(ConfigError::Io)?;
        let mut header = [0u8; 1024];
        let n = file.read(&mut header).map_err(ConfigError::Io)?;

        #[cfg(target_arch = "aarch64")]
        {
            // Reject x86_64 bzImage: "HdrS" magic at offset 0x202
            if n > 0x206 {
                let magic = u32::from_le_bytes([
                    header[0x202],
                    header[0x203],
                    header[0x204],
                    header[0x205],
                ]);
                if magic == 0x5372_6448 {
                    return Err(ConfigError::ArchMismatch(format!(
                        "{} is an x86_64 bzImage but this host is aarch64",
                        path.display()
                    )));
                }
            }
        }

        #[cfg(target_arch = "x86_64")]
        {
            // Reject ARM64 Image: magic 0x644d5241 ("ARM\x64") at offset 56
            if n > 60 {
                let magic = u32::from_le_bytes([header[56], header[57], header[58], header[59]]);
                if magic == 0x644d_5241 {
                    return Err(ConfigError::ArchMismatch(format!(
                        "{} is an ARM64 Image but this host is x86_64",
                        path.display()
                    )));
                }
            }
        }

        // Suppress unused-variable warnings on other architectures
        let _ = n;
        Ok(())
    }

    fn verify_hash(path: &Path, expected_hash: &str) -> Result<(), ConfigError> {
        let _span = debug_span!("verify_hash", path = %path.display()).entered();
        use std::fs::File;
        use std::io::Read;

        let mut file = File::open(path)?;
        let mut hasher = blake3::Hasher::new();
        let mut buffer = [0; 65536];
        loop {
            let n = file.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            hasher.update(&buffer[..n]);
        }
        let hash = hasher.finalize().to_hex().to_string();
        if hash != expected_hash {
            return Err(ConfigError::HashMismatch(
                path.display().to_string(),
                expected_hash.to_string(),
                hash,
            ));
        }
        Ok(())
    }

    pub fn build(self) -> Result<VmConfig, ConfigError> {
        if self.cpu_count < MIN_CPU || self.cpu_count > MAX_CPU {
            return Err(ConfigError::CpuOutOfRange(self.cpu_count));
        }
        if self.ram_bytes < MIN_RAM || self.ram_bytes > MAX_RAM {
            return Err(ConfigError::RamOutOfRange(self.ram_bytes));
        }

        let kernel_path = self
            .kernel_path
            .ok_or_else(|| ConfigError::MissingKernel(PathBuf::from("<not set>")))?;
        if !kernel_path.exists() {
            return Err(ConfigError::MissingKernel(kernel_path.clone()));
        }
        Self::validate_kernel_arch(&kernel_path)?;
        if let Some(ref expected) = self.expected_kernel_hash {
            Self::verify_hash(&kernel_path, expected)?;
        }

        if let Some(ref initrd) = self.initrd_path {
            if !initrd.exists() {
                return Err(ConfigError::MissingInitrd(initrd.clone()));
            }
            if let Some(ref expected) = self.expected_initrd_hash {
                Self::verify_hash(initrd, expected)?;
            }
        }

        if let Some(ref disk) = self.disk_path {
            if !disk.exists() {
                return Err(ConfigError::MissingDisk(disk.clone()));
            }
            if let Some(ref expected) = self.expected_disk_hash {
                Self::verify_hash(disk, expected)?;
            }
        }

        if let Some(ref scratch) = self.scratch_disk_path {
            if !scratch.exists() {
                return Err(ConfigError::MissingDisk(scratch.clone()));
            }
        }

        // Validate VirtioFS shares
        let mut seen_tags = std::collections::HashSet::new();
        for share in &self.virtio_fs_shares {
            if share.tag.is_empty() || share.tag.len() > 36 || !share.tag.is_ascii() {
                return Err(ConfigError::InvalidVirtioFsTag(share.tag.clone()));
            }
            if !seen_tags.insert(&share.tag) {
                return Err(ConfigError::DuplicateVirtioFsTag(share.tag.clone()));
            }
            if !share.host_path.is_dir() {
                return Err(ConfigError::MissingVirtioFsDir(share.host_path.clone()));
            }
        }

        Ok(VmConfig {
            cpu_count: self.cpu_count,
            ram_bytes: self.ram_bytes,
            kernel_path,
            initrd_path: self.initrd_path,
            disk_path: self.disk_path,
            scratch_disk_path: self.scratch_disk_path,
            virtio_fs_shares: self.virtio_fs_shares,
            kernel_cmdline: self.kernel_cmdline,
            expected_kernel_hash: self.expected_kernel_hash,
            expected_initrd_hash: self.expected_initrd_hash,
            checkpoint_path: self.checkpoint_path,
            expected_disk_hash: self.expected_disk_hash,
            machine_identifier_path: self.machine_identifier_path,
            serial_log_path: self.serial_log_path,
        })
    }
}

#[cfg(test)]
mod tests;
