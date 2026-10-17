use anyhow::{bail, Result};

use super::{
    get_i64, get_u16, get_u32, put_i64, put_u16, put_u32, MAX_GUEST_SHARE_PATH_BYTES, POLICY_DIGEST_RANGE,
    UPSTREAM_GRANT_FRAME_SIZE,
};

const PATH_LENGTH_RANGE: std::ops::Range<usize> = POLICY_DIGEST_RANGE.end..POLICY_DIGEST_RANGE.end + 2;
pub(super) const PATH_RANGE: std::ops::Range<usize> =
    PATH_LENGTH_RANGE.end..PATH_LENGTH_RANGE.end + MAX_GUEST_SHARE_PATH_BYTES;
pub(super) const TIME_FLAGS_OFFSET: usize = PATH_RANGE.end;
pub(super) const ATIME_SECONDS_RANGE: std::ops::Range<usize> = TIME_FLAGS_OFFSET + 1..TIME_FLAGS_OFFSET + 9;
const ATIME_NANOS_RANGE: std::ops::Range<usize> = ATIME_SECONDS_RANGE.end..ATIME_SECONDS_RANGE.end + 4;
const MTIME_SECONDS_RANGE: std::ops::Range<usize> = ATIME_NANOS_RANGE.end..ATIME_NANOS_RANGE.end + 8;
const MTIME_NANOS_RANGE: std::ops::Range<usize> = MTIME_SECONDS_RANGE.end..MTIME_SECONDS_RANGE.end + 4;
const TIME_FIELDS_RANGE: std::ops::Range<usize> = TIME_FLAGS_OFFSET..MTIME_NANOS_RANGE.end;
pub(super) const RESERVED_RANGE: std::ops::Range<usize> = TIME_FIELDS_RANGE.end..UPSTREAM_GRANT_FRAME_SIZE;

/// One guest-requested file timestamp. Exact values preserve archive metadata;
/// `Now` and `Omit` retain the corresponding `utimensat(2)` semantics.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GuestFileTime {
    Omit,
    Now,
    Exact { seconds: i64, nanoseconds: u32 },
}

pub(super) fn validate_guest_share_path(path: &[u8]) -> Result<()> {
    if path.len() > MAX_GUEST_SHARE_PATH_BYTES || path.contains(&0) || path.first() == Some(&b'/') {
        bail!("invalid guest-share path");
    }
    if !path.is_empty()
        && path
            .split(|byte| *byte == b'/')
            .any(|component| component.is_empty() || component == b"." || component == b"..")
    {
        bail!("guest-share path is not normalized");
    }
    Ok(())
}

pub(super) fn put_path(frame: &mut [u8; UPSTREAM_GRANT_FRAME_SIZE], path: &[u8]) -> Result<()> {
    validate_guest_share_path(path)?;
    let length = u16::try_from(path.len()).map_err(|_| anyhow::anyhow!("guest-share path is too long"))?;
    put_u16(frame, PATH_LENGTH_RANGE, length);
    frame[PATH_RANGE.start..PATH_RANGE.start + path.len()].copy_from_slice(path);
    Ok(())
}

pub(super) fn get_path(frame: &[u8; UPSTREAM_GRANT_FRAME_SIZE]) -> Result<Vec<u8>> {
    let length = usize::from(get_u16(frame, PATH_LENGTH_RANGE));
    if length > PATH_RANGE.len() {
        bail!("guest-share path length exceeds its field");
    }
    let (used, padding) = frame[PATH_RANGE].split_at(length);
    if padding.iter().any(|byte| *byte != 0) {
        bail!("guest-share path padding is nonzero");
    }
    Ok(used.to_vec())
}

pub(super) fn require_empty_path(path: &[u8]) -> Result<()> {
    if !path.is_empty() {
        bail!("record carries an unrelated guest-share path");
    }
    Ok(())
}

pub(super) fn put_guest_times(
    frame: &mut [u8; UPSTREAM_GRANT_FRAME_SIZE],
    atime: GuestFileTime,
    mtime: GuestFileTime,
) -> Result<()> {
    let (atime_code, atime_seconds, atime_nanos) = encode_guest_time(atime)?;
    let (mtime_code, mtime_seconds, mtime_nanos) = encode_guest_time(mtime)?;
    frame[TIME_FLAGS_OFFSET] = atime_code | (mtime_code << 2);
    put_i64(frame, ATIME_SECONDS_RANGE, atime_seconds);
    put_u32(frame, ATIME_NANOS_RANGE, atime_nanos);
    put_i64(frame, MTIME_SECONDS_RANGE, mtime_seconds);
    put_u32(frame, MTIME_NANOS_RANGE, mtime_nanos);
    Ok(())
}

fn encode_guest_time(value: GuestFileTime) -> Result<(u8, i64, u32)> {
    match value {
        GuestFileTime::Omit => Ok((0, 0, 0)),
        GuestFileTime::Now => Ok((1, 0, 0)),
        GuestFileTime::Exact { seconds, nanoseconds } if nanoseconds < 1_000_000_000 => Ok((2, seconds, nanoseconds)),
        GuestFileTime::Exact { .. } => bail!("guest timestamp nanoseconds must be below one second"),
    }
}

pub(super) fn get_guest_times(frame: &[u8; UPSTREAM_GRANT_FRAME_SIZE]) -> Result<(GuestFileTime, GuestFileTime)> {
    let flags = frame[TIME_FLAGS_OFFSET];
    if flags & 0xf0 != 0 {
        bail!("guest timestamp flags contain reserved bits");
    }
    Ok((
        decode_guest_time(
            flags & 0x03,
            get_i64(frame, ATIME_SECONDS_RANGE),
            get_u32(frame, ATIME_NANOS_RANGE),
        )?,
        decode_guest_time(
            (flags >> 2) & 0x03,
            get_i64(frame, MTIME_SECONDS_RANGE),
            get_u32(frame, MTIME_NANOS_RANGE),
        )?,
    ))
}

fn decode_guest_time(code: u8, seconds: i64, nanoseconds: u32) -> Result<GuestFileTime> {
    match (code, seconds, nanoseconds) {
        (0, 0, 0) => Ok(GuestFileTime::Omit),
        (1, 0, 0) => Ok(GuestFileTime::Now),
        (2, seconds, nanoseconds) if nanoseconds < 1_000_000_000 => Ok(GuestFileTime::Exact { seconds, nanoseconds }),
        (0 | 1, _, _) => bail!("non-exact guest timestamp carries a value"),
        (2, _, _) => bail!("guest timestamp nanoseconds must be below one second"),
        _ => bail!("invalid guest timestamp kind {code}"),
    }
}

pub(super) fn require_empty_time_fields(frame: &[u8; UPSTREAM_GRANT_FRAME_SIZE]) -> Result<()> {
    if frame[TIME_FIELDS_RANGE].iter().any(|byte| *byte != 0) {
        bail!("record carries unrelated guest timestamps");
    }
    Ok(())
}
