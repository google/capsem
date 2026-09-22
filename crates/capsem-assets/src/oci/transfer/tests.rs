use super::*;
use sha2::{Digest, Sha256};

fn layout(files: &[(&str, &[u8])]) -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    for (path, bytes) in files {
        let file = root.path().join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, bytes).unwrap();
    }
    root
}

#[test]
fn transfer_manifest_keys_each_verified_file_as_one_part_with_its_digest() {
    let root = layout(&[
        ("index.json", b"{}"),
        ("blobs/sha256/aa", b"layer bytes"),
        ("oci-layout", b""),
    ]);
    let files = vec![
        PathBuf::from("index.json"),
        PathBuf::from("blobs/sha256/aa"),
        PathBuf::from("oci-layout"),
    ];
    let manifest = transfer_manifest(root.path(), &files).unwrap();
    assert_eq!(manifest.len(), 3);
    assert_eq!(
        (manifest[1].path.as_str(), manifest[1].key, manifest[1].parts),
        ("blobs/sha256/aa", 1, 1)
    );
    assert_eq!(manifest[1].sha256, format!("{:x}", Sha256::digest(b"layer bytes")));
    assert_eq!(manifest[2].parts, 0, "an empty file has nothing to upload");
    assert_eq!(manifest[2].sha256, format!("{:x}", Sha256::digest(b"")));
}

#[test]
fn transfer_manifest_refuses_paths_that_escape_the_layout() {
    let root = layout(&[("index.json", b"{}")]);
    for bad in ["/etc/passwd", "../index.json", "blobs/../../x"] {
        assert!(
            transfer_manifest(root.path(), &[PathBuf::from(bad)]).is_err(),
            "accepted {bad}"
        );
    }
}
