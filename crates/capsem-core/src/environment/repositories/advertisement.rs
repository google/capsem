//! Git smart-HTTP v0/v1 ref discovery; no pack download or host Git process.
//! Wire grammar: https://git-scm.com/docs/gitprotocol-http and gitprotocol-common.
use super::RepositoryResolutionError as Error;
use std::collections::BTreeMap;

pub(super) fn valid_selection(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= capsem_proto::ENVIRONMENT_MAX_SPEC_BYTES
        && !value.starts_with('-')
        && valid_name(value)
}

fn valid_name(value: &str) -> bool {
    !value.is_empty()
        && !value.ends_with('.')
        && !value.contains("..")
        && !value.contains("@{")
        && value
            .split('/')
            .all(|part| !part.is_empty() && !part.starts_with('.') && !part.ends_with(".lock"))
        && !value
            .chars()
            .any(|ch| ch.is_control() || ch.is_whitespace() || "~^:?*[\\".contains(ch))
}
fn object_id(value: &str, width: usize) -> bool {
    value.len() == width && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}
fn line(bytes: &[u8]) -> Result<&str, Error> {
    std::str::from_utf8(bytes.strip_suffix(b"\n").unwrap_or(bytes)).map_err(|_| Error::InvalidAdvertisement)
}

struct Packets<'a>(&'a [u8]);
impl<'a> Packets<'a> {
    fn next(&mut self) -> Result<Option<&'a [u8]>, Error> {
        let header = self.0.get(..4).ok_or(Error::InvalidAdvertisement)?;
        if !header
            .iter()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
        {
            return Err(Error::InvalidAdvertisement);
        }
        let length = usize::from_str_radix(
            std::str::from_utf8(header).map_err(|_| Error::InvalidAdvertisement)?,
            16,
        )
        .map_err(|_| Error::InvalidAdvertisement)?;
        if length == 0 {
            self.0 = &self.0[4..];
            return Ok(None);
        }
        // The protocol's packet length includes its four ASCII hex bytes.
        if !(4..=65520).contains(&length) {
            return Err(Error::InvalidAdvertisement);
        }
        let data = self.0.get(4..length).ok_or(Error::InvalidAdvertisement)?;
        self.0 = &self.0[length..];
        Ok(Some(data))
    }
}

fn object_width(capabilities: &str) -> Result<usize, Error> {
    if capabilities.is_empty() || !capabilities.bytes().all(|byte| byte == b' ' || byte.is_ascii_graphic()) {
        return Err(Error::InvalidAdvertisement);
    }
    let mut format = None;
    for capability in capabilities.split(' ') {
        if let Some(value) = capability.strip_prefix("object-format=") {
            if format.replace(value).is_some() {
                return Err(Error::InvalidAdvertisement);
            }
        }
    }
    match format {
        None | Some("sha1") => Ok(40),
        Some("sha256") => Ok(64),
        _ => Err(Error::InvalidAdvertisement),
    }
}

pub(super) fn resolve(bytes: &[u8], selection: &str) -> Result<String, Error> {
    if bytes.len() > capsem_proto::ENVIRONMENT_MAX_REPOSITORY_ADVERTISEMENT_BYTES {
        return Err(Error::TooLarge);
    }
    if !valid_selection(selection) {
        return Err(Error::InvalidIntent);
    }
    let mut packets = Packets(bytes);
    if line(packets.next()?.ok_or(Error::InvalidAdvertisement)?)? != "# service=git-upload-pack"
        || packets.next()?.is_some()
    {
        return Err(Error::InvalidAdvertisement);
    }
    let mut first = packets.next()?.ok_or(Error::InvalidAdvertisement)?;
    if line(first)? == "version 1" {
        first = packets.next()?.ok_or(Error::InvalidAdvertisement)?;
    }
    let mut records = BTreeMap::new();
    let mut pending = Some(first);
    let mut width = 40;
    let mut is_first = true;
    let mut empty = false;
    let mut shallow = false;
    let mut previous = None;
    while let Some(data) = pending {
        let text = line(data)?;
        if text.starts_with("ERR ") {
            return Err(Error::Unavailable);
        }
        if let Some(id) = text.strip_prefix("shallow ") {
            if is_first || empty || !object_id(id, width) || id.bytes().all(|byte| byte == b'0') {
                return Err(Error::InvalidAdvertisement);
            }
            shallow = true;
        } else {
            if shallow || empty {
                return Err(Error::InvalidAdvertisement);
            }
            let (reference, capabilities) = text
                .split_once('\0')
                .map_or((text, None), |(reference, caps)| (reference, Some(caps)));
            if is_first {
                width = object_width(capabilities.ok_or(Error::InvalidAdvertisement)?)?;
            } else if capabilities.is_some() {
                return Err(Error::InvalidAdvertisement);
            }
            let (id, name) = reference.split_once(' ').ok_or(Error::InvalidAdvertisement)?;
            if !object_id(id, width) {
                return Err(Error::InvalidAdvertisement);
            }
            if id.bytes().all(|byte| byte == b'0') {
                if !is_first || name != "capabilities^{}" {
                    return Err(Error::InvalidAdvertisement);
                }
                empty = true;
            } else {
                let base = name.strip_suffix("^{}");
                if let Some(base) = base {
                    if !base.starts_with("refs/tags/") || previous != Some(base) {
                        return Err(Error::InvalidAdvertisement);
                    }
                }
                let checked = base.unwrap_or(name);
                if !(checked == "HEAD" || checked.starts_with("refs/"))
                    || !valid_name(checked)
                    || records.insert(name, id).is_some()
                {
                    return Err(Error::InvalidAdvertisement);
                }
                previous = Some(name);
            }
            is_first = false;
        }
        pending = packets.next()?;
    }
    if !packets.0.is_empty() {
        return Err(Error::InvalidAdvertisement);
    }
    let pin = if object_id(selection, width) {
        records
            .values()
            .copied()
            .find(|id| id.eq_ignore_ascii_case(selection))
            .ok_or(Error::UnknownReference)?
    } else if selection == "HEAD" || selection.starts_with("refs/") {
        selected(&records, selection).ok_or(Error::UnknownReference)?
    } else {
        let branch = selected(&records, &format!("refs/heads/{selection}"));
        let tag = selected(&records, &format!("refs/tags/{selection}"));
        match (branch, tag) {
            (Some(_), Some(_)) => return Err(Error::AmbiguousReference),
            (Some(id), None) | (None, Some(id)) => id,
            (None, None) => return Err(Error::UnknownReference),
        }
    };
    Ok(pin.to_ascii_lowercase())
}
fn selected<'a>(records: &BTreeMap<&str, &'a str>, name: &str) -> Option<&'a str> {
    if name.starts_with("refs/tags/") {
        if let Some(id) = records.get(format!("{name}^{{}}").as_str()) {
            return Some(*id);
        }
    }
    records.get(name).copied()
}

#[cfg(test)]
pub(in crate::environment::repositories) mod tests;
