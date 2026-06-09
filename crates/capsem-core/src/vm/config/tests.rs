//! Tests for `config` (extracted from inline `mod tests`).

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
    let config = VmConfig::builder().kernel_path(&kernel).build().unwrap();
    assert_eq!(config.cpu_count, 4); // default
    assert_eq!(config.ram_bytes, 4 * 1024 * 1024 * 1024); // default 4GB
    assert_eq!(config.kernel_cmdline, default_kernel_cmdline());
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
    assert_eq!(
        config.kernel_cmdline,
        "console=ttyS0 root=/dev/vda rw quiet"
    );
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
    assert!(
        msg.contains("/nonexistent/vmlinuz"),
        "error should contain path: {msg}"
    );
}

// --- kernel arch validation tests ---

#[cfg(target_arch = "aarch64")]
#[test]
fn rejects_x86_64_bzimage_on_aarch64() {
    let dir = std::env::temp_dir().join("capsem-test-arch");
    std::fs::create_dir_all(&dir).unwrap();
    let kernel = dir.join("vmlinuz-bzimage");
    let mut data = vec![0u8; 4096];
    // bzImage "HdrS" magic at offset 0x202
    data[0x202..0x206].copy_from_slice(&0x5372_6448u32.to_le_bytes());
    std::fs::write(&kernel, &data).unwrap();
    let err = VmConfig::builder().kernel_path(&kernel).build();
    assert!(matches!(err, Err(ConfigError::ArchMismatch(_))));
    let msg = err.unwrap_err().to_string();
    assert!(msg.contains("x86_64"), "error should mention x86_64: {msg}");
}

#[cfg(target_arch = "x86_64")]
#[test]
fn rejects_arm64_image_on_x86_64() {
    let dir = std::env::temp_dir().join("capsem-test-arch");
    std::fs::create_dir_all(&dir).unwrap();
    let kernel = dir.join("vmlinuz-arm64");
    let mut data = vec![0u8; 4096];
    // ARM64 Image magic at offset 56
    data[56..60].copy_from_slice(&0x644d_5241u32.to_le_bytes());
    std::fs::write(&kernel, &data).unwrap();
    let err = VmConfig::builder().kernel_path(&kernel).build();
    assert!(matches!(err, Err(ConfigError::ArchMismatch(_))));
    let msg = err.unwrap_err().to_string();
    assert!(msg.contains("ARM64"), "error should mention ARM64: {msg}");
}

#[test]
fn accepts_correct_arch_kernel() {
    // A kernel with no recognized wrong-arch magic should pass validation.
    let kernel = temp_file("vmlinuz-correct-arch");
    let config = VmConfig::builder().kernel_path(&kernel).build();
    assert!(config.is_ok());
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
        .disk_path("/nonexistent/rootfs.erofs")
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
    assert_eq!(b.kernel_cmdline, default_kernel_cmdline());
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
    let err = VmConfig::builder().cpu_count(0).build().unwrap_err();
    assert!(matches!(err, ConfigError::CpuOutOfRange(0)));
}

#[test]
fn ram_validated_before_kernel_check() {
    let err = VmConfig::builder().ram_bytes(0).build().unwrap_err();
    assert!(matches!(err, ConfigError::RamOutOfRange(0)));
}

// --- VirtioFS share tests ---

#[test]
fn accepts_valid_virtiofs_share() {
    let kernel = temp_file("vmlinuz-vfs-ok");
    let share_dir = std::env::temp_dir().join("capsem-test-vfs-ok");
    std::fs::create_dir_all(&share_dir).unwrap();
    let config = VmConfig::builder()
        .kernel_path(&kernel)
        .virtio_fs_share("capsem", &share_dir, false)
        .build()
        .unwrap();
    assert_eq!(config.virtio_fs_shares.len(), 1);
    assert_eq!(config.virtio_fs_shares[0].tag, "capsem");
    assert!(!config.virtio_fs_shares[0].read_only);
}

#[test]
fn accepts_multiple_virtiofs_shares() {
    let kernel = temp_file("vmlinuz-vfs-multi");
    let dir1 = std::env::temp_dir().join("capsem-test-vfs-m1");
    let dir2 = std::env::temp_dir().join("capsem-test-vfs-m2");
    std::fs::create_dir_all(&dir1).unwrap();
    std::fs::create_dir_all(&dir2).unwrap();
    let config = VmConfig::builder()
        .kernel_path(&kernel)
        .virtio_fs_share("overlay", &dir1, false)
        .virtio_fs_share("cache", &dir2, true)
        .build()
        .unwrap();
    assert_eq!(config.virtio_fs_shares.len(), 2);
}

#[test]
fn rejects_virtiofs_missing_dir() {
    let kernel = temp_file("vmlinuz-vfs-nodir");
    let err = VmConfig::builder()
        .kernel_path(&kernel)
        .virtio_fs_share("capsem", "/nonexistent/virtiofs/dir", false)
        .build();
    assert!(matches!(err, Err(ConfigError::MissingVirtioFsDir(_))));
}

#[test]
fn rejects_virtiofs_empty_tag() {
    let kernel = temp_file("vmlinuz-vfs-empty-tag");
    let dir = std::env::temp_dir().join("capsem-test-vfs-et");
    std::fs::create_dir_all(&dir).unwrap();
    let err = VmConfig::builder()
        .kernel_path(&kernel)
        .virtio_fs_share("", &dir, false)
        .build();
    assert!(matches!(err, Err(ConfigError::InvalidVirtioFsTag(_))));
}

#[test]
fn rejects_virtiofs_tag_too_long() {
    let kernel = temp_file("vmlinuz-vfs-long-tag");
    let dir = std::env::temp_dir().join("capsem-test-vfs-lt");
    std::fs::create_dir_all(&dir).unwrap();
    let long_tag = "a".repeat(37);
    let err = VmConfig::builder()
        .kernel_path(&kernel)
        .virtio_fs_share(long_tag, &dir, false)
        .build();
    assert!(matches!(err, Err(ConfigError::InvalidVirtioFsTag(_))));
}

#[test]
fn accepts_virtiofs_tag_at_max_length() {
    let kernel = temp_file("vmlinuz-vfs-max-tag");
    let dir = std::env::temp_dir().join("capsem-test-vfs-mt");
    std::fs::create_dir_all(&dir).unwrap();
    let max_tag = "a".repeat(36);
    let config = VmConfig::builder()
        .kernel_path(&kernel)
        .virtio_fs_share(max_tag, &dir, false)
        .build()
        .unwrap();
    assert_eq!(config.virtio_fs_shares[0].tag.len(), 36);
}

#[test]
fn rejects_virtiofs_duplicate_tags() {
    let kernel = temp_file("vmlinuz-vfs-dup");
    let dir1 = std::env::temp_dir().join("capsem-test-vfs-d1");
    let dir2 = std::env::temp_dir().join("capsem-test-vfs-d2");
    std::fs::create_dir_all(&dir1).unwrap();
    std::fs::create_dir_all(&dir2).unwrap();
    let err = VmConfig::builder()
        .kernel_path(&kernel)
        .virtio_fs_share("capsem", &dir1, false)
        .virtio_fs_share("capsem", &dir2, true)
        .build();
    assert!(matches!(err, Err(ConfigError::DuplicateVirtioFsTag(_))));
}

#[test]
fn rejects_virtiofs_non_ascii_tag() {
    let kernel = temp_file("vmlinuz-vfs-unicode");
    let dir = std::env::temp_dir().join("capsem-test-vfs-uni");
    std::fs::create_dir_all(&dir).unwrap();
    let err = VmConfig::builder()
        .kernel_path(&kernel)
        .virtio_fs_share("caps\u{00e9}m", &dir, false)
        .build();
    assert!(matches!(err, Err(ConfigError::InvalidVirtioFsTag(_))));
}

#[test]
fn no_virtiofs_shares_by_default() {
    let b = VmConfigBuilder::default();
    assert!(b.virtio_fs_shares.is_empty());
}

// --- hash verification tests ---

#[test]
fn hash_verification_succeeds_with_correct_blake3() {
    let dir = tempfile::tempdir().unwrap();
    let kernel = dir.path().join("vmlinuz");
    let initrd = dir.path().join("initrd.img");
    let rootfs = dir.path().join("rootfs.erofs");
    std::fs::write(&kernel, b"test kernel data").unwrap();
    std::fs::write(&initrd, b"test initrd data").unwrap();
    std::fs::write(&rootfs, b"test rootfs data").unwrap();

    let kh = blake3::hash(b"test kernel data").to_hex().to_string();
    let ih = blake3::hash(b"test initrd data").to_hex().to_string();
    let rh = blake3::hash(b"test rootfs data").to_hex().to_string();

    let config = VmConfig::builder()
        .kernel_path(&kernel)
        .expected_kernel_hash(&kh)
        .initrd_path(&initrd)
        .expected_initrd_hash(&ih)
        .disk_path(&rootfs)
        .expected_disk_hash(&rh)
        .build();
    assert!(config.is_ok(), "all hashes match, build should succeed");
}

#[test]
fn hash_verification_fails_on_corrupted_kernel() {
    let dir = tempfile::tempdir().unwrap();
    let kernel = dir.path().join("vmlinuz");
    std::fs::write(&kernel, b"corrupted kernel").unwrap();
    let wrong_hash = blake3::hash(b"correct kernel").to_hex().to_string();

    let result = VmConfig::builder()
        .kernel_path(&kernel)
        .expected_kernel_hash(&wrong_hash)
        .build();
    assert!(
        matches!(result, Err(ConfigError::HashMismatch(..))),
        "wrong hash should produce HashMismatch, got: {result:?}"
    );
}

#[test]
fn no_expected_hash_skips_verification() {
    let dir = tempfile::tempdir().unwrap();
    let kernel = dir.path().join("vmlinuz");
    std::fs::write(&kernel, b"any content at all").unwrap();

    let config = VmConfig::builder().kernel_path(&kernel).build();
    assert!(config.is_ok(), "no hash set means no verification");
}
