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
mod tests;
