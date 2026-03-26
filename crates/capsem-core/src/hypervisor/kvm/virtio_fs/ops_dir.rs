//! Directory and namespace FUSE operations: OPENDIR, READDIR, RELEASEDIR,
//! MKDIR, RMDIR, UNLINK, RENAME, MKNOD, SYMLINK, READLINK, LINK.

use std::os::unix::fs::{MetadataExt, PermissionsExt};

use crate::hypervisor::fuse::{self, *};
use super::FuseProcessor;

impl FuseProcessor {
    pub(super) fn do_opendir(&mut self, header: &FuseInHeader) -> Vec<u8> {
        let path = match self.inodes.get(header.nodeid) {
            Some(p) => p.clone(),
            None => return fuse::error_response(header.unique, -libc::ENOENT),
        };
        let read_dir = match std::fs::read_dir(&path) {
            Ok(rd) => rd,
            Err(e) => return fuse::error_response(header.unique, -fuse::io_error_to_errno(&e)),
        };

        let mut entries = vec![
            DirEntryData { name: b".".to_vec(), ino: header.nodeid, type_: DT_DIR },
            DirEntryData { name: b"..".to_vec(), ino: header.nodeid, type_: DT_DIR },
        ];
        for entry in read_dir.flatten() {
            let name = entry.file_name().into_encoded_bytes().to_vec();
            let meta = entry.metadata().ok();
            let ino = meta.as_ref().map_or(0, |m| m.ino());
            let type_ = meta.as_ref().map_or(DT_UNKNOWN, |m| fuse::mode_to_dtype(m.mode()));
            entries.push(DirEntryData { name, ino, type_ });
        }

        let fh = match self.file_handles.alloc(OpenHandle::Dir(entries)) {
            Some(fh) => fh,
            None => return fuse::error_response(header.unique, -libc::EMFILE),
        };
        fuse::success_response(header.unique, fuse::as_bytes(&FuseOpenOut { fh, open_flags: 0, padding: 0 }))
    }

    pub(super) fn do_readdir(&self, header: &FuseInHeader, body: &[u8]) -> Vec<u8> {
        let read_in: FuseReadIn = match fuse::read_struct(body) {
            Some(s) => s,
            None => return fuse::error_response(header.unique, -libc::EIO),
        };
        let entries = match self.file_handles.get_dir(read_in.fh) {
            Some(e) => e,
            None => return fuse::error_response(header.unique, -libc::EBADF),
        };

        let max_size = read_in.size as usize;
        let start = read_in.offset as usize;
        let dirent_hdr = std::mem::size_of::<FuseDirent>();
        let mut buf = Vec::new();

        for (i, entry) in entries.iter().enumerate() {
            if i < start { continue; }
            let entry_size = fuse::dirent_align(dirent_hdr + entry.name.len());
            if buf.len() + entry_size > max_size { break; }

            let dirent = FuseDirent {
                ino: entry.ino, off: (i + 1) as u64,
                namelen: entry.name.len() as u32, type_: entry.type_,
            };
            buf.extend_from_slice(fuse::as_bytes(&dirent));
            buf.extend_from_slice(&entry.name);
            buf.extend(std::iter::repeat(0u8).take(entry_size - dirent_hdr - entry.name.len()));
        }

        fuse::success_response(header.unique, &buf)
    }

    pub(super) fn do_releasedir(&mut self, header: &FuseInHeader, body: &[u8]) -> Vec<u8> {
        if let Some(r) = fuse::read_struct::<FuseReleaseIn>(body) {
            self.file_handles.remove(r.fh);
        }
        fuse::success_response(header.unique, &[])
    }

    pub(super) fn do_mkdir(&mut self, header: &FuseInHeader, body: &[u8]) -> Vec<u8> {
        if self.read_only { return fuse::error_response(header.unique, -libc::EROFS); }
        let mkdir_in: FuseMkdirIn = match fuse::read_struct(body) {
            Some(s) => s,
            None => return fuse::error_response(header.unique, -libc::EIO),
        };
        let name = match fuse::extract_name(&body[std::mem::size_of::<FuseMkdirIn>()..]) {
            Some(n) => n,
            None => return fuse::error_response(header.unique, -libc::EINVAL),
        };
        let name_str = match std::str::from_utf8(name) {
            Ok(s) => s,
            Err(_) => return fuse::error_response(header.unique, -libc::EINVAL),
        };
        let parent = match self.inodes.get(header.nodeid) {
            Some(p) => p.clone(),
            None => return fuse::error_response(header.unique, -libc::ENOENT),
        };
        let child_path = parent.join(name_str);

        if let Err(e) = std::fs::create_dir(&child_path) {
            return fuse::error_response(header.unique, -fuse::io_error_to_errno(&e));
        }
        let _ = std::fs::set_permissions(&child_path,
            std::fs::Permissions::from_mode(mkdir_in.mode & !mkdir_in.umask));

        let ino = match self.inodes.lookup(header.nodeid, name) {
            Some(i) => i,
            None => return fuse::error_response(header.unique, -libc::EIO),
        };
        let meta = match std::fs::symlink_metadata(&child_path) {
            Ok(m) => m,
            Err(e) => return fuse::error_response(header.unique, -fuse::io_error_to_errno(&e)),
        };
        let entry = FuseEntryOut {
            nodeid: ino, generation: 0, entry_valid: 1, attr_valid: 1,
            entry_valid_nsec: 0, attr_valid_nsec: 0,
            attr: fuse::metadata_to_fuse_attr(ino, &meta),
        };
        fuse::success_response(header.unique, fuse::as_bytes(&entry))
    }

    pub(super) fn do_unlink(&mut self, header: &FuseInHeader, body: &[u8]) -> Vec<u8> {
        if self.read_only { return fuse::error_response(header.unique, -libc::EROFS); }
        let name_str = match fuse::extract_name(body).and_then(|n| std::str::from_utf8(n).ok()) {
            Some(s) => s,
            None => return fuse::error_response(header.unique, -libc::EINVAL),
        };
        let parent = match self.inodes.get(header.nodeid) {
            Some(p) => p.clone(),
            None => return fuse::error_response(header.unique, -libc::ENOENT),
        };
        match std::fs::remove_file(parent.join(name_str)) {
            Ok(()) => fuse::success_response(header.unique, &[]),
            Err(e) => fuse::error_response(header.unique, -fuse::io_error_to_errno(&e)),
        }
    }

    pub(super) fn do_rmdir(&mut self, header: &FuseInHeader, body: &[u8]) -> Vec<u8> {
        if self.read_only { return fuse::error_response(header.unique, -libc::EROFS); }
        let name_str = match fuse::extract_name(body).and_then(|n| std::str::from_utf8(n).ok()) {
            Some(s) => s,
            None => return fuse::error_response(header.unique, -libc::EINVAL),
        };
        let parent = match self.inodes.get(header.nodeid) {
            Some(p) => p.clone(),
            None => return fuse::error_response(header.unique, -libc::ENOENT),
        };
        match std::fs::remove_dir(parent.join(name_str)) {
            Ok(()) => fuse::success_response(header.unique, &[]),
            Err(e) => fuse::error_response(header.unique, -fuse::io_error_to_errno(&e)),
        }
    }

    pub(super) fn do_rename(&mut self, header: &FuseInHeader, body: &[u8]) -> Vec<u8> {
        if self.read_only { return fuse::error_response(header.unique, -libc::EROFS); }
        let r: FuseRenameIn = match fuse::read_struct(body) {
            Some(s) => s,
            None => return fuse::error_response(header.unique, -libc::EIO),
        };
        self.rename_impl(header, r.newdir, &body[std::mem::size_of::<FuseRenameIn>()..])
    }

    pub(super) fn do_rename2(&mut self, header: &FuseInHeader, body: &[u8]) -> Vec<u8> {
        if self.read_only { return fuse::error_response(header.unique, -libc::EROFS); }
        let r: FuseRename2In = match fuse::read_struct(body) {
            Some(s) => s,
            None => return fuse::error_response(header.unique, -libc::EIO),
        };
        self.rename_impl(header, r.newdir, &body[std::mem::size_of::<FuseRename2In>()..])
    }

    fn rename_impl(&self, header: &FuseInHeader, newdir: u64, names_buf: &[u8]) -> Vec<u8> {
        let (old_name, new_name) = match fuse::extract_two_names(names_buf) {
            Some(n) => n, None => return fuse::error_response(header.unique, -libc::EINVAL),
        };
        let old_str = match std::str::from_utf8(old_name) {
            Ok(s) => s, Err(_) => return fuse::error_response(header.unique, -libc::EINVAL),
        };
        let new_str = match std::str::from_utf8(new_name) {
            Ok(s) => s, Err(_) => return fuse::error_response(header.unique, -libc::EINVAL),
        };
        let old_parent = match self.inodes.get(header.nodeid) {
            Some(p) => p.clone(), None => return fuse::error_response(header.unique, -libc::ENOENT),
        };
        let new_parent = match self.inodes.get(newdir) {
            Some(p) => p.clone(), None => return fuse::error_response(header.unique, -libc::ENOENT),
        };
        match std::fs::rename(old_parent.join(old_str), new_parent.join(new_str)) {
            Ok(()) => fuse::success_response(header.unique, &[]),
            Err(e) => fuse::error_response(header.unique, -fuse::io_error_to_errno(&e)),
        }
    }

    pub(super) fn do_mknod(&mut self, header: &FuseInHeader, body: &[u8]) -> Vec<u8> {
        if self.read_only { return fuse::error_response(header.unique, -libc::EROFS); }
        let mk: FuseMknodIn = match fuse::read_struct(body) {
            Some(s) => s,
            None => return fuse::error_response(header.unique, -libc::EIO),
        };
        let name = match fuse::extract_name(&body[std::mem::size_of::<FuseMknodIn>()..]) {
            Some(n) => n, None => return fuse::error_response(header.unique, -libc::EINVAL),
        };
        let name_str = match std::str::from_utf8(name) {
            Ok(s) => s, Err(_) => return fuse::error_response(header.unique, -libc::EINVAL),
        };
        let parent = match self.inodes.get(header.nodeid) {
            Some(p) => p.clone(), None => return fuse::error_response(header.unique, -libc::ENOENT),
        };
        let child_path = parent.join(name_str);
        let c_path = match std::ffi::CString::new(child_path.as_os_str().as_encoded_bytes()) {
            Ok(c) => c, Err(_) => return fuse::error_response(header.unique, -libc::EINVAL),
        };
        if unsafe { libc::mknod(c_path.as_ptr(), mk.mode & !mk.umask, mk.rdev as libc::dev_t) } != 0 {
            return fuse::error_response(header.unique, -fuse::errno());
        }
        let ino = match self.inodes.lookup(header.nodeid, name) {
            Some(i) => i, None => return fuse::error_response(header.unique, -libc::EIO),
        };
        let meta = match std::fs::symlink_metadata(&child_path) {
            Ok(m) => m, Err(e) => return fuse::error_response(header.unique, -fuse::io_error_to_errno(&e)),
        };
        let entry = FuseEntryOut {
            nodeid: ino, generation: 0, entry_valid: 1, attr_valid: 1,
            entry_valid_nsec: 0, attr_valid_nsec: 0,
            attr: fuse::metadata_to_fuse_attr(ino, &meta),
        };
        fuse::success_response(header.unique, fuse::as_bytes(&entry))
    }

    pub(super) fn do_symlink(&mut self, header: &FuseInHeader, body: &[u8]) -> Vec<u8> {
        if self.read_only { return fuse::error_response(header.unique, -libc::EROFS); }
        let (name, target) = match fuse::extract_two_names(body) {
            Some(n) => n, None => return fuse::error_response(header.unique, -libc::EINVAL),
        };
        let name_str = match std::str::from_utf8(name) {
            Ok(s) => s, Err(_) => return fuse::error_response(header.unique, -libc::EINVAL),
        };
        let target_str = match std::str::from_utf8(target) {
            Ok(s) => s, Err(_) => return fuse::error_response(header.unique, -libc::EINVAL),
        };
        let parent = match self.inodes.get(header.nodeid) {
            Some(p) => p.clone(), None => return fuse::error_response(header.unique, -libc::ENOENT),
        };
        let link_path = parent.join(name_str);
        if let Err(e) = std::os::unix::fs::symlink(target_str, &link_path) {
            return fuse::error_response(header.unique, -fuse::io_error_to_errno(&e));
        }
        let ino = match self.inodes.lookup(header.nodeid, name) {
            Some(i) => i, None => return fuse::error_response(header.unique, -libc::EIO),
        };
        let meta = match std::fs::symlink_metadata(&link_path) {
            Ok(m) => m, Err(e) => return fuse::error_response(header.unique, -fuse::io_error_to_errno(&e)),
        };
        let entry = FuseEntryOut {
            nodeid: ino, generation: 0, entry_valid: 1, attr_valid: 1,
            entry_valid_nsec: 0, attr_valid_nsec: 0,
            attr: fuse::metadata_to_fuse_attr(ino, &meta),
        };
        fuse::success_response(header.unique, fuse::as_bytes(&entry))
    }

    pub(super) fn do_readlink(&self, header: &FuseInHeader) -> Vec<u8> {
        let path = match self.inodes.get(header.nodeid) {
            Some(p) => p.clone(), None => return fuse::error_response(header.unique, -libc::ENOENT),
        };
        match std::fs::read_link(&path) {
            Ok(t) => fuse::success_response(header.unique, t.as_os_str().as_encoded_bytes()),
            Err(e) => fuse::error_response(header.unique, -fuse::io_error_to_errno(&e)),
        }
    }

    pub(super) fn do_link(&mut self, header: &FuseInHeader, body: &[u8]) -> Vec<u8> {
        if self.read_only { return fuse::error_response(header.unique, -libc::EROFS); }
        let link_in: FuseLinkIn = match fuse::read_struct(body) {
            Some(s) => s,
            None => return fuse::error_response(header.unique, -libc::EIO),
        };
        let name = match fuse::extract_name(&body[std::mem::size_of::<FuseLinkIn>()..]) {
            Some(n) => n, None => return fuse::error_response(header.unique, -libc::EINVAL),
        };
        let name_str = match std::str::from_utf8(name) {
            Ok(s) => s, Err(_) => return fuse::error_response(header.unique, -libc::EINVAL),
        };
        let old_path = match self.inodes.get(link_in.oldnodeid) {
            Some(p) => p.clone(), None => return fuse::error_response(header.unique, -libc::ENOENT),
        };
        let new_parent = match self.inodes.get(header.nodeid) {
            Some(p) => p.clone(), None => return fuse::error_response(header.unique, -libc::ENOENT),
        };
        let new_path = new_parent.join(name_str);
        if let Err(e) = std::fs::hard_link(&old_path, &new_path) {
            return fuse::error_response(header.unique, -fuse::io_error_to_errno(&e));
        }
        let ino = match self.inodes.lookup(header.nodeid, name) {
            Some(i) => i, None => return fuse::error_response(header.unique, -libc::EIO),
        };
        let meta = match std::fs::symlink_metadata(&new_path) {
            Ok(m) => m, Err(e) => return fuse::error_response(header.unique, -fuse::io_error_to_errno(&e)),
        };
        let entry = FuseEntryOut {
            nodeid: ino, generation: 0, entry_valid: 1, attr_valid: 1,
            entry_valid_nsec: 0, attr_valid_nsec: 0,
            attr: fuse::metadata_to_fuse_attr(ino, &meta),
        };
        fuse::success_response(header.unique, fuse::as_bytes(&entry))
    }
}
