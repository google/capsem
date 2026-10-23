use super::*;

impl VirtioBlockDevice {
    /// Process a discard request by punching holes in the backing file.
    pub(super) fn process_discard(
        file: &mut std::fs::File,
        mem: &GuestMemoryRef,
        read_only: bool,
        capacity_sectors: u64,
        data_descs: &[(u64, u32)],
    ) -> u8 {
        if read_only {
            return VIRTIO_BLK_S_IOERR;
        }

        let data = match Self::read_guest_data(mem, data_descs) {
            Some(data) => data,
            None => return VIRTIO_BLK_S_IOERR,
        };
        if data.len() % DISCARD_SEGMENT_SIZE != 0 {
            return VIRTIO_BLK_S_IOERR;
        }

        for segment in data.chunks_exact(DISCARD_SEGMENT_SIZE) {
            let sector = u64::from_le_bytes(segment[0..8].try_into().unwrap());
            let num_sectors = u64::from(u32::from_le_bytes(segment[8..12].try_into().unwrap()));
            if num_sectors == 0 {
                continue;
            }

            let offset = match sector.checked_mul(SECTOR_SIZE) {
                Some(offset) => offset,
                None => return VIRTIO_BLK_S_IOERR,
            };
            let len = match num_sectors.checked_mul(SECTOR_SIZE) {
                Some(len) => len,
                None => return VIRTIO_BLK_S_IOERR,
            };
            if offset
                .checked_add(len)
                .is_none_or(|end| end > capacity_sectors * SECTOR_SIZE)
            {
                return VIRTIO_BLK_S_IOERR;
            }

            if Self::discard_range(file, offset, len).is_err() {
                return VIRTIO_BLK_S_IOERR;
            }
        }

        VIRTIO_BLK_S_OK
    }

    pub(super) fn read_guest_data(mem: &GuestMemoryRef, data_descs: &[(u64, u32)]) -> Option<Vec<u8>> {
        let total_len = data_descs.iter().try_fold(0_usize, |total, &(_, len)| {
            total
                .checked_add(len as usize)
                .filter(|&size| size <= MAX_DISCARD_BYTES)
        })?;
        let mut data = Vec::with_capacity(total_len);
        for &(gpa, len) in data_descs {
            if len == 0 {
                continue;
            }
            let host_ptr = mem.gpa_range_to_host(gpa, u64::from(len))?;
            let buf = unsafe { std::slice::from_raw_parts(host_ptr, len as usize) };
            data.extend_from_slice(buf);
        }
        Some(data)
    }
}
