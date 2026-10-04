use super::*;
use std::fs;

fn source_commit() -> SourceCommit {
    "0123456789abcdef0123456789abcdef01234567"
        .parse()
        .expect("source commit")
}

fn file_url(path: &Path) -> String {
    let path = path.canonicalize().expect("canonical test path");
    format!("file://{}", path.display())
}

fn minimal_manifest_json(hash: Option<&str>, include_refresh_policy: bool) -> String {
    let hash = hash.unwrap_or("1111111111111111111111111111111111111111111111111111111111111111");
    format!(
        r#"{{
  "format": 2,
  {refresh}
  "assets": {{
    "current": "2026.0607.1",
    "releases": {{
      "2026.0607.1": {{
        "arches": {{
          "arm64": {{
            "rootfs.erofs": {{
              "hash": "{hash}",
              "size": 17
            }}
          }}
        }}
      }}
    }}
  }},
  "binaries": {{
    "current": "1.0.0",
    "releases": {{
      "1.0.0": {{
        "min_assets": "2026.0607.1"
      }}
    }}
  }}
}}"#,
        refresh = if include_refresh_policy {
            r#""refresh_policy": "24h","#
        } else {
            ""
        },
        hash = hash,
    )
}

fn write_test_assets_manifest(root: &Path, arch: &str) -> PathBuf {
    let assets_dir = root.join("assets").join(arch);
    fs::create_dir_all(&assets_dir).expect("assets dir");
    let kernel = format!("kernel-{arch}");
    let initrd = format!("initrd-{arch}");
    let rootfs = format!("rootfs-{arch}");
    let obom = test_obom_json();
    let software_inventory = test_software_inventory_json(arch);
    let pkg_sha256 = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
    let sbom_sha256 = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789";
    let pkg_blake3 = "1111111111111111111111111111111111111111111111111111111111111111";
    let sbom_blake3 = "2222222222222222222222222222222222222222222222222222222222222222";
    fs::write(assets_dir.join("vmlinuz"), kernel.as_bytes()).expect("kernel");
    fs::write(assets_dir.join("initrd.img"), initrd.as_bytes()).expect("initrd");
    fs::write(assets_dir.join("rootfs.erofs"), rootfs.as_bytes()).expect("rootfs");
    fs::write(assets_dir.join("obom.cdx.json"), obom.as_bytes()).expect("obom");
    fs::write(
        assets_dir.join("software-inventory.json"),
        software_inventory.as_bytes(),
    )
    .expect("software inventory");
    let manifest_path = root.join("assets/manifest.json");
    fs::write(
        &manifest_path,
        format!(
            r#"{{
  "format": 2,
  "refresh_policy": "24h",
  "assets": {{
    "current": "2030.0101.1",
    "releases": {{
      "2030.0101.1": {{
        "date": "2030-01-01",
        "deprecated": false,
        "min_binary": "1.0.0",
        "arches": {{
          "{arch}": {{
            "vmlinuz": {{"hash": "{kernel_hash}", "size": {kernel_size}}},
            "initrd.img": {{"hash": "{initrd_hash}", "size": {initrd_size}}},
            "rootfs.erofs": {{"hash": "{rootfs_hash}", "size": {rootfs_size}}},
            "obom.cdx.json": {{"hash": "{obom_hash}", "size": {obom_size}}},
            "software-inventory.json": {{"hash": "{software_inventory_hash}", "size": {software_inventory_size}}}
          }}
        }}
      }}
    }}
  }},
  "binaries": {{
    "current": "1.0.0",
    "releases": {{
      "1.0.0": {{
        "date": "2030-01-01",
        "deprecated": false,
        "min_assets": "2030.0101.1",
        "files": [
          {{"name": "capsem-1.0.0.pkg", "size": 123, "sha256": "{pkg_sha256}", "blake3": "{pkg_blake3}", "binaries": [
            {{
              "name": "capsem-app",
              "installed_path": "/Applications/Capsem.app/Contents/MacOS/capsem-app",
              "size": 17,
              "sha256": "{binary_sha256}",
              "blake3": "{binary_blake3}",
              "sbom_component_ref": "SPDXRef-File-capsem-app"
            }}
          ]}},
          {{"name": "capsem-sbom.spdx.json", "size": 456, "sha256": "{sbom_sha256}", "blake3": "{sbom_blake3}"}}
        ]
      }}
    }}
  }}
}}"#,
            arch = arch,
            kernel_hash = blake3::hash(kernel.as_bytes()).to_hex(),
            kernel_size = kernel.len(),
            initrd_hash = blake3::hash(initrd.as_bytes()).to_hex(),
            initrd_size = initrd.len(),
            rootfs_hash = blake3::hash(rootfs.as_bytes()).to_hex(),
            rootfs_size = rootfs.len(),
            obom_hash = blake3::hash(obom.as_bytes()).to_hex(),
            obom_size = obom.len(),
            software_inventory_hash = blake3::hash(software_inventory.as_bytes()).to_hex(),
            software_inventory_size = software_inventory.len(),
            pkg_sha256 = pkg_sha256,
            sbom_sha256 = sbom_sha256,
            pkg_blake3 = pkg_blake3,
            sbom_blake3 = sbom_blake3,
            binary_sha256 = "3333333333333333333333333333333333333333333333333333333333333333",
            binary_blake3 = "4444444444444444444444444444444444444444444444444444444444444444",
        ),
    )
    .expect("manifest");
    manifest_path
}

/// A complete runtime graph for `channel`: one package cohort at 1.0.0 and a
/// runtime at `revision` whose arm64 image set validates.
fn test_runtime_graph(channel: &str, revision: &str) -> serde_json::Value {
    let digest = |sha256: char, blake3: char| {
        serde_json::json!({
            "sha256": sha256.to_string().repeat(64),
            "blake3": blake3.to_string().repeat(64),
        })
    };
    let url = |file: &str| format!("/runtime/releases/{channel}/{revision}/arm64/{file}");
    let image = |kind: &str, name: &str, bytes: u64| {
        serde_json::json!({
            "kind": kind,
            "name": name,
            "url": url(name),
            "bytes": bytes,
            "digest": digest('6', '7'),
            "status": "current",
        })
    };
    let evidence = |kind: &str, file: &str, sha256: char| {
        serde_json::json!({
            "kind": kind,
            "url": url(file),
            "bytes": 64,
            "digest": digest(sha256, '9'),
            "status": "current",
        })
    };
    serde_json::json!({
        "version": "1.0.2",
        "channel": channel,
        "status": "current",
        "packages": [
            {
                "id": "old-capsem-pkg",
                "kind": "macos_pkg",
                "name": "Capsem-1.0.0.pkg",
                "version": "1.0.0",
                "platform": "macos",
                "architecture": "arm64",
                "url": "https://github.com/google/capsem/releases/download/v1.0.0/Capsem-1.0.0.pkg",
                "bytes": 123,
                "digest": digest('0', '1'),
                "binaries": [
                    {
                        "name": "capsem-app",
                        "description": "",
                        "version": "1.0.0",
                        "installed_path": "/Applications/Capsem.app/Contents/MacOS/capsem-app",
                        "platform": "macos",
                        "architecture": "arm64",
                        "bytes": 17,
                        "digest": digest('2', '3'),
                        "status": "current",
                        "sbom_component_ref": "SPDXRef-File-capsem-app",
                    }
                ],
                "evidence": [],
                "status": "current",
            }
        ],
        "runtime": {
            "revision": revision,
            "status": "current",
            "min_capsem_version": "1.0.0",
            "architectures": [
                {
                    "architecture": "arm64",
                    "package_inventory_revision": revision,
                    "image_revision": revision,
                    "software": [
                        {
                            "name": "python3",
                            "version": "3.12.1-1",
                            "source": "apt",
                            "architecture": "arm64",
                            "evidence": url("software-inventory.json"),
                            "digest": digest('4', '5'),
                        }
                    ],
                    "images": [
                        image("kernel", "vmlinuz", 777),
                        image("initrd", "initrd.img", 778),
                        image("rootfs", "rootfs.erofs", 779),
                    ],
                    "evidence": [
                        evidence("obom", "obom.cdx.json", '8'),
                        evidence("software_inventory", "software-inventory.json", 'a'),
                    ],
                }
            ],
        },
    })
}

fn write_test_release_graph_manifest(root: &Path) -> PathBuf {
    let manifest_path = root.join("graph-manifest.json");
    fs::write(
        &manifest_path,
        format!(
            "{}\n",
            serde_json::to_string_pretty(&test_runtime_graph("stable", "2030.0101.1")).expect("graph manifest")
        ),
    )
    .expect("manifest");
    manifest_path
}

fn test_software_inventory_json(arch: &str) -> String {
    format!(
        "{}\n",
        serde_json::json!({
            "schema": "capsem.runtime_software_inventory.v1",
            "architecture": arch,
            "packages": [
                {
                    "name": "python3",
                    "version": "3.12.1-1",
                    "source": "apt",
                    "architecture": arch
                },
                {
                    "name": "@openai/codex",
                    "version": "1.2.3",
                    "source": "npm",
                    "architecture": "all"
                }
            ]
        })
    )
}

fn test_obom_json() -> String {
    serde_json::json!({
        "bomFormat": "CycloneDX",
        "specVersion": "1.6",
        "metadata": {
            "tools": {
                "components": [
                    {"name": "cdxgen", "version": "11.0.0", "type": "application"}
                ]
            },
            "component": {
                "name": "capsem-code-rootfs",
                "type": "operating-system"
            }
        },
        "components": [
            {"name": "bash", "version": "5.2", "type": "library"}
        ]
    })
    .to_string()
}

#[path = "tests/channel_build.rs"]
mod channel_build;
#[path = "tests/channel_build_bounds.rs"]
mod channel_build_bounds;
#[path = "tests/channel_validation.rs"]
mod channel_validation;
#[path = "tests/config_validation.rs"]
mod config_validation;
#[path = "tests/image_build.rs"]
mod image_build;
#[path = "tests/release_commands.rs"]
mod release_commands;

fn write_minimal_pkg_with_file(path: &Path, file_path: &str, contents: &[u8]) {
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::fs::PermissionsExt;

        let root = tempfile::tempdir().expect("pkg root");
        let payload_path = root.path().join(file_path);
        fs::create_dir_all(payload_path.parent().expect("payload parent")).expect("payload parent dir");
        fs::write(&payload_path, contents).expect("payload file");
        fs::set_permissions(&payload_path, fs::Permissions::from_mode(0o755)).expect("payload executable");

        let output = Command::new("pkgbuild")
            .arg("--root")
            .arg(root.path())
            .arg("--identifier")
            .arg("org.capsem.test")
            .arg("--version")
            .arg("1.4.1234567890")
            .arg(path)
            .output()
            .expect("run pkgbuild");
        assert!(
            output.status.success(),
            "pkgbuild failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    #[cfg(not(target_os = "macos"))]
    {
        use flate2::{write::GzEncoder, Compression};
        use tar::{Builder, Header};

        let mut pkg = Vec::new();
        {
            let encoder = GzEncoder::new(&mut pkg, Compression::default());
            let mut builder = Builder::new(encoder);
            let mut header = Header::new_gnu();
            header.set_size(contents.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            builder
                .append_data(&mut header, format!("capsem.pkg/Payload/{file_path}"), contents)
                .expect("append pkg executable");
            let encoder = builder.into_inner().expect("finish tar");
            encoder.finish().expect("finish gzip");
        }
        fs::write(path, pkg).expect("write synthetic pkg");
    }
}

pub(super) fn write_minimal_deb_with_file(
    path: &Path,
    file_path: &str,
    contents: &[u8],
    architecture: release_graph::PackageArchitecture,
) {
    let control = format!(
        "Package: capsem\nVersion: 1.0.0\nArchitecture: {}\n",
        architecture.as_str()
    );
    write_minimal_deb_with_control(path, file_path, contents, control.as_bytes());
}

pub(super) fn write_minimal_deb_with_control(path: &Path, file_path: &str, contents: &[u8], control: &[u8]) {
    use flate2::{write::GzEncoder, Compression};
    use tar::{Builder, Header};

    let mut control_tar_gz = Vec::new();
    {
        let encoder = GzEncoder::new(&mut control_tar_gz, Compression::default());
        let mut builder = Builder::new(encoder);
        let mut header = Header::new_gnu();
        header.set_size(control.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder
            .append_data(&mut header, "control", control)
            .expect("append Debian control file");
        let encoder = builder.into_inner().expect("finish control tar");
        encoder.finish().expect("finish control gzip");
    }

    let mut data_tar_gz = Vec::new();
    {
        let encoder = GzEncoder::new(&mut data_tar_gz, Compression::default());
        let mut builder = Builder::new(encoder);
        let mut header = Header::new_gnu();
        header.set_size(contents.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        builder
            .append_data(&mut header, file_path, contents)
            .expect("append executable");
        let encoder = builder.into_inner().expect("finish tar");
        encoder.finish().expect("finish gzip");
    }

    let mut deb = Vec::new();
    deb.extend_from_slice(b"!<arch>\n");
    append_ar_member(&mut deb, "debian-binary", b"2.0\n");
    append_ar_member(&mut deb, "control.tar.gz", &control_tar_gz);
    append_ar_member(&mut deb, "data.tar.gz", &data_tar_gz);
    fs::write(path, deb).expect("write deb");
}

fn append_ar_member(out: &mut Vec<u8>, name: &str, contents: &[u8]) {
    use std::io::Write;

    let header = format!(
        "{:<16}{:<12}{:<6}{:<6}{:<8}{:<10}`\n",
        format!("{name}/"),
        0,
        0,
        0,
        0o100644,
        contents.len()
    );
    assert_eq!(header.len(), 60);
    out.write_all(header.as_bytes()).expect("ar header");
    out.write_all(contents).expect("ar contents");
    if !contents.len().is_multiple_of(2) {
        out.write_all(b"\n").expect("ar padding");
    }
}
