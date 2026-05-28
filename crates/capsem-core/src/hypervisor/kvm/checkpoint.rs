//! KVM checkpoint file read/write.
//!
//! Capsem controls guest quiescence, so KVM checkpoints store parked vCPU state
//! first, followed by a raw guest RAM image.

use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use super::memory::GuestMemory;
#[cfg(target_arch = "x86_64")]
use super::sys::{KvmMpState, KvmRegs, KvmSregs, VcpuFd};
#[cfg(all(target_arch = "x86_64", test))]
use super::sys::KVM_MP_STATE_RUNNABLE;

const MAGIC: &[u8; 16] = b"CAPSEM-KVM-CKPT\0";
const VERSION: u32 = 3;
const HEADER_LEN: u64 = 16 + 4 + 4 + 8 + 4 + 4;
const COPY_CHUNK_SIZE: usize = 1024 * 1024;
#[cfg(target_arch = "x86_64")]
const X86_VCPU_STATE_LEN: u32 =
    (std::mem::size_of::<KvmRegs>()
        + std::mem::size_of::<KvmSregs>()
        + std::mem::size_of::<KvmMpState>()) as u32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct CheckpointHeader {
    pub version: u32,
    pub arch: [u8; 4],
    pub ram_bytes: u64,
    pub vcpu_count: u32,
    pub vcpu_state_len: u32,
}

impl CheckpointHeader {
    #[cfg(target_arch = "x86_64")]
    pub fn current(ram_bytes: u64, vcpu_count: u32) -> Self {
        Self {
            version: VERSION,
            arch: arch_tag(),
            ram_bytes,
            vcpu_count,
            vcpu_state_len: X86_VCPU_STATE_LEN,
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
        Ok(Self {
            version,
            arch,
            ram_bytes,
            vcpu_count,
            vcpu_state_len,
        })
    }
}

#[cfg(target_arch = "x86_64")]
#[derive(Debug, Clone, Copy)]
pub(super) struct VcpuSnapshot {
    pub id: u32,
    pub regs: KvmRegs,
    pub sregs: KvmSregs,
    pub mp_state: KvmMpState,
}

#[cfg(target_arch = "x86_64")]
#[derive(Debug)]
pub(super) struct RestoredCheckpoint {
    pub vcpus: Vec<VcpuSnapshot>,
}

#[cfg(target_arch = "x86_64")]
pub(super) fn snapshot_vcpu(vcpu: &VcpuFd) -> Result<VcpuSnapshot> {
    Ok(VcpuSnapshot {
        id: vcpu.id(),
        regs: vcpu.get_regs()?,
        sregs: vcpu.get_sregs()?,
        mp_state: vcpu.get_mp_state()?,
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
        vcpu.set_sregs(&snapshot.sregs)?;
        vcpu.set_regs(&snapshot.regs)?;
        vcpu.set_mp_state(snapshot.mp_state)?;
    }
    Ok(())
}

#[cfg(target_arch = "x86_64")]
pub(super) fn write_checkpoint(
    path: &Path,
    memory: &GuestMemory,
    vcpus: &[VcpuSnapshot],
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
    let write_result = write_checkpoint_inner(&tmp_path, memory, vcpus);
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
) -> Result<RestoredCheckpoint> {
    let file = std::fs::File::open(path)
        .with_context(|| format!("open KVM checkpoint: {}", path.display()))?;
    let mut reader = BufReader::new(file);
    let mut header_bytes = [0u8; HEADER_LEN as usize];
    reader
        .read_exact(&mut header_bytes)
        .context("read checkpoint header")?;
    let header = CheckpointHeader::decode(&header_bytes)?;
    validate_header(&header, memory.size(), expected_vcpu_count)?;

    let mut vcpus = Vec::with_capacity(header.vcpu_count as usize);
    for id in 0..header.vcpu_count {
        vcpus.push(read_vcpu_snapshot(&mut reader, id)?);
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

    Ok(RestoredCheckpoint { vcpus })
}

#[cfg(target_arch = "x86_64")]
fn write_checkpoint_inner(path: &Path, memory: &GuestMemory, vcpus: &[VcpuSnapshot]) -> Result<()> {
    let file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .with_context(|| format!("create checkpoint temp file: {}", path.display()))?;
    let mut writer = BufWriter::new(file);

    let header = CheckpointHeader::current(memory.size(), vcpus.len() as u32);
    writer
        .write_all(&header.encode())
        .context("write checkpoint header")?;
    for snapshot in vcpus {
        write_vcpu_snapshot(&mut writer, snapshot)?;
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
fn validate_header(header: &CheckpointHeader, ram_bytes: u64, vcpu_count: u32) -> Result<()> {
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
    })
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
        let header = CheckpointHeader::current(4096, 2);
        let decoded = CheckpointHeader::decode(&header.encode()).unwrap();
        assert_eq!(decoded, header);
        assert_eq!(decoded.version, VERSION);
        assert_eq!(decoded.ram_bytes, 4096);
        assert_eq!(decoded.vcpu_count, 2);
        assert_eq!(decoded.vcpu_state_len, X86_VCPU_STATE_LEN);
    }

    #[test]
    fn header_rejects_bad_magic() {
        let mut encoded = CheckpointHeader::current(4096, 1).encode();
        encoded[0] = b'X';
        let err = CheckpointHeader::decode(&encoded).unwrap_err();
        assert!(err.to_string().contains("bad checkpoint magic"));
    }

    fn snapshot(id: u32) -> VcpuSnapshot {
        let mut regs = KvmRegs::default();
        regs.rax = id as u64 + 10;
        regs.rip = 0x1000 + id as u64;
        let mut sregs = KvmSregs::default();
        sregs.cr3 = 0x2000 + id as u64;
        let mp_state = KvmMpState {
            mp_state: KVM_MP_STATE_RUNNABLE,
        };
        VcpuSnapshot {
            id,
            regs,
            sregs,
            mp_state,
        }
    }

    #[test]
    fn writes_header_and_memory() {
        let dir = temp_dir("writes-header-memory");
        let path = dir.join("state.kvm");
        let mem = GuestMemory::new(8192).unwrap();
        mem.write_at(0, b"hello").unwrap();
        mem.write_at(4096, b"world").unwrap();

        write_checkpoint(&path, &mem, &[snapshot(0), snapshot(1)]).unwrap();

        let bytes = std::fs::read(path).unwrap();
        let header = CheckpointHeader::decode(&bytes[..HEADER_LEN as usize]).unwrap();
        assert_eq!(header.ram_bytes, 8192);
        let memory_offset =
            HEADER_LEN as usize + (4 + X86_VCPU_STATE_LEN as usize) * header.vcpu_count as usize;
        assert_eq!(&bytes[memory_offset..memory_offset + 5], b"hello");
        assert_eq!(&bytes[memory_offset + 4096..memory_offset + 4101], b"world");
        assert_eq!(bytes.len(), memory_offset + 8192);
    }

    #[test]
    fn restores_memory_and_vcpu_state() {
        let dir = temp_dir("restore-memory-vcpu");
        let path = dir.join("state.kvm");
        let mem = GuestMemory::new(8192).unwrap();
        mem.write_at(0, b"hello").unwrap();
        mem.write_at(4096, b"world").unwrap();
        write_checkpoint(&path, &mem, &[snapshot(0), snapshot(1)]).unwrap();

        let restored_mem = GuestMemory::new(8192).unwrap();
        let restored = read_checkpoint(&path, &restored_mem, 2).unwrap();

        let mut buf = [0u8; 5];
        restored_mem.read_at(0, &mut buf).unwrap();
        assert_eq!(&buf, b"hello");
        restored_mem.read_at(4096, &mut buf).unwrap();
        assert_eq!(&buf, b"world");
        assert_eq!(restored.vcpus.len(), 2);
        assert_eq!(restored.vcpus[1].regs.rip, 0x1001);
        assert_eq!(restored.vcpus[1].sregs.cr3, 0x2001);
        assert_eq!(restored.vcpus[1].mp_state.mp_state, KVM_MP_STATE_RUNNABLE);
    }

    #[test]
    fn overwrites_atomically() {
        let dir = temp_dir("atomic-overwrite");
        let path = dir.join("state.kvm");
        std::fs::write(&path, b"old").unwrap();
        let mem = GuestMemory::new(4096).unwrap();

        write_checkpoint(&path, &mem, &[snapshot(0)]).unwrap();

        let bytes = std::fs::read(path).unwrap();
        assert_ne!(&bytes, b"old");
        assert_eq!(
            bytes.len(),
            HEADER_LEN as usize + 4 + X86_VCPU_STATE_LEN as usize + 4096
        );
        assert!(std::fs::read_dir(&dir).unwrap().all(|e| !e
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains(".tmp.")));
    }

    #[test]
    fn rejects_missing_parent() {
        let dir = temp_dir("missing-parent");
        let path = dir.join("missing").join("state.kvm");
        let mem = GuestMemory::new(4096).unwrap();

        let err = write_checkpoint(&path, &mem, &[snapshot(0)]).unwrap_err();

        assert!(err
            .to_string()
            .contains("checkpoint parent directory does not exist"));
    }

    #[test]
    fn removes_temp_file_after_create_failure() {
        let dir = temp_dir("temp-cleanup");
        let path = dir.join("state.kvm");
        let tmp = temp_path_for(&path);
        std::fs::write(&tmp, b"conflict").unwrap();
        let mem = GuestMemory::new(4096).unwrap();

        let err = write_checkpoint(&path, &mem, &[snapshot(0)]).unwrap_err();

        assert!(err.to_string().contains("create checkpoint temp file"));
        assert!(!tmp.exists());
        assert!(!path.exists());
    }

    #[test]
    fn restore_rejects_wrong_ram_size() {
        let dir = temp_dir("wrong-ram-size");
        let path = dir.join("state.kvm");
        let mem = GuestMemory::new(4096).unwrap();
        write_checkpoint(&path, &mem, &[snapshot(0)]).unwrap();
        let larger_mem = GuestMemory::new(8192).unwrap();

        let err = read_checkpoint(&path, &larger_mem, 1).unwrap_err();

        assert!(err.to_string().contains("checkpoint RAM size mismatch"));
    }

    #[test]
    fn restore_rejects_wrong_vcpu_count() {
        let dir = temp_dir("wrong-vcpu-count");
        let path = dir.join("state.kvm");
        let mem = GuestMemory::new(4096).unwrap();
        write_checkpoint(&path, &mem, &[snapshot(0)]).unwrap();

        let err = read_checkpoint(&path, &mem, 2).unwrap_err();

        assert!(err.to_string().contains("checkpoint vCPU count mismatch"));
    }

    #[test]
    fn restore_rejects_trailing_bytes() {
        let dir = temp_dir("trailing-bytes");
        let path = dir.join("state.kvm");
        let mem = GuestMemory::new(4096).unwrap();
        write_checkpoint(&path, &mem, &[snapshot(0)]).unwrap();
        std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap()
            .write_all(b"extra")
            .unwrap();

        let err = read_checkpoint(&path, &mem, 1).unwrap_err();

        assert!(err.to_string().contains("checkpoint has trailing bytes"));
    }
}
