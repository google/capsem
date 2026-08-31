use std::{
    collections::BTreeSet,
    fs,
    io::{ErrorKind, Read},
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{anyhow, Context, Result};
use capsem_assets::asset_manager::{BinaryExecutable, BinaryFile};
use sha2::{Digest, Sha256};

use super::{
    binary_description_for_name, is_host_sbom_file, is_package_sbom_file, release_graph, validate_host_spdx_sbom_bytes,
};

#[cfg(test)]
mod tests;

pub(super) fn binary_files_from_artifacts(artifacts: &[PathBuf]) -> Result<Vec<BinaryFile>> {
    let mut files = Vec::new();
    let mut names = BTreeSet::new();
    for path in artifacts {
        let metadata =
            fs::metadata(path).with_context(|| format!("stat binary release artifact {}", path.display()))?;
        if !metadata.is_file() {
            return Err(anyhow!("binary release artifact is not a file: {}", path.display()));
        }
        if metadata.len() == 0 {
            return Err(anyhow!("binary release artifact is empty: {}", path.display()));
        }
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| anyhow!("artifact path has no UTF-8 file name: {}", path.display()))?
            .to_string();
        if !names.insert(name.clone()) {
            return Err(anyhow!("duplicate binary release artifact name: {name}"));
        }
        let bytes = fs::read(path).with_context(|| format!("read binary release artifact {}", path.display()))?;
        if name.ends_with(".deb") {
            let filename_architecture = release_graph::PackageArchitecture::from_package_name(&name)?;
            let control_architecture = deb_control_architecture(&bytes)
                .with_context(|| format!("read Debian control metadata from {}", path.display()))?;
            if filename_architecture != control_architecture {
                return Err(anyhow!(
                    "Debian package filename architecture {} does not match control Architecture {}: {}",
                    filename_architecture.as_str(),
                    control_architecture.as_str(),
                    name
                ));
            }
        } else if name.ends_with(".pkg") {
            release_graph::PackageArchitecture::from_package_name(&name)?;
        }
        if is_host_sbom_file(&name) || is_package_sbom_file(&name) {
            validate_host_spdx_sbom_bytes(&bytes, path)
                .with_context(|| format!("validate host SBOM artifact {}", path.display()))?;
        }
        let sha256 = format!("{:x}", Sha256::digest(&bytes));
        let blake3 = blake3::hash(&bytes).to_hex().to_string();
        files.push(BinaryFile {
            name,
            size: bytes.len() as u64,
            sha256,
            blake3,
            binaries: packaged_executable_inventory(path, &bytes)?,
        });
    }
    files.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(files)
}

fn packaged_executable_inventory(path: &Path, bytes: &[u8]) -> Result<Vec<BinaryExecutable>> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow!("artifact path has no UTF-8 file name: {}", path.display()))?;
    if name.ends_with(".deb") {
        return deb_executable_inventory(bytes)
            .with_context(|| format!("extract executable inventory from {}", path.display()));
    }
    if name.ends_with(".pkg") {
        return pkg_executable_inventory(path, bytes)
            .with_context(|| format!("extract executable inventory from {}", path.display()));
    }
    Ok(Vec::new())
}

fn pkg_executable_inventory(path: &Path, bytes: &[u8]) -> Result<Vec<BinaryExecutable>> {
    if bytes.starts_with(&[0x1f, 0x8b]) {
        return pkg_payload_tar_executable_inventory(path);
    }
    let temp = std::env::temp_dir().join(format!(
        "capsem-admin-pkg-expand-{}-{}",
        std::process::id(),
        blake3::hash(path.to_string_lossy().as_bytes()).to_hex()
    ));
    if temp.exists() {
        fs::remove_dir_all(&temp).with_context(|| format!("remove {}", temp.display()))?;
    }
    let output = match Command::new("pkgutil")
        .arg("--expand-full")
        .arg(path)
        .arg(&temp)
        .output()
    {
        Ok(output) => output,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return pkg_xar_payload_executable_inventory(path).or_else(|_| pkg_payload_tar_executable_inventory(path))
        }
        Err(error) => return Err(error).context("run pkgutil --expand-full"),
    };
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let _ = fs::remove_dir_all(&temp);
        return Err(anyhow!("pkgutil --expand-full failed: {stderr}"));
    }
    let result = collect_pkg_payload_executables(&temp);
    let _ = fs::remove_dir_all(&temp);
    result
}

fn pkg_xar_payload_executable_inventory(path: &Path) -> Result<Vec<BinaryExecutable>> {
    let bytes = fs::read(path).with_context(|| format!("read {}", path.display()))?;
    if bytes.len() < 28 || &bytes[..4] != b"xar!" {
        return Err(anyhow!("{} is not a xar pkg archive", path.display()));
    }
    let header_size = u16::from_be_bytes([bytes[4], bytes[5]]) as usize;
    if header_size < 28 {
        return Err(anyhow!("{} has an invalid xar header", path.display()));
    }
    let compressed_toc_size =
        u64::from_be_bytes(bytes[8..16].try_into().expect("xar compressed TOC size width")) as usize;
    let toc_end = header_size
        .checked_add(compressed_toc_size)
        .ok_or_else(|| anyhow!("{} xar TOC size overflow", path.display()))?;
    if toc_end > bytes.len() {
        return Err(anyhow!("{} xar TOC extends past end of file", path.display()));
    }
    let mut toc_decoder = flate2::read::ZlibDecoder::new(&bytes[header_size..toc_end]);
    let mut toc = String::new();
    toc_decoder
        .read_to_string(&mut toc)
        .with_context(|| format!("decompress xar TOC {}", path.display()))?;
    let mut binaries = Vec::new();
    let mut search_from = 0;
    while let Some(relative_name) = toc[search_from..].find("<name>Payload</name>") {
        let name_index = search_from + relative_name;
        let block_start = toc[..name_index]
            .rfind("<file")
            .ok_or_else(|| anyhow!("{} xar Payload entry missing file start", path.display()))?;
        let block_end = name_index
            + toc[name_index..]
                .find("</file>")
                .ok_or_else(|| anyhow!("{} xar Payload entry missing file end", path.display()))?
            + "</file>".len();
        let block = &toc[block_start..block_end];
        let offset = xml_tag_u64(block, "offset")? as usize;
        let length = xml_tag_u64(block, "length")? as usize;
        let payload_start = toc_end
            .checked_add(offset)
            .ok_or_else(|| anyhow!("{} xar Payload offset overflow", path.display()))?;
        let payload_end = payload_start
            .checked_add(length)
            .ok_or_else(|| anyhow!("{} xar Payload length overflow", path.display()))?;
        if payload_end > bytes.len() {
            return Err(anyhow!("{} xar Payload extends past end of file", path.display()));
        }
        let mut payload = Vec::new();
        if block.contains("application/x-gzip") || bytes[payload_start..payload_end].starts_with(&[0x1f, 0x8b]) {
            let mut decoder = flate2::read::GzDecoder::new(&bytes[payload_start..payload_end]);
            decoder
                .read_to_end(&mut payload)
                .with_context(|| format!("decompress xar Payload {}", path.display()))?;
        } else {
            payload.extend_from_slice(&bytes[payload_start..payload_end]);
        }
        collect_newc_cpio_executables(&payload, &mut binaries)
            .with_context(|| format!("read xar Payload cpio {}", path.display()))?;
        search_from = block_end;
    }
    if binaries.is_empty() {
        return Err(anyhow!(
            "{} xar Payload contained no Capsem executables",
            path.display()
        ));
    }
    binaries.sort_by(|left, right| left.installed_path.cmp(&right.installed_path));
    Ok(binaries)
}

fn xml_tag_u64(block: &str, tag: &str) -> Result<u64> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = block.find(&open).ok_or_else(|| anyhow!("xar XML missing <{tag}>"))? + open.len();
    let end = start
        + block[start..]
            .find(&close)
            .ok_or_else(|| anyhow!("xar XML missing </{tag}>"))?;
    block[start..end]
        .trim()
        .parse::<u64>()
        .with_context(|| format!("parse xar XML <{tag}>"))
}

fn collect_newc_cpio_executables(bytes: &[u8], binaries: &mut Vec<BinaryExecutable>) -> Result<()> {
    if bytes.starts_with(b"070707") {
        return collect_odc_cpio_executables(bytes, binaries);
    }
    let mut offset = 0usize;
    while offset < bytes.len() {
        if offset + 110 > bytes.len() {
            return Err(anyhow!("newc cpio header truncated"));
        }
        let header = &bytes[offset..offset + 110];
        if &header[..6] != b"070701" && &header[..6] != b"070702" {
            return Err(anyhow!("newc cpio header magic mismatch"));
        }
        let mode = cpio_hex_field(header, 14)?;
        let file_size = cpio_hex_field(header, 54)? as usize;
        let name_size = cpio_hex_field(header, 94)? as usize;
        let name_start = offset + 110;
        let name_end = name_start
            .checked_add(name_size)
            .ok_or_else(|| anyhow!("newc cpio name size overflow"))?;
        if name_end > bytes.len() || name_size == 0 {
            return Err(anyhow!("newc cpio name truncated"));
        }
        let name_bytes = &bytes[name_start..name_end - 1];
        let name = std::str::from_utf8(name_bytes).context("newc cpio path is not UTF-8")?;
        let data_start = align4(name_end);
        let data_end = data_start
            .checked_add(file_size)
            .ok_or_else(|| anyhow!("newc cpio file size overflow"))?;
        if data_end > bytes.len() {
            return Err(anyhow!("newc cpio file data truncated"));
        }
        if name == "TRAILER!!!" {
            break;
        }
        let normalized = name.trim_start_matches("./");
        let is_regular = mode & 0o170000 == 0o100000;
        if is_regular && mode & 0o111 != 0 {
            let mut contents = &bytes[data_start..data_end];
            push_pkg_payload_executable(normalized, &mut contents, binaries)?;
        }
        offset = align4(data_end);
    }
    Ok(())
}

fn collect_odc_cpio_executables(bytes: &[u8], binaries: &mut Vec<BinaryExecutable>) -> Result<()> {
    let mut offset = 0usize;
    while offset < bytes.len() {
        if offset + 76 > bytes.len() {
            return Err(anyhow!("odc cpio header truncated"));
        }
        let header = &bytes[offset..offset + 76];
        if &header[..6] != b"070707" {
            return Err(anyhow!("odc cpio header magic mismatch"));
        }
        let mode = cpio_octal_field(header, 18, 6)?;
        let file_size = cpio_octal_field(header, 65, 11)? as usize;
        let name_size = cpio_octal_field(header, 59, 6)? as usize;
        let name_start = offset + 76;
        let name_end = name_start
            .checked_add(name_size)
            .ok_or_else(|| anyhow!("odc cpio name size overflow"))?;
        if name_end > bytes.len() || name_size == 0 {
            return Err(anyhow!("odc cpio name truncated"));
        }
        let name_bytes = &bytes[name_start..name_end - 1];
        let name = std::str::from_utf8(name_bytes).context("odc cpio path is not UTF-8")?;
        let data_start = name_end;
        let data_end = data_start
            .checked_add(file_size)
            .ok_or_else(|| anyhow!("odc cpio file size overflow"))?;
        if data_end > bytes.len() {
            return Err(anyhow!("odc cpio file data truncated"));
        }
        if name == "TRAILER!!!" {
            break;
        }
        let normalized = name.trim_start_matches("./");
        let is_regular = mode & 0o170000 == 0o100000;
        if is_regular && mode & 0o111 != 0 {
            let mut contents = &bytes[data_start..data_end];
            push_pkg_payload_executable(normalized, &mut contents, binaries)?;
        }
        offset = data_end;
    }
    Ok(())
}

fn cpio_hex_field(header: &[u8], start: usize) -> Result<u64> {
    let end = start + 8;
    let value = std::str::from_utf8(&header[start..end]).context("newc cpio hex field UTF-8")?;
    u64::from_str_radix(value, 16).with_context(|| format!("parse newc cpio field {value}"))
}

fn cpio_octal_field(header: &[u8], start: usize, width: usize) -> Result<u64> {
    let end = start + width;
    let value = std::str::from_utf8(&header[start..end]).context("odc cpio octal field UTF-8")?;
    u64::from_str_radix(value, 8).with_context(|| format!("parse odc cpio field {value}"))
}

fn align4(value: usize) -> usize {
    (value + 3) & !3
}

fn pkg_payload_tar_executable_inventory(path: &Path) -> Result<Vec<BinaryExecutable>> {
    let bytes = fs::read(path).with_context(|| format!("read {}", path.display()))?;
    if !bytes.starts_with(&[0x1f, 0x8b]) {
        return Ok(Vec::new());
    }
    let decoder = flate2::read::GzDecoder::new(bytes.as_slice());
    let mut archive = tar::Archive::new(decoder);
    let mut binaries = Vec::new();
    for entry in archive.entries().context("read synthetic pkg payload tar")? {
        let mut entry = entry.context("read synthetic pkg payload entry")?;
        let header = entry.header().clone();
        if !header.entry_type().is_file() || header.mode().unwrap_or(0) & 0o111 == 0 {
            continue;
        }
        let path = entry.path().context("read synthetic pkg payload entry path")?;
        let normalized = path.to_string_lossy().to_string();
        let Some((_, installed_path)) = normalized.split_once("/Payload/") else {
            continue;
        };
        push_pkg_payload_executable(installed_path, &mut entry, &mut binaries)?;
    }
    binaries.sort_by(|left, right| left.installed_path.cmp(&right.installed_path));
    Ok(binaries)
}

fn collect_pkg_payload_executables(root: &Path) -> Result<Vec<BinaryExecutable>> {
    let mut binaries = Vec::new();
    collect_pkg_payload_executables_from(root, &mut binaries)?;
    binaries.sort_by(|left, right| left.installed_path.cmp(&right.installed_path));
    Ok(binaries)
}

fn collect_pkg_payload_executables_from(path: &Path, binaries: &mut Vec<BinaryExecutable>) -> Result<()> {
    for entry in fs::read_dir(path).with_context(|| format!("read {}", path.display()))? {
        let entry = entry.with_context(|| format!("read entry in {}", path.display()))?;
        let path = entry.path();
        let metadata = entry.metadata().with_context(|| format!("stat {}", path.display()))?;
        if metadata.is_dir() {
            collect_pkg_payload_executables_from(&path, binaries)?;
            continue;
        }
        if !metadata.is_file() {
            continue;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if metadata.permissions().mode() & 0o111 == 0 {
                continue;
            }
        }
        let normalized = path.to_string_lossy();
        let Some((_, installed_path)) = normalized.split_once("/Payload/") else {
            continue;
        };
        let mut contents = fs::File::open(&path).with_context(|| format!("open {}", path.display()))?;
        push_pkg_payload_executable(installed_path, &mut contents, binaries)?;
    }
    Ok(())
}

fn push_pkg_payload_executable(
    installed_path: &str,
    reader: &mut dyn Read,
    binaries: &mut Vec<BinaryExecutable>,
) -> Result<()> {
    if !installed_path.starts_with("usr/local/share/capsem/bin/")
        && !installed_path.starts_with("Applications/Capsem.app/Contents/MacOS/")
    {
        return Ok(());
    }
    let name = Path::new(installed_path)
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow!("pkg executable has no file name: {installed_path}"))?
        .to_string();
    let mut contents = Vec::new();
    reader
        .read_to_end(&mut contents)
        .with_context(|| format!("read pkg executable {installed_path}"))?;
    binaries.push(BinaryExecutable {
        sbom_component_ref: format!("SPDXRef-File-{}", spdx_ref_fragment(&name)),
        description: binary_description_for_name(&name).to_string(),
        installed_path: format!("/{installed_path}"),
        name,
        size: contents.len() as u64,
        sha256: format!("{:x}", Sha256::digest(&contents)),
        blake3: blake3::hash(&contents).to_hex().to_string(),
    });
    Ok(())
}

fn deb_executable_inventory(bytes: &[u8]) -> Result<Vec<BinaryExecutable>> {
    let mut reader: Box<dyn Read> = if let Ok(data_tar) = deb_member(bytes, "data.tar.gz") {
        Box::new(flate2::read::GzDecoder::new(data_tar))
    } else {
        let data_tar = deb_member(bytes, "data.tar.zst")?;
        Box::new(zstd::stream::read::Decoder::new(data_tar).context("decode data.tar.zst")?)
    };
    let mut archive = tar::Archive::new(&mut reader);
    let mut binaries = Vec::new();
    for entry in archive.entries().context("read data.tar.gz entries")? {
        let mut entry = entry.context("read data.tar.gz entry")?;
        let header = entry.header().clone();
        if !header.entry_type().is_file() || header.mode().unwrap_or(0) & 0o111 == 0 {
            continue;
        }
        let path = entry.path().context("read data.tar.gz entry path")?;
        let normalized = path.to_string_lossy().trim_start_matches("./").to_string();
        if !normalized.starts_with("usr/bin/") && !normalized.starts_with("usr/local/bin/") {
            continue;
        }
        let name = Path::new(&normalized)
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| anyhow!("deb executable has no file name: {normalized}"))?
            .to_string();
        let mut contents = Vec::new();
        entry
            .read_to_end(&mut contents)
            .with_context(|| format!("read deb executable {normalized}"))?;
        binaries.push(BinaryExecutable {
            sbom_component_ref: format!("SPDXRef-File-{}", spdx_ref_fragment(&name)),
            description: binary_description_for_name(&name).to_string(),
            installed_path: format!("/{normalized}"),
            name,
            size: contents.len() as u64,
            sha256: format!("{:x}", Sha256::digest(&contents)),
            blake3: blake3::hash(&contents).to_hex().to_string(),
        });
    }
    binaries.sort_by(|left, right| left.installed_path.cmp(&right.installed_path));
    Ok(binaries)
}

fn deb_control_architecture(bytes: &[u8]) -> Result<release_graph::PackageArchitecture> {
    let mut reader: Box<dyn Read> = if let Ok(control_tar) = deb_member(bytes, "control.tar.gz") {
        Box::new(flate2::read::GzDecoder::new(control_tar))
    } else {
        let control_tar = deb_member(bytes, "control.tar.zst")?;
        Box::new(zstd::stream::read::Decoder::new(control_tar).context("decode control.tar.zst")?)
    };
    let mut archive = tar::Archive::new(&mut reader);
    let mut architecture = None;
    for entry in archive.entries().context("read Debian control archive")? {
        let mut entry = entry.context("read Debian control entry")?;
        let path = entry.path().context("read Debian control entry path")?;
        if path.to_string_lossy().trim_start_matches("./") != "control" {
            continue;
        }
        let mut control = String::new();
        entry.read_to_string(&mut control).context("read Debian control file")?;
        for line in control.lines() {
            let Some(value) = line.strip_prefix("Architecture:") else {
                continue;
            };
            if architecture.is_some() {
                return Err(anyhow!("Debian control file contains duplicate Architecture fields"));
            }
            architecture = Some(match value.trim() {
                "amd64" => release_graph::PackageArchitecture::Amd64,
                "arm64" => release_graph::PackageArchitecture::Arm64,
                value => {
                    return Err(anyhow!("unsupported Debian control Architecture: {value}"));
                }
            });
        }
    }
    architecture.ok_or_else(|| anyhow!("Debian control file is missing Architecture"))
}

fn deb_member<'a>(bytes: &'a [u8], member_name: &str) -> Result<&'a [u8]> {
    if !bytes.starts_with(b"!<arch>\n") {
        return Err(anyhow!("deb archive missing ar global header"));
    }
    let mut offset = 8usize;
    while offset + 60 <= bytes.len() {
        let header = &bytes[offset..offset + 60];
        offset += 60;
        if &header[58..60] != b"`\n" {
            return Err(anyhow!("deb archive member header is malformed"));
        }
        let raw_name = std::str::from_utf8(&header[0..16])
            .context("deb archive member name is not UTF-8")?
            .trim();
        let name = raw_name.trim_end_matches('/');
        let size_text = std::str::from_utf8(&header[48..58])
            .context("deb archive member size is not UTF-8")?
            .trim();
        let size = size_text
            .parse::<usize>()
            .with_context(|| format!("deb archive member {name} has invalid size"))?;
        let end = offset
            .checked_add(size)
            .ok_or_else(|| anyhow!("deb archive member {name} size overflows"))?;
        if end > bytes.len() {
            return Err(anyhow!("deb archive member {name} extends past end of file"));
        }
        if name == member_name {
            return Ok(&bytes[offset..end]);
        }
        offset = end + (size % 2);
    }
    Err(anyhow!("deb archive missing {member_name}"))
}

fn spdx_ref_fragment(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-') {
                ch
            } else {
                '-'
            }
        })
        .collect()
}
