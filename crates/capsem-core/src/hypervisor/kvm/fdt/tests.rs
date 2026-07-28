use super::*;

fn minimal_config() -> FdtConfig {
    FdtConfig {
        ram_base: memory::RAM_BASE,
        ram_size: 512 * 1024 * 1024, // 512MB
        cpu_count: 1,
        cmdline: "console=hvc0".to_string(),
        initrd_start: 0,
        initrd_end: 0,
        virtio_devices: vec![],
    }
}

// -----------------------------------------------------------------------
// Basic FDT generation
// -----------------------------------------------------------------------

#[test]
fn builds_minimal_fdt() {
    let config = minimal_config();
    let blob = build_fdt(&config).unwrap();
    // FDT magic: 0xd00dfeed
    assert_eq!(blob[0], 0xd0);
    assert_eq!(blob[1], 0x0d);
    assert_eq!(blob[2], 0xfe);
    assert_eq!(blob[3], 0xed);
}

#[test]
fn fdt_has_reasonable_size() {
    let config = minimal_config();
    let blob = build_fdt(&config).unwrap();
    // Minimal FDT should be < 4KB
    assert!(blob.len() < 4096, "FDT too large: {} bytes", blob.len());
    // But not empty
    assert!(blob.len() > 100, "FDT too small: {} bytes", blob.len());
}

#[test]
fn fdt_zero_cpus_fails() {
    let mut config = minimal_config();
    config.cpu_count = 0;
    assert!(build_fdt(&config).is_err());
}

// -----------------------------------------------------------------------
// CPU count variations
// -----------------------------------------------------------------------

#[test]
fn fdt_single_cpu() {
    let config = minimal_config();
    let blob = build_fdt(&config).unwrap();
    assert!(!blob.is_empty());
}

#[test]
fn fdt_four_cpus() {
    let mut config = minimal_config();
    config.cpu_count = 4;
    let blob = build_fdt(&config).unwrap();
    assert!(!blob.is_empty());
}

#[test]
fn fdt_eight_cpus() {
    let mut config = minimal_config();
    config.cpu_count = 8;
    let blob = build_fdt(&config).unwrap();
    assert!(!blob.is_empty());
}

// -----------------------------------------------------------------------
// Cmdline variations
// -----------------------------------------------------------------------

#[test]
fn fdt_with_long_cmdline() {
    let mut config = minimal_config();
    config.cmdline = "console=hvc0 root=/dev/vda ro init_on_alloc=1 slab_nomerge page_alloc.shuffle=1 capsem.storage=virtiofs".to_string();
    let blob = build_fdt(&config).unwrap();
    assert!(!blob.is_empty());
}

#[test]
fn fdt_with_empty_cmdline() {
    let mut config = minimal_config();
    config.cmdline = String::new();
    let blob = build_fdt(&config).unwrap();
    assert!(!blob.is_empty());
}

// -----------------------------------------------------------------------
// Initrd
// -----------------------------------------------------------------------

#[test]
fn fdt_with_initrd() {
    let mut config = minimal_config();
    config.initrd_start = 0x5000_0000;
    config.initrd_end = 0x5100_0000;
    let blob = build_fdt(&config).unwrap();
    assert!(!blob.is_empty());
}

#[test]
fn fdt_without_initrd() {
    let config = minimal_config(); // initrd_start = 0
    let blob = build_fdt(&config).unwrap();
    assert!(!blob.is_empty());
}

// -----------------------------------------------------------------------
// Virtio devices
// -----------------------------------------------------------------------

#[test]
fn fdt_with_virtio_devices() {
    let mut config = minimal_config();
    config.virtio_devices = vec![
        VirtioDeviceInfo {
            base_addr: memory::virtio_mmio_addr(0),
            irq: memory::virtio_mmio_irq(0),
        },
        VirtioDeviceInfo {
            base_addr: memory::virtio_mmio_addr(1),
            irq: memory::virtio_mmio_irq(1),
        },
        VirtioDeviceInfo {
            base_addr: memory::virtio_mmio_addr(2),
            irq: memory::virtio_mmio_irq(2),
        },
    ];
    let blob = build_fdt(&config).unwrap();
    assert!(!blob.is_empty());
}

#[test]
fn fdt_with_many_virtio_devices() {
    let mut config = minimal_config();
    config.virtio_devices = (0..memory::VIRTIO_MMIO_MAX_DEVICES)
        .map(|i| VirtioDeviceInfo {
            base_addr: memory::virtio_mmio_addr(i),
            irq: memory::virtio_mmio_irq(i),
        })
        .collect();
    let blob = build_fdt(&config).unwrap();
    // With 32 devices, FDT should still be < 64KB
    assert!(
        blob.len() < 65536,
        "FDT too large with 32 devices: {}",
        blob.len()
    );
}

// -----------------------------------------------------------------------
// RAM size variations
// -----------------------------------------------------------------------

#[test]
fn fdt_256mb_ram() {
    let mut config = minimal_config();
    config.ram_size = 256 * 1024 * 1024;
    let blob = build_fdt(&config).unwrap();
    assert!(!blob.is_empty());
}

#[test]
fn fdt_16gb_ram() {
    let mut config = minimal_config();
    config.ram_size = 16 * 1024 * 1024 * 1024u64;
    let blob = build_fdt(&config).unwrap();
    assert!(!blob.is_empty());
}

// -----------------------------------------------------------------------
// Full config (everything at once)
// -----------------------------------------------------------------------

#[test]
fn fdt_full_config() {
    let config = FdtConfig {
        ram_base: memory::RAM_BASE,
        ram_size: 4 * 1024 * 1024 * 1024,
        cpu_count: 4,
        cmdline: "console=hvc0 root=/dev/vda ro init_on_alloc=1 slab_nomerge".to_string(),
        initrd_start: 0x1_3000_0000,
        initrd_end: 0x1_3500_0000,
        virtio_devices: vec![
            VirtioDeviceInfo {
                base_addr: memory::virtio_mmio_addr(0),
                irq: memory::virtio_mmio_irq(0),
            },
            VirtioDeviceInfo {
                base_addr: memory::virtio_mmio_addr(1),
                irq: memory::virtio_mmio_irq(1),
            },
        ],
    };
    let blob = build_fdt(&config).unwrap();
    assert!(!blob.is_empty());
    // Should be well under 1MB
    assert!(blob.len() < 1024 * 1024);
}
