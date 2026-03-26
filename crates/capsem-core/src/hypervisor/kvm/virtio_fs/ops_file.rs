//! File I/O FUSE operations: OPEN, READ, WRITE, CREATE, RELEASE, FLUSH, FSYNC, LSEEK.

use std::io::{Read, Seek, SeekFrom, Write};
use std::os::unix::fs::PermissionsExt;

use crate::hypervisor::fuse::{self, *};
use super::FuseProcessor;

impl FuseProcessor {
    pub(super) fn do_open(&mut self, header: &FuseInHeader, body: &[u8]) -> Vec<u8> {
        let open_in: FuseOpenIn = match fuse::read_struct(body) {
            Some(s) => s,
            None => return fuse::error_response(header.unique, -libc::EIO),
        };
        let path = match self.inodes.get(header.nodeid) {
            Some(p) => p.clone(),
            None => return fuse::error_response(header.unique, -libc::ENOENT),
        };

        let flags = open_in.flags as i32;
        let accmode = flags & libc::O_ACCMODE;
        if self.read_only && accmode != libc::O_RDONLY {
            return fuse::error_response(header.unique, -libc::EROFS);
        }

        let file = match std::fs::OpenOptions::new()
            .read(accmode == libc::O_RDONLY || accmode == libc::O_RDWR)
            .write(accmode == libc::O_WRONLY || accmode == libc::O_RDWR)
            .append(flags & libc::O_APPEND != 0)
            .truncate(flags & libc::O_TRUNC != 0)
            .open(&path)
        {
            Ok(f) => f,
            Err(e) => return fuse::error_response(header.unique, -fuse::io_error_to_errno(&e)),
        };

        let fh = match self.file_handles.alloc(OpenHandle::File(file)) {
            Some(fh) => fh,
            None => return fuse::error_response(header.unique, -libc::EMFILE),
        };
        let open_out = FuseOpenOut { fh, open_flags: 0, padding: 0 };
        fuse::success_response(header.unique, fuse::as_bytes(&open_out))
    }

    pub(super) fn do_release(&mut self, header: &FuseInHeader, body: &[u8]) -> Vec<u8> {
        if let Some(r) = fuse::read_struct::<FuseReleaseIn>(body) {
            self.file_handles.remove(r.fh);
        }
        fuse::success_response(header.unique, &[])
    }

    pub(super) fn do_read(&mut self, header: &FuseInHeader, body: &[u8]) -> Vec<u8> {
        let read_in: FuseReadIn = match fuse::read_struct(body) {
            Some(s) => s,
            None => return fuse::error_response(header.unique, -libc::EIO),
        };
        let file = match self.file_handles.get_file(read_in.fh) {
            Some(f) => f,
            None => return fuse::error_response(header.unique, -libc::EBADF),
        };

        if file.seek(SeekFrom::Start(read_in.offset)).is_err() {
            return fuse::error_response(header.unique, -libc::EIO);
        }

        let clamped = read_in.size.min(super::MAX_READ_SIZE);
        let mut data = vec![0u8; clamped as usize];
        let n = match file.read(&mut data) {
            Ok(n) => n,
            Err(e) => return fuse::error_response(header.unique, -fuse::io_error_to_errno(&e)),
        };
        data.truncate(n);
        fuse::success_response(header.unique, &data)
    }

    pub(super) fn do_write(&mut self, header: &FuseInHeader, body: &[u8]) -> Vec<u8> {
        if self.read_only { return fuse::error_response(header.unique, -libc::EROFS); }
        let write_in: FuseWriteIn = match fuse::read_struct(body) {
            Some(s) => s,
            None => return fuse::error_response(header.unique, -libc::EIO),
        };
        let write_data = &body[std::mem::size_of::<FuseWriteIn>()..];
        let to_write = (write_in.size as usize).min(write_data.len());

        let file = match self.file_handles.get_file(write_in.fh) {
            Some(f) => f,
            None => return fuse::error_response(header.unique, -libc::EBADF),
        };
        if file.seek(SeekFrom::Start(write_in.offset)).is_err() {
            return fuse::error_response(header.unique, -libc::EIO);
        }
        if let Err(e) = file.write_all(&write_data[..to_write]) {
            return fuse::error_response(header.unique, -fuse::io_error_to_errno(&e));
        }

        let write_out = FuseWriteOut { size: to_write as u32, padding: 0 };
        fuse::success_response(header.unique, fuse::as_bytes(&write_out))
    }

    pub(super) fn do_create(&mut self, header: &FuseInHeader, body: &[u8]) -> Vec<u8> {
        if self.read_only { return fuse::error_response(header.unique, -libc::EROFS); }
        let create_in: FuseCreateIn = match fuse::read_struct(body) {
            Some(s) => s,
            None => return fuse::error_response(header.unique, -libc::EIO),
        };
        let name_buf = &body[std::mem::size_of::<FuseCreateIn>()..];
        let name = match fuse::extract_name(name_buf) {
            Some(n) => n,
            None => return fuse::error_response(header.unique, -libc::EINVAL),
        };

        let ino = match self.inodes.lookup(header.nodeid, name) {
            Some(i) => i,
            None => {
                // File doesn't exist yet -- create it
                let name_str = match std::str::from_utf8(name) {
                    Ok(s) => s,
                    Err(_) => return fuse::error_response(header.unique, -libc::EINVAL),
                };
                let parent_path = match self.inodes.get(header.nodeid) {
                    Some(p) => p.clone(),
                    None => return fuse::error_response(header.unique, -libc::ENOENT),
                };
                let child_path = parent_path.join(name_str);

                let flags = create_in.flags as i32;
                let accmode = flags & libc::O_ACCMODE;
                let file = match std::fs::OpenOptions::new()
                    .read(accmode == libc::O_RDONLY || accmode == libc::O_RDWR)
                    .write(true).create_new(true).open(&child_path)
                {
                    Ok(f) => f,
                    Err(e) => return fuse::error_response(header.unique, -fuse::io_error_to_errno(&e)),
                };
                let mode = create_in.mode & !create_in.umask;
                let _ = std::fs::set_permissions(&child_path, std::fs::Permissions::from_mode(mode));

                let ino = match self.inodes.lookup(header.nodeid, name) {
                    Some(i) => i,
                    None => return fuse::error_response(header.unique, -libc::EIO),
                };
                let fh = match self.file_handles.alloc(OpenHandle::File(file)) {
                    Some(fh) => fh,
                    None => return fuse::error_response(header.unique, -libc::EMFILE),
                };
                let meta = match std::fs::symlink_metadata(&child_path) {
                    Ok(m) => m,
                    Err(e) => return fuse::error_response(header.unique, -fuse::io_error_to_errno(&e)),
                };
                return self.entry_and_open_response(header.unique, ino, &meta, fh);
            }
        };

        // File already exists -- open it
        let path = match self.inodes.get(ino) {
            Some(p) => p.clone(),
            None => return fuse::error_response(header.unique, -libc::EIO),
        };
        let flags = create_in.flags as i32;
        let accmode = flags & libc::O_ACCMODE;
        let file = match std::fs::OpenOptions::new()
            .read(accmode == libc::O_RDONLY || accmode == libc::O_RDWR)
            .write(true).truncate(flags & libc::O_TRUNC != 0).open(&path)
        {
            Ok(f) => f,
            Err(e) => return fuse::error_response(header.unique, -fuse::io_error_to_errno(&e)),
        };
        let fh = match self.file_handles.alloc(OpenHandle::File(file)) {
            Some(fh) => fh,
            None => return fuse::error_response(header.unique, -libc::EMFILE),
        };
        let meta = match std::fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(e) => return fuse::error_response(header.unique, -fuse::io_error_to_errno(&e)),
        };
        self.entry_and_open_response(header.unique, ino, &meta, fh)
    }

    pub(super) fn entry_and_open_response(&self, unique: u64, ino: u64, meta: &std::fs::Metadata, fh: u64) -> Vec<u8> {
        let entry = FuseEntryOut {
            nodeid: ino, generation: 0, entry_valid: 1, attr_valid: 1,
            entry_valid_nsec: 0, attr_valid_nsec: 0,
            attr: fuse::metadata_to_fuse_attr(ino, meta),
        };
        let open_out = FuseOpenOut { fh, open_flags: 0, padding: 0 };
        let mut body = fuse::as_bytes(&entry).to_vec();
        body.extend_from_slice(fuse::as_bytes(&open_out));
        fuse::success_response(unique, &body)
    }

    pub(super) fn do_flush(&mut self, header: &FuseInHeader, body: &[u8]) -> Vec<u8> {
        let f: FuseFlushIn = match fuse::read_struct(body) {
            Some(s) => s,
            None => return fuse::error_response(header.unique, -libc::EIO),
        };
        let file = match self.file_handles.get_file(f.fh) {
            Some(f) => f,
            None => return fuse::error_response(header.unique, -libc::EBADF),
        };
        match file.flush() {
            Ok(()) => fuse::success_response(header.unique, &[]),
            Err(e) => fuse::error_response(header.unique, -fuse::io_error_to_errno(&e)),
        }
    }

    pub(super) fn do_fsync(&mut self, header: &FuseInHeader, body: &[u8]) -> Vec<u8> {
        let f: FuseFsyncIn = match fuse::read_struct(body) {
            Some(s) => s,
            None => return fuse::error_response(header.unique, -libc::EIO),
        };
        let file = match self.file_handles.get_file(f.fh) {
            Some(f) => f,
            None => return fuse::error_response(header.unique, -libc::EBADF),
        };
        let result = if f.fsync_flags & 1 != 0 {
            file.sync_data()
        } else {
            file.sync_all()
        };
        match result {
            Ok(()) => fuse::success_response(header.unique, &[]),
            Err(e) => fuse::error_response(header.unique, -fuse::io_error_to_errno(&e)),
        }
    }

    pub(super) fn do_fsyncdir(&mut self, header: &FuseInHeader, body: &[u8]) -> Vec<u8> {
        // Directory handles are in-memory Vec<DirEntryData>, no fd to sync.
        // Validate the request body and handle for correctness, but sync is a no-op.
        let f: FuseFsyncIn = match fuse::read_struct(body) {
            Some(s) => s,
            None => return fuse::error_response(header.unique, -libc::EIO),
        };
        if self.file_handles.get_dir(f.fh).is_none() {
            return fuse::error_response(header.unique, -libc::EBADF);
        }
        fuse::success_response(header.unique, &[])
    }

    pub(super) fn do_lseek(&mut self, header: &FuseInHeader, body: &[u8]) -> Vec<u8> {
        let ls: FuseLseekIn = match fuse::read_struct(body) {
            Some(s) => s,
            None => return fuse::error_response(header.unique, -libc::EIO),
        };
        let file = match self.file_handles.get_file(ls.fh) {
            Some(f) => f,
            None => return fuse::error_response(header.unique, -libc::EBADF),
        };
        let whence = match ls.whence {
            0 => SeekFrom::Start(ls.offset),
            1 => SeekFrom::Current(ls.offset as i64),
            2 => SeekFrom::End(ls.offset as i64),
            _ => return fuse::error_response(header.unique, -libc::EINVAL),
        };
        match file.seek(whence) {
            Ok(offset) => fuse::success_response(header.unique, fuse::as_bytes(&FuseLseekOut { offset })),
            Err(e) => fuse::error_response(header.unique, -fuse::io_error_to_errno(&e)),
        }
    }
}
