use super::*;

#[test]
fn fuse_in_header_size() {
    assert_eq!(std::mem::size_of::<FuseInHeader>(), 40);
}
#[test]
fn fuse_out_header_size() {
    assert_eq!(std::mem::size_of::<FuseOutHeader>(), 16);
}
#[test]
fn fuse_attr_size() {
    assert_eq!(std::mem::size_of::<FuseAttr>(), 88);
}
#[test]
fn fuse_entry_out_size() {
    assert_eq!(std::mem::size_of::<FuseEntryOut>(), 128);
}
#[test]
fn fuse_attr_out_size() {
    assert_eq!(std::mem::size_of::<FuseAttrOut>(), 104);
}
#[test]
fn fuse_init_in_size() {
    assert_eq!(std::mem::size_of::<FuseInitIn>(), 16);
}
#[test]
fn fuse_init_out_size() {
    assert_eq!(std::mem::size_of::<FuseInitOut>(), 64);
}
#[test]
fn fuse_open_in_size() {
    assert_eq!(std::mem::size_of::<FuseOpenIn>(), 8);
}
#[test]
fn fuse_open_out_size() {
    assert_eq!(std::mem::size_of::<FuseOpenOut>(), 16);
}
#[test]
fn fuse_read_in_size() {
    assert_eq!(std::mem::size_of::<FuseReadIn>(), 40);
}
#[test]
fn fuse_write_in_size() {
    assert_eq!(std::mem::size_of::<FuseWriteIn>(), 40);
}
#[test]
fn fuse_write_out_size() {
    assert_eq!(std::mem::size_of::<FuseWriteOut>(), 8);
}
#[test]
fn fuse_create_in_size() {
    assert_eq!(std::mem::size_of::<FuseCreateIn>(), 16);
}
#[test]
fn fuse_mkdir_in_size() {
    assert_eq!(std::mem::size_of::<FuseMkdirIn>(), 8);
}
#[test]
fn fuse_mknod_in_size() {
    assert_eq!(std::mem::size_of::<FuseMknodIn>(), 16);
}
#[test]
fn fuse_setattr_in_size() {
    assert_eq!(std::mem::size_of::<FuseSetAttrIn>(), 88);
}
#[test]
fn fuse_rename_in_size() {
    assert_eq!(std::mem::size_of::<FuseRenameIn>(), 8);
}
#[test]
fn fuse_rename2_in_size() {
    assert_eq!(std::mem::size_of::<FuseRename2In>(), 16);
}
#[test]
fn fuse_link_in_size() {
    assert_eq!(std::mem::size_of::<FuseLinkIn>(), 8);
}
#[test]
fn fuse_forget_in_size() {
    assert_eq!(std::mem::size_of::<FuseForgetIn>(), 8);
}
#[test]
fn fuse_batch_forget_in_size() {
    assert_eq!(std::mem::size_of::<FuseBatchForgetIn>(), 8);
}
#[test]
fn fuse_forget_one_size() {
    assert_eq!(std::mem::size_of::<FuseForgetOne>(), 16);
}
#[test]
fn fuse_release_in_size() {
    assert_eq!(std::mem::size_of::<FuseReleaseIn>(), 24);
}
#[test]
fn fuse_fsync_in_size() {
    assert_eq!(std::mem::size_of::<FuseFsyncIn>(), 16);
}
#[test]
fn fuse_flush_in_size() {
    assert_eq!(std::mem::size_of::<FuseFlushIn>(), 24);
}
#[test]
fn fuse_kstatfs_size() {
    assert_eq!(std::mem::size_of::<FuseKStatfs>(), 80);
}
#[test]
fn fuse_lseek_in_size() {
    assert_eq!(std::mem::size_of::<FuseLseekIn>(), 24);
}
#[test]
fn fuse_lseek_out_size() {
    assert_eq!(std::mem::size_of::<FuseLseekOut>(), 8);
}
#[test]
fn fuse_dirent_size() {
    assert_eq!(std::mem::size_of::<FuseDirent>(), 24);
}
