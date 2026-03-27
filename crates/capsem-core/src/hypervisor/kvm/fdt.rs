//! Flattened Device Tree (FDT) generation for aarch64 KVM guests.
//!
//! Builds the device tree blob that tells Linux about hardware:
//! memory, CPUs, GIC, timer, virtio MMIO devices.

use anyhow::{Context, Result};
use vm_fdt::FdtWriter;

use super::memory;

/// Configuration for FDT generation.
pub(super) struct FdtConfig {
    pub ram_base: u64,
    pub ram_size: u64,
    pub cpu_count: u32,
    pub cmdline: String,
    pub initrd_start: u64,
    pub initrd_end: u64,
    pub virtio_devices: Vec<VirtioDeviceInfo>,
}

/// Info about a virtio MMIO device to include in the FDT.
pub(super) struct VirtioDeviceInfo {
    pub base_addr: u64,
    pub irq: u32,
}

/// Build a complete FDT blob for an aarch64 KVM guest.
pub(super) fn build_fdt(config: &FdtConfig) -> Result<Vec<u8>> {
    if config.cpu_count == 0 {
        anyhow::bail!("FDT requires at least 1 CPU");
    }

    let mut fdt = FdtWriter::new().context("FdtWriter::new")?;

    // Root node
    let root = fdt.begin_node("").context("begin root")?;
    fdt.property_string("compatible", "linux,dummy-virt")?;
    fdt.property_string("model", "capsem-vm")?;
    fdt.property_u32("#address-cells", 2)?;
    fdt.property_u32("#size-cells", 2)?;
    // interrupt-parent phandle = 1 (GIC)
    fdt.property_u32("interrupt-parent", 1)?;

    // /chosen
    let chosen = fdt.begin_node("chosen")?;
    fdt.property_string("bootargs", &config.cmdline)?;
    if config.initrd_start != 0 && config.initrd_end > config.initrd_start {
        fdt.property_u64("linux,initrd-start", config.initrd_start)?;
        fdt.property_u64("linux,initrd-end", config.initrd_end)?;
    }
    // stdout-path points to the first virtio device (console)
    if !config.virtio_devices.is_empty() {
        fdt.property_string(
            "stdout-path",
            &format!("/virtio_mmio@{:x}", config.virtio_devices[0].base_addr),
        )?;
    }
    fdt.end_node(chosen)?;

    // /memory@{ram_base}
    let mem_node = fdt.begin_node(&format!("memory@{:x}", config.ram_base))?;
    fdt.property_string("device_type", "memory")?;
    // reg = <ram_base_hi ram_base_lo ram_size_hi ram_size_lo>
    fdt.property_array_u64("reg", &[config.ram_base, config.ram_size])?;
    fdt.end_node(mem_node)?;

    // /cpus
    let cpus = fdt.begin_node("cpus")?;
    fdt.property_u32("#address-cells", 1)?;
    fdt.property_u32("#size-cells", 0)?;
    for i in 0..config.cpu_count {
        let cpu = fdt.begin_node(&format!("cpu@{i}"))?;
        fdt.property_string("device_type", "cpu")?;
        fdt.property_string("compatible", "arm,arm-v8")?;
        fdt.property_u32("reg", i)?;
        fdt.property_string("enable-method", "psci")?;
        fdt.end_node(cpu)?;
    }
    fdt.end_node(cpus)?;

    // /psci
    let psci = fdt.begin_node("psci")?;
    fdt.property_string("compatible", "arm,psci-0.2")?;
    fdt.property_string("method", "hvc")?;
    fdt.end_node(psci)?;

    // /intc (GICv3) -- phandle 1
    let gic_redist_size = config.cpu_count as u64 * memory::GIC_REDIST_PER_CPU;
    let intc = fdt.begin_node(&format!("intc@{:x}", memory::GIC_DIST_BASE))?;
    fdt.property_string("compatible", "arm,gic-v3")?;
    fdt.property_u32("#interrupt-cells", 3)?;
    fdt.property_null("interrupt-controller")?;
    fdt.property_u32("phandle", 1)?;
    // reg = <dist_base dist_size redist_base redist_size>
    fdt.property_array_u64(
        "reg",
        &[
            memory::GIC_DIST_BASE,
            memory::GIC_DIST_SIZE,
            memory::GIC_REDIST_BASE,
            gic_redist_size,
        ],
    )?;
    fdt.end_node(intc)?;

    // /timer (ARM generic timer)
    let timer = fdt.begin_node("timer")?;
    fdt.property_string("compatible", "arm,armv8-timer")?;
    fdt.property_null("always-on")?;
    // interrupts: 4 PPIs (type=1) with standard numbers
    // Format: <type irq_num flags> for each PPI
    // PPI 13 (secure phys), PPI 14 (non-secure phys), PPI 11 (virt), PPI 10 (hyp)
    // Flags: 0xf04 = level-low, CPU mask 0xf
    #[allow(clippy::identity_op)]
    let timer_irqs: Vec<u32> = vec![
        1, 13, 0xf04, // secure physical timer
        1, 14, 0xf04, // non-secure physical timer
        1, 11, 0xf04, // virtual timer
        1, 10, 0xf04, // hypervisor timer
    ];
    fdt.property_array_u32("interrupts", &timer_irqs)?;
    fdt.end_node(timer)?;

    // /apb-pclk (fixed clock for PL011 compatibility, even though we use virtio-console)
    let clk = fdt.begin_node("apb-pclk")?;
    fdt.property_string("compatible", "fixed-clock")?;
    fdt.property_u32("#clock-cells", 0)?;
    fdt.property_u32("clock-frequency", 24000000)?;
    fdt.property_string("clock-output-names", "clk24mhz")?;
    fdt.property_u32("phandle", 2)?;
    fdt.end_node(clk)?;

    // Virtio MMIO devices
    for dev in &config.virtio_devices {
        let node = fdt.begin_node(&format!("virtio_mmio@{:x}", dev.base_addr))?;
        fdt.property_string("compatible", "virtio,mmio")?;
        fdt.property_array_u64("reg", &[dev.base_addr, memory::VIRTIO_MMIO_SIZE])?;
        // interrupts: <SPI irq_num edge_rising>
        // SPI type = 0, IRQ number = irq - 32 (SPI offset), flags = 1 (edge rising)
        let spi_num = dev.irq - 32;
        fdt.property_array_u32("interrupts", &[0, spi_num, 1])?;
        fdt.end_node(node)?;
    }

    fdt.end_node(root)?;

    fdt.finish().context("FDT finish")
}

#[cfg(test)]
mod tests {
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
        assert!(blob.len() < 65536, "FDT too large with 32 devices: {}", blob.len());
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
}
