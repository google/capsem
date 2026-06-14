//! KVM checkpoint file read/write.
//!
//! Capsem controls guest quiescence, so KVM checkpoints store parked vCPU state
//! first, followed by a raw guest RAM image.

use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use super::memory::GuestMemory;
#[cfg(all(target_arch = "x86_64", test))]
use super::sys::KVM_MP_STATE_RUNNABLE;
#[cfg(target_arch = "x86_64")]
use super::sys::{
    KvmClockData, KvmDebugRegs, KvmFpu, KvmIrqchip, KvmLapicState, KvmMpState, KvmMsrEntry,
    KvmPitState2, KvmRegs, KvmSregs, KvmVcpuEvents, KvmXcrs, KvmXsave, VcpuFd, VmFd,
    KVM_IRQCHIP_IOAPIC, KVM_IRQCHIP_PIC_MASTER, KVM_IRQCHIP_PIC_SLAVE,
};
#[cfg(target_arch = "x86_64")]
use super::virtio_mmio::{QueueSnapshot, VirtioMmioSnapshot};

const MAGIC: &[u8; 16] = b"CAPSEM-KVM-CKPT\0";
const VERSION: u32 = 7;
const HEADER_LEN: u64 = 16 + 4 + 4 + 8 + 4 + 4 + 4;
const COPY_CHUNK_SIZE: usize = 1024 * 1024;
#[cfg(target_arch = "x86_64")]
const SELECTED_MSR_INDEXES: &[u32] = &[
    0x0000_0010, // IA32_TSC
    0x0000_0011, // KVM_WALL_CLOCK
    0x0000_0012, // KVM_SYSTEM_TIME
    0x0000_001b, // IA32_APIC_BASE
    0x0000_0174, // IA32_SYSENTER_CS
    0x0000_0175, // IA32_SYSENTER_ESP
    0x0000_0176, // IA32_SYSENTER_EIP
    0x0000_0277, // IA32_PAT
    0x0000_06e0, // IA32_TSC_DEADLINE
    0xc000_0081, // IA32_STAR
    0xc000_0082, // IA32_LSTAR
    0xc000_0083, // IA32_CSTAR
    0xc000_0084, // IA32_FMASK
    0xc000_0100, // FS.base
    0xc000_0101, // GS.base
    0xc000_0102, // KernelGSBase
    0xc000_0103, // TSC_AUX
    0x4b56_4d00, // KVM_WALL_CLOCK_NEW
    0x4b56_4d01, // KVM_SYSTEM_TIME_NEW
    0x4b56_4d02, // KVM_ASYNC_PF_EN
    0x4b56_4d03, // KVM_STEAL_TIME
    0x4b56_4d04, // KVM_PV_EOI_EN
    0x4b56_4d05, // KVM_PV_UNHALT
];
#[cfg(target_arch = "x86_64")]
const X86_VCPU_STATE_LEN: u32 = (std::mem::size_of::<KvmRegs>()
    + std::mem::size_of::<KvmSregs>()
    + std::mem::size_of::<KvmMpState>()
    + std::mem::size_of::<u32>()
    + SELECTED_MSR_INDEXES.len() * std::mem::size_of::<KvmMsrEntry>()
    + std::mem::size_of::<KvmLapicState>()
    + std::mem::size_of::<KvmVcpuEvents>()
    + std::mem::size_of::<KvmDebugRegs>()
    + std::mem::size_of::<KvmFpu>()
    + std::mem::size_of::<KvmXcrs>()
    + std::mem::size_of::<KvmXsave>()) as u32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct CheckpointHeader {
    pub version: u32,
    pub arch: [u8; 4],
    pub ram_bytes: u64,
    pub vcpu_count: u32,
    pub vcpu_state_len: u32,
    pub mmio_device_count: u32,
}

impl CheckpointHeader {
    #[cfg(target_arch = "x86_64")]
    pub fn current(ram_bytes: u64, vcpu_count: u32, mmio_device_count: u32) -> Self {
        Self {
            version: VERSION,
            arch: arch_tag(),
            ram_bytes,
            vcpu_count,
            vcpu_state_len: X86_VCPU_STATE_LEN,
            mmio_device_count,
        }
    }

    fn encode(self) -> [u8; HEADER_LEN as usize] {
        let mut out = [0u8; HEADER_LEN as usize];
        out[..16].copy_from_slice(MAGIC);
        out[16..20].copy_from_slice(&self.version.to_le_bytes());
        out[20..24].copy_from_slice(&self.arch);
        out[24..32].copy_from_slice(&self.ram_bytes.to_le_bytes());
        out[32..36].copy_from_slice(&self.vcpu_count.to_le_bytes());
        out[36..40].copy_from_slice(&self.vcpu_state_len.to_le_bytes());
        out[40..44].copy_from_slice(&self.mmio_device_count.to_le_bytes());
        out
    }

    fn decode(buf: &[u8]) -> Result<Self> {
        if buf.len() < HEADER_LEN as usize {
            bail!("checkpoint header too short");
        }
        if &buf[..16] != MAGIC {
            bail!("bad checkpoint magic");
        }
        let version = u32::from_le_bytes(buf[16..20].try_into().unwrap());
        let arch = buf[20..24].try_into().unwrap();
        let ram_bytes = u64::from_le_bytes(buf[24..32].try_into().unwrap());
        let vcpu_count = u32::from_le_bytes(buf[32..36].try_into().unwrap());
        let vcpu_state_len = u32::from_le_bytes(buf[36..40].try_into().unwrap());
        let mmio_device_count = u32::from_le_bytes(buf[40..44].try_into().unwrap());
        Ok(Self {
            version,
            arch,
            ram_bytes,
            vcpu_count,
            vcpu_state_len,
            mmio_device_count,
        })
    }
}

#[cfg(target_arch = "x86_64")]
#[derive(Debug, Clone)]
pub(super) struct VcpuSnapshot {
    pub id: u32,
    pub regs: KvmRegs,
    pub sregs: KvmSregs,
    pub mp_state: KvmMpState,
    pub msrs: Vec<KvmMsrEntry>,
    pub lapic: KvmLapicState,
    pub events: KvmVcpuEvents,
    pub debugregs: KvmDebugRegs,
    pub fpu: KvmFpu,
    pub xcrs: KvmXcrs,
    pub xsave: KvmXsave,
}

#[cfg(target_arch = "x86_64")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct VmSnapshot {
    pub irqchips: [KvmIrqchip; 3],
    pub pit2: KvmPitState2,
    pub clock: KvmClockData,
}

#[cfg(target_arch = "x86_64")]
impl Default for VmSnapshot {
    fn default() -> Self {
        Self {
            irqchips: [
                KvmIrqchip {
                    chip_id: KVM_IRQCHIP_PIC_MASTER,
                    ..Default::default()
                },
                KvmIrqchip {
                    chip_id: KVM_IRQCHIP_PIC_SLAVE,
                    ..Default::default()
                },
                KvmIrqchip {
                    chip_id: KVM_IRQCHIP_IOAPIC,
                    ..Default::default()
                },
            ],
            pit2: KvmPitState2::default(),
            clock: KvmClockData::default(),
        }
    }
}

#[cfg(target_arch = "x86_64")]
#[derive(Debug)]
pub(super) struct RestoredCheckpoint {
    pub vcpus: Vec<VcpuSnapshot>,
    pub vm: VmSnapshot,
    pub mmio_devices: Vec<MmioDeviceSnapshot>,
}

#[cfg(target_arch = "x86_64")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct MmioDeviceSnapshot {
    pub slot: u32,
    pub transport: VirtioMmioSnapshot,
}

#[cfg(target_arch = "x86_64")]
pub(super) fn snapshot_vcpu(vcpu: &VcpuFd) -> Result<VcpuSnapshot> {
    Ok(VcpuSnapshot {
        id: vcpu.id(),
        regs: vcpu.get_regs()?,
        sregs: vcpu.get_sregs()?,
        mp_state: vcpu.get_mp_state()?,
        msrs: vcpu.get_msrs(SELECTED_MSR_INDEXES)?,
        lapic: vcpu.get_lapic()?,
        events: vcpu.get_vcpu_events()?,
        debugregs: vcpu.get_debugregs()?,
        fpu: vcpu.get_fpu()?,
        xcrs: vcpu.get_xcrs()?,
        xsave: vcpu.get_xsave()?,
    })
}

#[cfg(target_arch = "x86_64")]
pub(super) fn restore_vcpus(vcpu_fds: &[VcpuFd], snapshots: &[VcpuSnapshot]) -> Result<()> {
    if vcpu_fds.len() != snapshots.len() {
        bail!(
            "checkpoint vCPU count mismatch: checkpoint={}, vm={}",
            snapshots.len(),
            vcpu_fds.len()
        );
    }
    for (vcpu, snapshot) in vcpu_fds.iter().zip(snapshots) {
        if vcpu.id() != snapshot.id {
            bail!(
                "checkpoint vCPU id mismatch: checkpoint={}, vm={}",
                snapshot.id,
                vcpu.id()
            );
        }
        vcpu.set_xsave(&snapshot.xsave)?;
        vcpu.set_xcrs(&snapshot.xcrs)?;
        vcpu.set_fpu(&snapshot.fpu)?;
        vcpu.set_debugregs(&snapshot.debugregs)?;
        vcpu.set_lapic(&snapshot.lapic)?;
        vcpu.set_sregs(&snapshot.sregs)?;
        vcpu.set_regs(&snapshot.regs)?;
        vcpu.set_vcpu_events(&snapshot.events)?;
        vcpu.set_msrs(&snapshot.msrs)?;
        vcpu.set_mp_state(snapshot.mp_state)?;
    }
    Ok(())
}

#[cfg(target_arch = "x86_64")]
pub(super) fn snapshot_vm(vm: &VmFd) -> Result<VmSnapshot> {
    Ok(VmSnapshot {
        irqchips: [
            vm.get_irqchip(KVM_IRQCHIP_PIC_MASTER)?,
            vm.get_irqchip(KVM_IRQCHIP_PIC_SLAVE)?,
            vm.get_irqchip(KVM_IRQCHIP_IOAPIC)?,
        ],
        pit2: vm.get_pit2()?,
        clock: vm.get_clock()?,
    })
}

#[cfg(target_arch = "x86_64")]
pub(super) fn restore_vm(vm: &VmFd, snapshot: &VmSnapshot) -> Result<()> {
    for irqchip in &snapshot.irqchips {
        vm.set_irqchip(irqchip)?;
    }
    vm.set_pit2(&snapshot.pit2)?;
    vm.set_clock(&snapshot.clock)?;
    Ok(())
}

#[cfg(target_arch = "x86_64")]
pub(super) fn write_checkpoint(
    path: &Path,
    memory: &GuestMemory,
    vcpus: &[VcpuSnapshot],
    vm: &VmSnapshot,
    mmio_devices: &[MmioDeviceSnapshot],
) -> Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .context("checkpoint path must have a parent directory")?;
    if !parent.is_dir() {
        bail!(
            "checkpoint parent directory does not exist: {}",
            parent.display()
        );
    }

    let tmp_path = temp_path_for(path);
    let write_result = write_checkpoint_inner(&tmp_path, memory, vcpus, vm, mmio_devices);
    if let Err(err) = write_result {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(err);
    }

    std::fs::rename(&tmp_path, path).with_context(|| {
        format!(
            "rename checkpoint {} -> {}",
            tmp_path.display(),
            path.display()
        )
    })?;

    Ok(())
}

#[cfg(target_arch = "x86_64")]
pub(super) fn read_checkpoint(
    path: &Path,
    memory: &GuestMemory,
    expected_vcpu_count: u32,
    expected_mmio_device_count: u32,
) -> Result<RestoredCheckpoint> {
    let file = std::fs::File::open(path)
        .with_context(|| format!("open KVM checkpoint: {}", path.display()))?;
    let mut reader = BufReader::new(file);
    let mut header_bytes = [0u8; HEADER_LEN as usize];
    reader
        .read_exact(&mut header_bytes)
        .context("read checkpoint header")?;
    let header = CheckpointHeader::decode(&header_bytes)?;
    validate_header(
        &header,
        memory.size(),
        expected_vcpu_count,
        expected_mmio_device_count,
    )?;

    let mut vcpus = Vec::with_capacity(header.vcpu_count as usize);
    for id in 0..header.vcpu_count {
        vcpus.push(read_vcpu_snapshot(&mut reader, id)?);
    }

    let vm = read_vm_snapshot(&mut reader)?;

    let mut mmio_devices = Vec::with_capacity(header.mmio_device_count as usize);
    for _ in 0..header.mmio_device_count {
        mmio_devices.push(read_mmio_device_snapshot(&mut reader)?);
    }

    let mut offset = 0u64;
    let mut buf = vec![0u8; COPY_CHUNK_SIZE.min(memory.size() as usize)];
    while offset < memory.size() {
        let len = (memory.size() - offset).min(buf.len() as u64) as usize;
        reader
            .read_exact(&mut buf[..len])
            .context("read checkpoint memory")?;
        memory
            .write_at(offset, &buf[..len])
            .context("restore checkpoint memory")?;
        offset += len as u64;
    }

    let mut trailing = [0u8; 1];
    if reader
        .read(&mut trailing)
        .context("check checkpoint length")?
        != 0
    {
        bail!("checkpoint has trailing bytes");
    }

    Ok(RestoredCheckpoint {
        vcpus,
        vm,
        mmio_devices,
    })
}

#[cfg(target_arch = "x86_64")]
fn write_checkpoint_inner(
    path: &Path,
    memory: &GuestMemory,
    vcpus: &[VcpuSnapshot],
    vm: &VmSnapshot,
    mmio_devices: &[MmioDeviceSnapshot],
) -> Result<()> {
    let file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .with_context(|| format!("create checkpoint temp file: {}", path.display()))?;
    let mut writer = BufWriter::new(file);

    let header =
        CheckpointHeader::current(memory.size(), vcpus.len() as u32, mmio_devices.len() as u32);
    writer
        .write_all(&header.encode())
        .context("write checkpoint header")?;
    for snapshot in vcpus {
        write_vcpu_snapshot(&mut writer, snapshot)?;
    }
    write_vm_snapshot(&mut writer, vm)?;
    for snapshot in mmio_devices {
        write_mmio_device_snapshot(&mut writer, snapshot)?;
    }

    let mut offset = 0u64;
    let mut buf = vec![0u8; COPY_CHUNK_SIZE.min(memory.size() as usize)];
    while offset < memory.size() {
        let len = (memory.size() - offset).min(buf.len() as u64) as usize;
        memory
            .read_at(offset, &mut buf[..len])
            .context("read guest memory for checkpoint")?;
        writer
            .write_all(&buf[..len])
            .context("write guest memory checkpoint")?;
        offset += len as u64;
    }

    writer.flush().context("flush checkpoint")?;
    writer
        .get_ref()
        .sync_all()
        .context("sync checkpoint temp file")?;
    Ok(())
}

#[cfg(target_arch = "x86_64")]
fn validate_header(
    header: &CheckpointHeader,
    ram_bytes: u64,
    vcpu_count: u32,
    mmio_device_count: u32,
) -> Result<()> {
    if header.version != VERSION {
        bail!(
            "unsupported KVM checkpoint version: got {}, expected {}",
            header.version,
            VERSION
        );
    }
    if header.arch != arch_tag() {
        bail!("KVM checkpoint architecture does not match this host");
    }
    if header.ram_bytes != ram_bytes {
        bail!(
            "checkpoint RAM size mismatch: checkpoint={}, vm={}",
            header.ram_bytes,
            ram_bytes
        );
    }
    if header.vcpu_count != vcpu_count {
        bail!(
            "checkpoint vCPU count mismatch: checkpoint={}, vm={}",
            header.vcpu_count,
            vcpu_count
        );
    }
    if header.mmio_device_count != mmio_device_count {
        bail!(
            "checkpoint MMIO device count mismatch: checkpoint={}, vm={}",
            header.mmio_device_count,
            mmio_device_count
        );
    }
    if header.vcpu_state_len != X86_VCPU_STATE_LEN {
        bail!(
            "checkpoint vCPU state size mismatch: checkpoint={}, expected={}",
            header.vcpu_state_len,
            X86_VCPU_STATE_LEN
        );
    }
    Ok(())
}

#[cfg(target_arch = "x86_64")]
fn write_vcpu_snapshot(writer: &mut impl Write, snapshot: &VcpuSnapshot) -> Result<()> {
    writer
        .write_all(&snapshot.id.to_le_bytes())
        .context("write checkpoint vCPU id")?;
    write_pod(writer, &snapshot.regs).context("write checkpoint vCPU regs")?;
    write_pod(writer, &snapshot.sregs).context("write checkpoint vCPU sregs")?;
    write_pod(writer, &snapshot.mp_state).context("write checkpoint vCPU mp_state")?;
    if snapshot.msrs.len() > SELECTED_MSR_INDEXES.len() {
        bail!(
            "checkpoint vCPU MSR count exceeds selected set: {} > {}",
            snapshot.msrs.len(),
            SELECTED_MSR_INDEXES.len()
        );
    }
    writer
        .write_all(&(snapshot.msrs.len() as u32).to_le_bytes())
        .context("write checkpoint vCPU MSR count")?;
    for entry in &snapshot.msrs {
        write_pod(writer, entry).context("write checkpoint vCPU MSR entry")?;
    }
    for _ in snapshot.msrs.len()..SELECTED_MSR_INDEXES.len() {
        write_pod(writer, &KvmMsrEntry::default()).context("write checkpoint vCPU MSR padding")?;
    }
    write_pod(writer, &snapshot.lapic).context("write checkpoint vCPU LAPIC state")?;
    write_pod(writer, &snapshot.events).context("write checkpoint vCPU events")?;
    write_pod(writer, &snapshot.debugregs).context("write checkpoint vCPU debug registers")?;
    write_pod(writer, &snapshot.fpu).context("write checkpoint vCPU FPU state")?;
    write_pod(writer, &snapshot.xcrs).context("write checkpoint vCPU XCR state")?;
    write_pod(writer, &snapshot.xsave).context("write checkpoint vCPU XSAVE state")?;
    Ok(())
}

#[cfg(target_arch = "x86_64")]
fn read_vcpu_snapshot(reader: &mut impl Read, expected_id: u32) -> Result<VcpuSnapshot> {
    let mut id_bytes = [0u8; 4];
    reader
        .read_exact(&mut id_bytes)
        .context("read checkpoint vCPU id")?;
    let id = u32::from_le_bytes(id_bytes);
    if id != expected_id {
        bail!("checkpoint vCPU id out of order: got {id}, expected {expected_id}");
    }
    Ok(VcpuSnapshot {
        id,
        regs: read_pod(reader).context("read checkpoint vCPU regs")?,
        sregs: read_pod(reader).context("read checkpoint vCPU sregs")?,
        mp_state: read_pod(reader).context("read checkpoint vCPU mp_state")?,
        msrs: {
            let mut count_bytes = [0u8; 4];
            reader
                .read_exact(&mut count_bytes)
                .context("read checkpoint vCPU MSR count")?;
            let count = u32::from_le_bytes(count_bytes) as usize;
            if count > SELECTED_MSR_INDEXES.len() {
                bail!(
                    "checkpoint vCPU MSR count exceeds selected set: {} > {}",
                    count,
                    SELECTED_MSR_INDEXES.len()
                );
            }
            let mut entries = Vec::with_capacity(count);
            for i in 0..SELECTED_MSR_INDEXES.len() {
                let entry: KvmMsrEntry =
                    read_pod(reader).context("read checkpoint vCPU MSR entry")?;
                if i < count {
                    entries.push(entry);
                }
            }
            entries
        },
        lapic: read_pod(reader).context("read checkpoint vCPU LAPIC state")?,
        events: read_pod(reader).context("read checkpoint vCPU events")?,
        debugregs: read_pod(reader).context("read checkpoint vCPU debug registers")?,
        fpu: read_pod(reader).context("read checkpoint vCPU FPU state")?,
        xcrs: read_pod(reader).context("read checkpoint vCPU XCR state")?,
        xsave: read_pod(reader).context("read checkpoint vCPU XSAVE state")?,
    })
}

#[cfg(target_arch = "x86_64")]
fn write_vm_snapshot(writer: &mut impl Write, snapshot: &VmSnapshot) -> Result<()> {
    for irqchip in &snapshot.irqchips {
        write_pod(writer, irqchip).context("write checkpoint IRQCHIP state")?;
    }
    write_pod(writer, &snapshot.pit2).context("write checkpoint PIT state")?;
    write_pod(writer, &snapshot.clock).context("write checkpoint KVM clock state")?;
    Ok(())
}

#[cfg(target_arch = "x86_64")]
fn read_vm_snapshot(reader: &mut impl Read) -> Result<VmSnapshot> {
    Ok(VmSnapshot {
        irqchips: [
            read_pod(reader).context("read checkpoint PIC master state")?,
            read_pod(reader).context("read checkpoint PIC slave state")?,
            read_pod(reader).context("read checkpoint IOAPIC state")?,
        ],
        pit2: read_pod(reader).context("read checkpoint PIT state")?,
        clock: read_pod(reader).context("read checkpoint KVM clock state")?,
    })
}

#[cfg(target_arch = "x86_64")]
fn write_mmio_device_snapshot(
    writer: &mut impl Write,
    snapshot: &MmioDeviceSnapshot,
) -> Result<()> {
    writer
        .write_all(&snapshot.slot.to_le_bytes())
        .context("write checkpoint MMIO slot")?;
    write_u32(writer, snapshot.transport.status).context("write checkpoint MMIO status")?;
    write_u32(writer, snapshot.transport.features_sel)
        .context("write checkpoint MMIO features_sel")?;
    write_u64(writer, snapshot.transport.driver_features)
        .context("write checkpoint MMIO driver_features")?;
    write_u32(writer, snapshot.transport.driver_features_sel)
        .context("write checkpoint MMIO driver_features_sel")?;
    write_u32(writer, snapshot.transport.queue_sel).context("write checkpoint MMIO queue_sel")?;
    write_u32(writer, snapshot.transport.interrupt_status)
        .context("write checkpoint MMIO interrupt_status")?;
    write_u32(writer, snapshot.transport.config_generation)
        .context("write checkpoint MMIO config_generation")?;
    writer
        .write_all(&[u8::from(snapshot.transport.activated)])
        .context("write checkpoint MMIO activated")?;
    write_u32(writer, snapshot.transport.queues.len() as u32)
        .context("write checkpoint MMIO queue count")?;
    for queue in &snapshot.transport.queues {
        write_queue_snapshot(writer, queue)?;
    }
    Ok(())
}

#[cfg(target_arch = "x86_64")]
fn read_mmio_device_snapshot(reader: &mut impl Read) -> Result<MmioDeviceSnapshot> {
    let slot = read_u32(reader).context("read checkpoint MMIO slot")?;
    let status = read_u32(reader).context("read checkpoint MMIO status")?;
    let features_sel = read_u32(reader).context("read checkpoint MMIO features_sel")?;
    let driver_features = read_u64(reader).context("read checkpoint MMIO driver_features")?;
    let driver_features_sel =
        read_u32(reader).context("read checkpoint MMIO driver_features_sel")?;
    let queue_sel = read_u32(reader).context("read checkpoint MMIO queue_sel")?;
    let interrupt_status = read_u32(reader).context("read checkpoint MMIO interrupt_status")?;
    let config_generation = read_u32(reader).context("read checkpoint MMIO config_generation")?;
    let mut activated = [0u8; 1];
    reader
        .read_exact(&mut activated)
        .context("read checkpoint MMIO activated")?;
    let queue_count = read_u32(reader).context("read checkpoint MMIO queue count")?;
    let mut queues = Vec::with_capacity(queue_count as usize);
    for _ in 0..queue_count {
        queues.push(read_queue_snapshot(reader)?);
    }
    Ok(MmioDeviceSnapshot {
        slot,
        transport: VirtioMmioSnapshot {
            status,
            features_sel,
            driver_features,
            driver_features_sel,
            queue_sel,
            queues,
            interrupt_status,
            config_generation,
            activated: activated[0] != 0,
        },
    })
}

#[cfg(target_arch = "x86_64")]
fn write_queue_snapshot(writer: &mut impl Write, queue: &QueueSnapshot) -> Result<()> {
    write_u16(writer, queue.num)?;
    writer.write_all(&[u8::from(queue.ready)])?;
    write_u32(writer, queue.desc_lo)?;
    write_u32(writer, queue.desc_hi)?;
    write_u32(writer, queue.driver_lo)?;
    write_u32(writer, queue.driver_hi)?;
    write_u32(writer, queue.device_lo)?;
    write_u32(writer, queue.device_hi)?;
    Ok(())
}

#[cfg(target_arch = "x86_64")]
fn read_queue_snapshot(reader: &mut impl Read) -> Result<QueueSnapshot> {
    let num = read_u16(reader)?;
    let mut ready = [0u8; 1];
    reader.read_exact(&mut ready)?;
    Ok(QueueSnapshot {
        num,
        ready: ready[0] != 0,
        desc_lo: read_u32(reader)?,
        desc_hi: read_u32(reader)?,
        driver_lo: read_u32(reader)?,
        driver_hi: read_u32(reader)?,
        device_lo: read_u32(reader)?,
        device_hi: read_u32(reader)?,
    })
}

#[cfg(target_arch = "x86_64")]
fn write_u16(writer: &mut impl Write, value: u16) -> Result<()> {
    writer.write_all(&value.to_le_bytes())?;
    Ok(())
}

#[cfg(target_arch = "x86_64")]
fn write_u32(writer: &mut impl Write, value: u32) -> Result<()> {
    writer.write_all(&value.to_le_bytes())?;
    Ok(())
}

#[cfg(target_arch = "x86_64")]
fn write_u64(writer: &mut impl Write, value: u64) -> Result<()> {
    writer.write_all(&value.to_le_bytes())?;
    Ok(())
}

#[cfg(target_arch = "x86_64")]
fn read_u16(reader: &mut impl Read) -> Result<u16> {
    let mut bytes = [0u8; 2];
    reader.read_exact(&mut bytes)?;
    Ok(u16::from_le_bytes(bytes))
}

#[cfg(target_arch = "x86_64")]
fn read_u32(reader: &mut impl Read) -> Result<u32> {
    let mut bytes = [0u8; 4];
    reader.read_exact(&mut bytes)?;
    Ok(u32::from_le_bytes(bytes))
}

#[cfg(target_arch = "x86_64")]
fn read_u64(reader: &mut impl Read) -> Result<u64> {
    let mut bytes = [0u8; 8];
    reader.read_exact(&mut bytes)?;
    Ok(u64::from_le_bytes(bytes))
}

#[cfg(target_arch = "x86_64")]
fn write_pod<T>(writer: &mut impl Write, value: &T) -> Result<()> {
    let bytes = unsafe {
        std::slice::from_raw_parts(value as *const T as *const u8, std::mem::size_of::<T>())
    };
    writer.write_all(bytes)?;
    Ok(())
}

#[cfg(target_arch = "x86_64")]
fn read_pod<T: Copy>(reader: &mut impl Read) -> Result<T> {
    let mut value = std::mem::MaybeUninit::<T>::zeroed();
    let bytes = unsafe {
        std::slice::from_raw_parts_mut(value.as_mut_ptr() as *mut u8, std::mem::size_of::<T>())
    };
    reader.read_exact(bytes)?;
    Ok(unsafe { value.assume_init() })
}

fn temp_path_for(path: &Path) -> PathBuf {
    let mut name = path
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_else(|| "checkpoint".into());
    name.push(format!(".tmp.{}", std::process::id()));
    path.with_file_name(name)
}

const fn arch_tag() -> [u8; 4] {
    #[cfg(target_arch = "x86_64")]
    {
        *b"x64\0"
    }
    #[cfg(target_arch = "aarch64")]
    {
        *b"arm\0"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_header() -> CheckpointHeader {
        CheckpointHeader {
            version: VERSION,
            arch: arch_tag(),
            ram_bytes: 4096,
            vcpu_count: 2,
            vcpu_state_len: 0,
            mmio_device_count: 3,
        }
    }

    #[cfg(target_arch = "x86_64")]
    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join("capsem-kvm-checkpoint")
            .join(name);
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn header_roundtrips() {
        let header = test_header();
        let decoded = CheckpointHeader::decode(&header.encode()).unwrap();
        assert_eq!(decoded, header);
        assert_eq!(decoded.version, VERSION);
        assert_eq!(decoded.ram_bytes, 4096);
        assert_eq!(decoded.vcpu_count, 2);
        #[cfg(target_arch = "x86_64")]
        assert_eq!(decoded.vcpu_state_len, X86_VCPU_STATE_LEN);
        #[cfg(not(target_arch = "x86_64"))]
        assert_eq!(decoded.vcpu_state_len, 0);
        assert_eq!(decoded.mmio_device_count, 3);
    }

    #[test]
    fn header_rejects_bad_magic() {
        let mut encoded = test_header().encode();
        encoded[0] = b'X';
        let err = CheckpointHeader::decode(&encoded).unwrap_err();
        assert!(err.to_string().contains("bad checkpoint magic"));
    }

    #[cfg(target_arch = "x86_64")]
    fn snapshot(id: u32) -> VcpuSnapshot {
        let regs = KvmRegs {
            rax: id as u64 + 10,
            rip: 0x1000 + id as u64,
            ..Default::default()
        };
        let sregs = KvmSregs {
            cr3: 0x2000 + id as u64,
            ..Default::default()
        };
        let mp_state = KvmMpState {
            mp_state: KVM_MP_STATE_RUNNABLE,
        };
        VcpuSnapshot {
            id,
            regs,
            sregs,
            mp_state,
            msrs: vec![KvmMsrEntry {
                index: 0x6e0,
                reserved: 0,
                data: 0x1000 + id as u64,
            }],
            lapic: KvmLapicState::default(),
            events: KvmVcpuEvents::default(),
            debugregs: KvmDebugRegs::default(),
            fpu: KvmFpu::default(),
            xcrs: KvmXcrs::default(),
            xsave: KvmXsave::default(),
        }
    }

    #[cfg(target_arch = "x86_64")]
    fn vm_snapshot() -> VmSnapshot {
        let mut pic_master = KvmIrqchip {
            chip_id: KVM_IRQCHIP_PIC_MASTER,
            ..Default::default()
        };
        pic_master.chip[0] = 1;
        let mut pic_slave = KvmIrqchip {
            chip_id: KVM_IRQCHIP_PIC_SLAVE,
            ..Default::default()
        };
        pic_slave.chip[0] = 2;
        let mut ioapic = KvmIrqchip {
            chip_id: KVM_IRQCHIP_IOAPIC,
            ..Default::default()
        };
        ioapic.chip[0] = 3;
        let mut pit2 = KvmPitState2::default();
        pit2.bytes[0] = 4;
        let mut clock = KvmClockData::default();
        clock.bytes[0] = 5;
        VmSnapshot {
            irqchips: [pic_master, pic_slave, ioapic],
            pit2,
            clock,
        }
    }

    #[cfg(target_arch = "x86_64")]
    fn mmio(slot: u32) -> MmioDeviceSnapshot {
        MmioDeviceSnapshot {
            slot,
            transport: VirtioMmioSnapshot {
                status: 0xf,
                features_sel: 1,
                driver_features: 0x1000_0000,
                driver_features_sel: 0,
                queue_sel: 1,
                queues: vec![QueueSnapshot {
                    num: 16,
                    ready: true,
                    desc_lo: 0x1000,
                    desc_hi: 0,
                    driver_lo: 0x2000,
                    driver_hi: 0,
                    device_lo: 0x3000,
                    device_hi: 0,
                }],
                interrupt_status: 1,
                config_generation: 2,
                activated: true,
            },
        }
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn writes_header_and_memory() {
        let dir = temp_dir("writes-header-memory");
        let path = dir.join("state.kvm");
        let mem = GuestMemory::new(8192).unwrap();
        mem.write_at(0, b"hello").unwrap();
        mem.write_at(4096, b"world").unwrap();

        write_checkpoint(
            &path,
            &mem,
            &[snapshot(0), snapshot(1)],
            &vm_snapshot(),
            &[mmio(0)],
        )
        .unwrap();

        let bytes = std::fs::read(path).unwrap();
        let header = CheckpointHeader::decode(&bytes[..HEADER_LEN as usize]).unwrap();
        assert_eq!(header.ram_bytes, 8192);
        let memory_offset = bytes.len() - 8192;
        assert_eq!(&bytes[memory_offset..memory_offset + 5], b"hello");
        assert_eq!(&bytes[memory_offset + 4096..memory_offset + 4101], b"world");
        assert_eq!(bytes.len(), memory_offset + 8192);
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn restores_memory_and_vcpu_state() {
        let dir = temp_dir("restore-memory-vcpu");
        let path = dir.join("state.kvm");
        let mem = GuestMemory::new(8192).unwrap();
        mem.write_at(0, b"hello").unwrap();
        mem.write_at(4096, b"world").unwrap();
        write_checkpoint(
            &path,
            &mem,
            &[snapshot(0), snapshot(1)],
            &vm_snapshot(),
            &[mmio(3)],
        )
        .unwrap();

        let restored_mem = GuestMemory::new(8192).unwrap();
        let restored = read_checkpoint(&path, &restored_mem, 2, 1).unwrap();

        let mut buf = [0u8; 5];
        restored_mem.read_at(0, &mut buf).unwrap();
        assert_eq!(&buf, b"hello");
        restored_mem.read_at(4096, &mut buf).unwrap();
        assert_eq!(&buf, b"world");
        assert_eq!(restored.vcpus.len(), 2);
        assert_eq!(restored.vcpus[1].regs.rip, 0x1001);
        assert_eq!(restored.vcpus[1].sregs.cr3, 0x2001);
        assert_eq!(restored.vcpus[1].mp_state.mp_state, KVM_MP_STATE_RUNNABLE);
        assert_eq!(restored.vcpus[1].msrs[0].index, 0x6e0);
        assert_eq!(restored.vcpus[1].msrs[0].data, 0x1001);
        assert_eq!(restored.vm, vm_snapshot());
        assert_eq!(restored.mmio_devices, vec![mmio(3)]);
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn overwrites_atomically() {
        let dir = temp_dir("atomic-overwrite");
        let path = dir.join("state.kvm");
        std::fs::write(&path, b"old").unwrap();
        let mem = GuestMemory::new(4096).unwrap();

        write_checkpoint(&path, &mem, &[snapshot(0)], &vm_snapshot(), &[]).unwrap();

        let bytes = std::fs::read(path).unwrap();
        assert_ne!(&bytes, b"old");
        assert_eq!(
            bytes.len(),
            HEADER_LEN as usize
                + 4
                + X86_VCPU_STATE_LEN as usize
                + (3 * std::mem::size_of::<KvmIrqchip>())
                + std::mem::size_of::<KvmPitState2>()
                + std::mem::size_of::<KvmClockData>()
                + 4096
        );
        assert!(std::fs::read_dir(&dir).unwrap().all(|e| !e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains(".tmp.")));
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn rejects_missing_parent() {
        let dir = temp_dir("missing-parent");
        let path = dir.join("missing").join("state.kvm");
        let mem = GuestMemory::new(4096).unwrap();

        let err = write_checkpoint(&path, &mem, &[snapshot(0)], &vm_snapshot(), &[]).unwrap_err();

        assert!(err
            .to_string()
            .contains("checkpoint parent directory does not exist"));
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn removes_temp_file_after_create_failure() {
        let dir = temp_dir("temp-cleanup");
        let path = dir.join("state.kvm");
        let tmp = temp_path_for(&path);
        std::fs::write(&tmp, b"conflict").unwrap();
        let mem = GuestMemory::new(4096).unwrap();

        let err = write_checkpoint(&path, &mem, &[snapshot(0)], &vm_snapshot(), &[]).unwrap_err();

        assert!(err.to_string().contains("create checkpoint temp file"));
        assert!(!tmp.exists());
        assert!(!path.exists());
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn restore_rejects_wrong_ram_size() {
        let dir = temp_dir("wrong-ram-size");
        let path = dir.join("state.kvm");
        let mem = GuestMemory::new(4096).unwrap();
        write_checkpoint(&path, &mem, &[snapshot(0)], &vm_snapshot(), &[]).unwrap();
        let larger_mem = GuestMemory::new(8192).unwrap();

        let err = read_checkpoint(&path, &larger_mem, 1, 0).unwrap_err();

        assert!(err.to_string().contains("checkpoint RAM size mismatch"));
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn restore_rejects_wrong_vcpu_count() {
        let dir = temp_dir("wrong-vcpu-count");
        let path = dir.join("state.kvm");
        let mem = GuestMemory::new(4096).unwrap();
        write_checkpoint(&path, &mem, &[snapshot(0)], &vm_snapshot(), &[]).unwrap();

        let err = read_checkpoint(&path, &mem, 2, 0).unwrap_err();

        assert!(err.to_string().contains("checkpoint vCPU count mismatch"));
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn restore_rejects_trailing_bytes() {
        let dir = temp_dir("trailing-bytes");
        let path = dir.join("state.kvm");
        let mem = GuestMemory::new(4096).unwrap();
        write_checkpoint(&path, &mem, &[snapshot(0)], &vm_snapshot(), &[]).unwrap();
        std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b"extra")
            .unwrap();

        let err = read_checkpoint(&path, &mem, 1, 0).unwrap_err();

        assert!(err.to_string().contains("checkpoint has trailing bytes"));
    }
}
