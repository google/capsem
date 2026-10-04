//! Verify the vendored xpra-html5 client against its pinned checksums, then
//! embed it. A file that differs from `SHA256SUMS`, is missing, or is not
//! listed there fails the build: the gateway serves exactly the bytes the pin
//! in `vendor/xpra-html5/SOURCE.toml` produced, or nothing.

use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

const VENDOR: &str = "vendor/xpra-html5";
const REFRESH: &str = "regenerate it with build_system/scripts/build/vendor_xpra_html5.py";

fn files_under(root: &Path, prefix: &str, found: &mut BTreeSet<String>) {
    let entries = std::fs::read_dir(root).unwrap_or_else(|e| panic!("read {}: {e}", root.display()));
    for entry in entries {
        let entry = entry.expect("read vendored client entry");
        let name = entry
            .file_name()
            .into_string()
            .expect("vendored client paths are UTF-8");
        let relative = format!("{prefix}{name}");
        let kind = entry.file_type().expect("vendored client entry type");
        assert!(!kind.is_symlink(), "vendored client holds a symlink: {relative}");
        if kind.is_dir() {
            files_under(&entry.path(), &format!("{relative}/"), found);
        } else {
            found.insert(relative);
        }
    }
}

fn main() {
    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let vendor = manifest_dir.join(VENDOR);
    println!("cargo:rerun-if-changed={VENDOR}");

    let sums = std::fs::read_to_string(vendor.join("SHA256SUMS")).expect("read vendor/xpra-html5/SHA256SUMS");
    let mut listed = BTreeSet::new();
    let mut table = String::from("&[\n");
    for line in sums.lines() {
        let (expected, path) = line
            .split_once("  ")
            .unwrap_or_else(|| panic!("SHA256SUMS line is not `<sha256>  <path>`: {line:?}"));
        // The path is pasted into include_bytes!; keep it to a plain charset.
        assert!(
            path.split('/')
                .all(|part| !part.is_empty() && part != "." && part != "..")
                && path
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'_' | b'/')),
            "unsafe vendored path {path:?}"
        );
        let bytes = std::fs::read(vendor.join("www").join(path))
            .unwrap_or_else(|e| panic!("vendored {path} is missing ({e}); {REFRESH}"));
        let actual = format!("{:x}", Sha256::digest(&bytes));
        assert_eq!(actual, expected, "vendored {path} does not match SHA256SUMS; {REFRESH}");
        assert!(listed.insert(path.to_owned()), "{path} is listed twice in SHA256SUMS");
        writeln!(
            table,
            "    ({path:?}, include_bytes!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/{VENDOR}/www/{path}\"))),"
        )
        .expect("write to a String");
    }
    table.push(']');

    let mut present = BTreeSet::new();
    files_under(&vendor.join("www"), "", &mut present);
    let unlisted: Vec<_> = present.difference(&listed).collect();
    assert!(
        unlisted.is_empty(),
        "vendored files not in SHA256SUMS: {unlisted:?}; {REFRESH}"
    );

    let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR")).join("xpra_html5.rs");
    std::fs::write(&out, table).unwrap_or_else(|e| panic!("write {}: {e}", out.display()));
}
