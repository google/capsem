//! Bounded vhost-vsock checkpoint wire state.

use super::{
    GuestMemoryRef, QueueConfig, VHOST_VSOCK_BACKEND_QUEUES, VSOCK_CHECKPOINT_HEADER_BYTES, VSOCK_CHECKPOINT_VERSION,
    VSOCK_CHECKPOINT_WITH_BASES_BYTES,
};
use anyhow::{bail, ensure, Context, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct VsockCheckpointState {
    pub(super) guest_cid: u64,
    pub(super) vring_bases: Option<[u32; VHOST_VSOCK_BACKEND_QUEUES]>,
}

pub(super) fn encode_vsock_checkpoint(
    guest_cid: u64,
    vring_bases: Option<[u32; VHOST_VSOCK_BACKEND_QUEUES]>,
) -> Vec<u8> {
    let capacity = if vring_bases.is_some() {
        VSOCK_CHECKPOINT_WITH_BASES_BYTES
    } else {
        VSOCK_CHECKPOINT_HEADER_BYTES
    };
    let mut encoded = Vec::with_capacity(capacity);
    encoded.extend_from_slice(&VSOCK_CHECKPOINT_VERSION.to_le_bytes());
    encoded.extend_from_slice(&guest_cid.to_le_bytes());
    encoded.push(u8::from(vring_bases.is_some()));
    if let Some(vring_bases) = vring_bases {
        for base in vring_bases {
            encoded.extend_from_slice(&base.to_le_bytes());
        }
    }
    encoded
}

pub(super) fn decode_vsock_checkpoint(encoded: &[u8]) -> Result<VsockCheckpointState> {
    ensure!(
        matches!(
            encoded.len(),
            VSOCK_CHECKPOINT_HEADER_BYTES | VSOCK_CHECKPOINT_WITH_BASES_BYTES
        ),
        "invalid vhost-vsock checkpoint length: {}",
        encoded.len()
    );
    let version = u32::from_le_bytes(encoded[0..4].try_into().unwrap());
    ensure!(
        version == VSOCK_CHECKPOINT_VERSION,
        "unsupported vhost-vsock checkpoint version: {version}"
    );
    let cid_offset = std::mem::size_of::<u32>();
    let guest_cid = u64::from_le_bytes(
        encoded[cid_offset..cid_offset + std::mem::size_of::<u64>()]
            .try_into()
            .unwrap(),
    );
    let presence_offset = cid_offset + std::mem::size_of::<u64>();
    let vring_bases = match encoded[presence_offset] {
        0 => {
            ensure!(
                encoded.len() == VSOCK_CHECKPOINT_HEADER_BYTES,
                "invalid vhost-vsock checkpoint length for bases-present flag 0: {}",
                encoded.len()
            );
            None
        }
        1 => {
            ensure!(
                encoded.len() == VSOCK_CHECKPOINT_WITH_BASES_BYTES,
                "invalid vhost-vsock checkpoint length for bases-present flag 1: {}",
                encoded.len()
            );
            let mut vring_bases = [0u32; VHOST_VSOCK_BACKEND_QUEUES];
            for (index, base) in vring_bases.iter_mut().enumerate() {
                let offset = VSOCK_CHECKPOINT_HEADER_BYTES + index * std::mem::size_of::<u32>();
                *base = u32::from_le_bytes(encoded[offset..offset + 4].try_into().unwrap());
                ensure!(
                    u16::try_from(*base).is_ok(),
                    "invalid vhost-vsock queue {index} vring base: {base}"
                );
            }
            Some(vring_bases)
        }
        flag => bail!("invalid vhost-vsock bases-present flag: {flag}"),
    };
    Ok(VsockCheckpointState { guest_cid, vring_bases })
}

pub(super) fn queue_used_idx(mem: &GuestMemoryRef, queue: &QueueConfig) -> Result<u32> {
    let address = queue
        .device_addr
        .checked_add(2)
        .context("vhost-vsock used index address overflow")?;
    let ptr = mem
        .gpa_range_to_host(address, 2)
        .context("vhost-vsock used ring idx GPA out of range")?;
    let idx = unsafe { u16::from_le(std::ptr::read_unaligned(ptr as *const u16)) };
    Ok(u32::from(idx))
}

pub(super) fn queue_avail_idx(mem: &GuestMemoryRef, queue: &QueueConfig) -> Result<u32> {
    let address = queue
        .driver_addr
        .checked_add(2)
        .context("vhost-vsock available index address overflow")?;
    let ptr = mem
        .gpa_range_to_host(address, 2)
        .context("vhost-vsock avail ring idx GPA out of range")?;
    let idx = unsafe { u16::from_le(std::ptr::read_unaligned(ptr as *const u16)) };
    Ok(u32::from(idx))
}
