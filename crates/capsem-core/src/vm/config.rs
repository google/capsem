use std::path::{Path, PathBuf};

use thiserror::Error;
use tracing::debug_span;

const MIN_CPU: u32 = 1;
const MAX_CPU: u32 = 8;
const MIN_RAM: u64 = 256 * 1024 * 1024; // 256 MB
const MAX_RAM: u64 = 16 * 1024 * 1024 * 1024; // 16 GB

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
}

#[derive(Debug, Clone)]
pub struct VmConfig {
    pub cpu_count: u32,
    pub ram_bytes: u64,
    pub kernel_path: PathBuf,
    pub initrd_path: Option<PathBuf>,
    pub disk_path: Option<PathBuf>,
    pub scratch_disk_path: Option<PathBuf>,
    pub kernel_cmdline: String,
    pub expected_kernel_hash: Option<String>,
    pub expected_initrd_hash: Option<String>,
    pub expected_disk_hash: Option<String>,
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
    kernel_cmdline: String,
    expected_kernel_hash: Option<String>,
    expected_initrd_hash: Option<String>,
    expected_disk_hash: Option<String>,
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
            kernel_cmdline: "console=hvc0 root=/dev/vda ro".to_string(),
            expected_kernel_hash: None,
            expected_initrd_hash: None,
            expected_disk_hash: None,
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

        Ok(VmConfig {
            cpu_count: self.cpu_count,
            ram_bytes: self.ram_bytes,
            kernel_path,
            initrd_path: self.initrd_path,
            disk_path: self.disk_path,
            scratch_disk_path: self.scratch_disk_path,
            kernel_cmdline: self.kernel_cmdline,
            expected_kernel_hash: self.expected_kernel_hash,
            expected_initrd_hash: self.expected_initrd_hash,
            expected_disk_hash: self.expected_disk_hash,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn temp_file(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join("capsem-test");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(b"fake").unwrap();
        path
    }

    // --- valid configs ---

    #[test]
    fn valid_config_minimal() {
        let kernel = temp_file("vmlinuz-valid-min");
        let config = VmConfig::builder()
            .kernel_path(&kernel)
            .build()
            .unwrap();
        assert_eq!(config.cpu_count, 4); // default
        assert_eq!(config.ram_bytes, 4 * 1024 * 1024 * 1024); // default 4GB
        assert_eq!(config.kernel_cmdline, "console=hvc0 root=/dev/vda ro");
        assert!(config.initrd_path.is_none());
        assert!(config.disk_path.is_none());
    }

    #[test]
    fn valid_config_all_fields() {
        let kernel = temp_file("vmlinuz-full");
        let initrd = temp_file("initrd-full");
        let disk = temp_file("disk-full");
        let config = VmConfig::builder()
            .cpu_count(4)
            .ram_bytes(4 * 1024 * 1024 * 1024)
            .kernel_path(&kernel)
            .initrd_path(&initrd)
            .disk_path(&disk)
            .kernel_cmdline("console=ttyS0 root=/dev/vda rw quiet")
            .build()
            .unwrap();
        assert_eq!(config.cpu_count, 4);
        assert_eq!(config.ram_bytes, 4 * 1024 * 1024 * 1024);
        assert_eq!(config.kernel_path, kernel);
        assert_eq!(config.initrd_path.unwrap(), initrd);
        assert_eq!(config.disk_path.unwrap(), disk);
        assert_eq!(config.kernel_cmdline, "console=ttyS0 root=/dev/vda rw quiet");
    }

    // --- CPU boundary tests ---

    #[test]
    fn accepts_cpu_min_boundary() {
        let kernel = temp_file("vmlinuz-cpu1");
        let config = VmConfig::builder()
            .cpu_count(1)
            .kernel_path(&kernel)
            .build();
        assert!(config.is_ok());
        assert_eq!(config.unwrap().cpu_count, 1);
    }

    #[test]
    fn accepts_cpu_max_boundary() {
        let kernel = temp_file("vmlinuz-cpu8");
        let config = VmConfig::builder()
            .cpu_count(8)
            .kernel_path(&kernel)
            .build();
        assert!(config.is_ok());
        assert_eq!(config.unwrap().cpu_count, 8);
    }

    #[test]
    fn rejects_cpu_zero() {
        let kernel = temp_file("vmlinuz-cpu0");
        let err = VmConfig::builder()
            .cpu_count(0)
            .kernel_path(&kernel)
            .build();
        assert!(matches!(err, Err(ConfigError::CpuOutOfRange(0))));
    }

    #[test]
    fn rejects_cpu_just_above_max() {
        let kernel = temp_file("vmlinuz-cpu9");
        let err = VmConfig::builder()
            .cpu_count(9)
            .kernel_path(&kernel)
            .build();
        assert!(matches!(err, Err(ConfigError::CpuOutOfRange(9))));
    }

    #[test]
    fn rejects_cpu_far_above_max() {
        let kernel = temp_file("vmlinuz-cpu99");
        let err = VmConfig::builder()
            .cpu_count(99)
            .kernel_path(&kernel)
            .build();
        assert!(matches!(err, Err(ConfigError::CpuOutOfRange(99))));
    }

    // --- RAM boundary tests ---

    #[test]
    fn accepts_ram_min_boundary() {
        let kernel = temp_file("vmlinuz-ram-min");
        let config = VmConfig::builder()
            .ram_bytes(256 * 1024 * 1024) // exactly 256MB
            .kernel_path(&kernel)
            .build();
        assert!(config.is_ok());
    }

    #[test]
    fn accepts_ram_max_boundary() {
        let kernel = temp_file("vmlinuz-ram-max");
        let config = VmConfig::builder()
            .ram_bytes(16 * 1024 * 1024 * 1024) // exactly 16GB
            .kernel_path(&kernel)
            .build();
        assert!(config.is_ok());
    }

    #[test]
    fn rejects_ram_just_below_min() {
        let kernel = temp_file("vmlinuz-ram-below");
        let err = VmConfig::builder()
            .ram_bytes(256 * 1024 * 1024 - 1)
            .kernel_path(&kernel)
            .build();
        assert!(matches!(err, Err(ConfigError::RamOutOfRange(_))));
    }

    #[test]
    fn rejects_ram_just_above_max() {
        let kernel = temp_file("vmlinuz-ram-above");
        let err = VmConfig::builder()
            .ram_bytes(16 * 1024 * 1024 * 1024 + 1)
            .kernel_path(&kernel)
            .build();
        assert!(matches!(err, Err(ConfigError::RamOutOfRange(_))));
    }

    #[test]
    fn rejects_ram_zero() {
        let kernel = temp_file("vmlinuz-ram0");
        let err = VmConfig::builder()
            .ram_bytes(0)
            .kernel_path(&kernel)
            .build();
        assert!(matches!(err, Err(ConfigError::RamOutOfRange(0))));
    }

    #[test]
    fn rejects_ram_very_large() {
        let kernel = temp_file("vmlinuz-ram-huge");
        let err = VmConfig::builder()
            .ram_bytes(32 * 1024 * 1024 * 1024)
            .kernel_path(&kernel)
            .build();
        assert!(matches!(err, Err(ConfigError::RamOutOfRange(_))));
    }

    // --- kernel path tests ---

    #[test]
    fn rejects_no_kernel_path_set() {
        let err = VmConfig::builder().build();
        assert!(matches!(err, Err(ConfigError::MissingKernel(_))));
    }

    #[test]
    fn rejects_nonexistent_kernel_file() {
        let err = VmConfig::builder()
            .kernel_path("/nonexistent/vmlinuz")
            .build();
        assert!(matches!(err, Err(ConfigError::MissingKernel(_))));
    }

    #[test]
    fn kernel_error_contains_path() {
        let err = VmConfig::builder()
            .kernel_path("/nonexistent/vmlinuz")
            .build()
            .unwrap_err();
        let msg = err.to_string();
        assert!(msg.contains("/nonexistent/vmlinuz"), "error should contain path: {msg}");
    }

    // --- initrd path tests ---

    #[test]
    fn rejects_nonexistent_initrd() {
        let kernel = temp_file("vmlinuz-initrd-bad");
        let err = VmConfig::builder()
            .kernel_path(&kernel)
            .initrd_path("/nonexistent/initrd.img")
            .build();
        assert!(matches!(err, Err(ConfigError::MissingInitrd(_))));
    }

    #[test]
    fn accepts_valid_initrd() {
        let kernel = temp_file("vmlinuz-initrd-ok");
        let initrd = temp_file("initrd-ok");
        let config = VmConfig::builder()
            .kernel_path(&kernel)
            .initrd_path(&initrd)
            .build();
        assert!(config.is_ok());
        assert_eq!(config.unwrap().initrd_path.unwrap(), initrd);
    }

    // --- disk path tests ---

    #[test]
    fn rejects_nonexistent_disk() {
        let kernel = temp_file("vmlinuz-disk-bad");
        let err = VmConfig::builder()
            .kernel_path(&kernel)
            .disk_path("/nonexistent/rootfs.img")
            .build();
        assert!(matches!(err, Err(ConfigError::MissingDisk(_))));
    }

    #[test]
    fn accepts_valid_disk() {
        let kernel = temp_file("vmlinuz-disk-ok");
        let disk = temp_file("rootfs-ok.img");
        let config = VmConfig::builder()
            .kernel_path(&kernel)
            .disk_path(&disk)
            .build();
        assert!(config.is_ok());
        assert_eq!(config.unwrap().disk_path.unwrap(), disk);
    }

    // --- scratch disk path tests ---

    #[test]
    fn rejects_nonexistent_scratch_disk() {
        let kernel = temp_file("vmlinuz-scratch-bad");
        let err = VmConfig::builder()
            .kernel_path(&kernel)
            .scratch_disk_path("/nonexistent/scratch.img")
            .build();
        assert!(matches!(err, Err(ConfigError::MissingDisk(_))));
    }

    #[test]
    fn accepts_valid_scratch_disk() {
        let kernel = temp_file("vmlinuz-scratch-ok");
        let scratch = temp_file("scratch-ok.img");
        let config = VmConfig::builder()
            .kernel_path(&kernel)
            .scratch_disk_path(&scratch)
            .build();
        assert!(config.is_ok());
        assert_eq!(config.unwrap().scratch_disk_path.unwrap(), scratch);
    }

    #[test]
    fn accepts_both_disk_and_scratch_disk() {
        let kernel = temp_file("vmlinuz-both-disks");
        let disk = temp_file("rootfs-both.img");
        let scratch = temp_file("scratch-both.img");
        let config = VmConfig::builder()
            .kernel_path(&kernel)
            .disk_path(&disk)
            .scratch_disk_path(&scratch)
            .build()
            .unwrap();
        assert_eq!(config.disk_path.unwrap(), disk);
        assert_eq!(config.scratch_disk_path.unwrap(), scratch);
    }

    // --- builder defaults ---

    #[test]
    fn builder_defaults_are_sane() {
        let b = VmConfigBuilder::default();
        assert_eq!(b.cpu_count, 4);
        assert_eq!(b.ram_bytes, 4 * 1024 * 1024 * 1024);
        assert!(b.kernel_path.is_none());
        assert!(b.initrd_path.is_none());
        assert!(b.disk_path.is_none());
        assert!(b.scratch_disk_path.is_none());
        assert_eq!(b.kernel_cmdline, "console=hvc0 root=/dev/vda ro");
    }

    #[test]
    fn builder_is_chainable() {
        let kernel = temp_file("vmlinuz-chain");
        // all setters return Self so this should compile and work
        let config = VmConfig::builder()
            .cpu_count(3)
            .ram_bytes(2 * 1024 * 1024 * 1024)
            .kernel_path(&kernel)
            .kernel_cmdline("quiet")
            .build()
            .unwrap();
        assert_eq!(config.cpu_count, 3);
        assert_eq!(config.ram_bytes, 2 * 1024 * 1024 * 1024);
        assert_eq!(config.kernel_cmdline, "quiet");
    }

    // --- error display ---

    #[test]
    fn cpu_error_displays_value() {
        let err = ConfigError::CpuOutOfRange(42);
        let msg = err.to_string();
        assert!(msg.contains("42"), "should contain the value: {msg}");
        assert!(msg.contains("1"), "should contain min bound: {msg}");
        assert!(msg.contains("8"), "should contain max bound: {msg}");
    }

    #[test]
    fn ram_error_displays_value() {
        let err = ConfigError::RamOutOfRange(999);
        let msg = err.to_string();
        assert!(msg.contains("999"), "should contain the value: {msg}");
    }

    // --- validation order: cpu/ram checked before file existence ---

    #[test]
    fn cpu_validated_before_kernel_check() {
        // Even with no kernel set, cpu out of range should be the error
        let err = VmConfig::builder()
            .cpu_count(0)
            .build()
            .unwrap_err();
        assert!(matches!(err, ConfigError::CpuOutOfRange(0)));
    }

    #[test]
    fn ram_validated_before_kernel_check() {
        let err = VmConfig::builder()
            .ram_bytes(0)
            .build()
            .unwrap_err();
        assert!(matches!(err, ConfigError::RamOutOfRange(0)));
    }
}
